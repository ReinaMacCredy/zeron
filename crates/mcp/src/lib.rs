//! zeron-mcp — the thread tools a chat's agent sees, served over MCP stdio.
//!
//! The engine spawns `zeron mcp --chat <id> --port <ipc>` next to each
//! harness process; the harness lists it as an MCP server, so the agent
//! calls these like any other tool. Newline-delimited JSON-RPC 2.0 on
//! stdin/stdout (the MCP stdio transport). Every tool is a thin call over
//! the engine's own RPC (d28); the caller's identity is the spawn argument,
//! never a tool input. Threads are peers (d25): nothing here interrupts,
//! cancels, or answers for another chat.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use zeron_proto::view::{Indicator, effective_indicator};
use zeron_proto::{Chat, Session};
use zeron_rpc::{RpcClient, methods};

/// The MCP revision this server was written against; a client's own value
/// is echoed back when it names one.
pub const PROTOCOL_VERSION: &str = "2025-06-18";
const DEFAULT_TAIL: usize = 30;
const MAX_TAIL: usize = 200;
const DEFAULT_WAIT_SECS: u64 = 600;
const MAX_WAIT_SECS: u64 = 1800;
/// Outgoing messages (sends and creates) one server process — one turn —
/// may issue. A ping-pong between two agents ends in a tool error instead of
/// a silent token burn.
pub const SEND_BUDGET: usize = 16;
/// A Working row that stops heartbeating only turns stale by the clock, so a
/// wait re-evaluates its snapshot on a timer, not just on pushes.
const STALE_RECHECK: Duration = Duration::from_secs(5);

pub struct ThreadTools {
    client: RpcClient,
    chat_id: String,
    sends: AtomicUsize,
}

/// `ReasoningLevel`'s serde names, spelled out so the schema can list them.
const REASONING_LEVELS: [&str; 9] = [
    "minimal",
    "low",
    "medium",
    "high",
    "xhigh",
    "max",
    "ultra",
    "ultracode",
    "ultrathink",
];

/// The tool catalog as `tools/list` reports it.
pub fn tool_list() -> Value {
    let thread_id = json!({ "type": "string" });
    json!([
        {
            "name": "list_threads",
            "description": "List the chats (threads) in this chat's project with their live status and the role and first 120 prose characters of their newest transcript entry. `lastRole` identifies the author only; neither an assistant entry nor idle status proves the task is complete or safe to archive. Use status for runtime state and read_thread for context. Results are ordered by id, default 50 and max 200 per page; pass nextCursor as cursor for the next page. Archived threads are left out unless `includeArchived` is true. Threads are peers, not workers: every one is a real sidebar chat the human can open, and none can stop another.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "includeArchived": { "type": "boolean" },
                    "status": { "type": "string", "enum": ["idle", "working", "awaiting_input", "errored"] },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 200 },
                    "cursor": { "type": "string", "description": "nextCursor from the previous page with the same filters." }
                },
                "additionalProperties": false
            }
        },
        {
            "name": "read_thread",
            "description": "Read the tail of a thread's transcript. Returns the newest `limit` entries (default 30, max 200) and a `before` cursor for older ones.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "threadId": thread_id,
                    "limit": { "type": "integer", "minimum": 1, "maximum": 200 },
                    "before": { "type": "string", "description": "An entry id from a previous page; returns entries older than it." }
                },
                "required": ["threadId"],
                "additionalProperties": false
            }
        },
        {
            "name": "create_thread",
            "description": "Open a new peer thread in this project and give it `prompt` as its first message. It runs this chat's harness, model and reasoning level unless `harness`, `model` or `reasoning` pick others; picking a different harness without a `model` uses that harness's default model, and an unknown harness or reasoning level is an error. Returns the new thread's row as list_threads reports it. Counts against this turn's send budget.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "prompt": { "type": "string" },
                    "title": { "type": "string" },
                    "harness": { "type": "string", "description": "Harness id such as claude-code or codex; default: this chat's." },
                    "model": { "type": "string", "description": "Model id as the chosen harness names it; default: this chat's." },
                    "reasoning": { "type": "string", "enum": REASONING_LEVELS, "description": "Reasoning level; default: this chat's." }
                },
                "required": ["prompt"],
                "additionalProperties": false
            }
        },
        {
            "name": "send_message_to_thread",
            "description": "Send a message to another thread in this project. An idle thread starts a turn with it; a working thread receives it inside its current turn at the next step (a wait_threads call there returns with it), or at turn end if its agent takes no mid-turn input. Never to this chat itself. Counts against this turn's send budget.",
            "inputSchema": {
                "type": "object",
                "properties": { "threadId": thread_id, "text": { "type": "string" } },
                "required": ["threadId", "text"],
                "additionalProperties": false
            }
        },
        {
            "name": "wait_threads",
            "description": "Wait with mode any (default: any listed thread stops working, or an inbox message arrives), all (all listed threads stop working), or message (an inbox message arrives, even if peers are already idle). Use message after sending when you need a reply; it receives pending inbox messages too and does not correlate replies to a particular send. all requires threadIds; any with no ids waits for a message. Idle, awaiting_input and errored all mean not working, not task success. A message returned here also reaches you as a user message at your next step, so act on it once. Timeout defaults to 600s, max 1800s.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "threadIds": { "type": "array", "items": { "type": "string" } },
                    "timeoutSeconds": { "type": "integer", "minimum": 0, "maximum": 1800 },
                    "mode": { "type": "string", "enum": ["any", "all", "message"] }
                },
                "additionalProperties": false
            }
        },
        {
            "name": "set_thread_title",
            "description": "Rename a thread in this project (this chat included).",
            "inputSchema": {
                "type": "object",
                "properties": { "threadId": thread_id, "title": { "type": "string" } },
                "required": ["threadId", "title"],
                "additionalProperties": false
            }
        },
        {
            "name": "set_thread_archived",
            "description": "Archive or unarchive one thread (`threadId`) or several (`threadIds`) in this project. A working thread (this chat included, while its turn runs) cannot be archived; archiving never cascades to other threads.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "threadId": thread_id,
                    "threadIds": { "type": "array", "items": { "type": "string" } },
                    "archived": { "type": "boolean" }
                },
                "required": ["archived"],
                "additionalProperties": false
            }
        }
    ])
}

impl ThreadTools {
    pub fn new(client: RpcClient, chat_id: impl Into<String>) -> Self {
        Self {
            client,
            chat_id: chat_id.into(),
            sends: AtomicUsize::new(0),
        }
    }

    pub async fn call(&self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "list_threads" => self.list_threads(args).await,
            "read_thread" => self.read_thread(args).await,
            "create_thread" => self.create_thread(args).await,
            "send_message_to_thread" => self.send_message_to_thread(args).await,
            "wait_threads" => self.wait_threads(args).await,
            "set_thread_title" => self.set_thread_title(args).await,
            "set_thread_archived" => self.set_thread_archived(args).await,
            other => Err(format!("unknown tool: {other}")),
        }
    }

    /// The first item of a watch stream: the snapshot every watch opens with.
    async fn snapshot<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
    ) -> Result<T, String> {
        let mut items = self
            .client
            .subscribe(method, params)
            .await
            .map_err(|e| format!("{method}: {e}"))?;
        let item = items
            .recv()
            .await
            .ok_or_else(|| format!("{method} ended without a snapshot"))?;
        serde_json::from_value(item).map_err(|e| format!("{method}: {e}"))
    }

    async fn call_engine(&self, method: &str, params: Value) -> Result<Value, String> {
        self.client
            .call(method, params)
            .await
            .map_err(|e| format!("{method}: {e}"))
    }

    /// The caller's row plus every other chat it may see: same device, same
    /// project (space), not archived.
    async fn peers(&self) -> Result<(Chat, Vec<Chat>), String> {
        let chats: Vec<Chat> = self.snapshot(methods::WATCH_CHATS, json!({})).await?;
        let caller = chats
            .iter()
            .find(|chat| chat.id == self.chat_id)
            .cloned()
            .ok_or_else(|| format!("this chat ({}) is not in the registry", self.chat_id))?;
        let peers = chats
            .into_iter()
            .filter(|chat| chat.device_id == caller.device_id && chat.space_id == caller.space_id)
            .collect();
        Ok((caller, peers))
    }

    /// `thread_id` as one of the caller's peers, or a scope error.
    async fn peer(&self, thread_id: &str) -> Result<Chat, String> {
        let (_, peers) = self.peers().await?;
        peers
            .into_iter()
            .find(|chat| chat.id == thread_id)
            .ok_or_else(|| format!("no thread {thread_id} in this project"))
    }

    fn thread_id<'a>(args: &'a Value) -> Result<&'a str, String> {
        args["threadId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| "threadId is required".to_string())
    }

    fn spend_send(&self) -> Result<(), String> {
        let used = self.sends.fetch_add(1, Ordering::SeqCst);
        if used >= SEND_BUDGET {
            self.sends.fetch_sub(1, Ordering::SeqCst);
            return Err(format!(
                "this turn's outgoing message budget ({SEND_BUDGET}) is spent; finish the turn and continue in the next one"
            ));
        }
        Ok(())
    }

    pub async fn list_threads(&self, args: &Value) -> Result<Value, String> {
        let include_archived = args["includeArchived"].as_bool().unwrap_or(false);
        let filter = match args.get("status") {
            None => None,
            Some(Value::String(status))
                if matches!(
                    status.as_str(),
                    "idle" | "working" | "awaiting_input" | "errored"
                ) =>
            {
                Some(status.as_str())
            }
            _ => return Err("status must be idle, working, awaiting_input or errored".into()),
        };
        let limit = match args.get("limit") {
            None => 50,
            Some(value) => value
                .as_u64()
                .filter(|n| (1..=200).contains(n))
                .ok_or("limit must be between 1 and 200")? as usize,
        };
        let cursor = match args.get("cursor") {
            None => None,
            Some(Value::String(cursor)) => Some(cursor.as_str()),
            _ => return Err("cursor must be a string".into()),
        };
        let (_, mut peers) = self.peers().await?;
        peers.sort_by(|a, b| a.id.cmp(&b.id));
        if let Some(cursor) = cursor
            && !peers.iter().any(|chat| chat.id == cursor)
        {
            return Err("cursor thread is no longer in this project; restart listing".into());
        }
        let sessions: Vec<Session> = self.snapshot(methods::WATCH_SESSIONS, json!({})).await?;
        let now = Utc::now();
        let page: Vec<_> = peers
            .iter()
            .filter(|chat| {
                (include_archived || !chat.archived)
                    && cursor.is_none_or(|cursor| chat.id.as_str() > cursor)
                    && filter.is_none_or(|status| status_of(&sessions, &chat.id, now) == status)
            })
            .take(limit + 1)
            .collect();
        let next_cursor = (page.len() > limit).then(|| page[limit - 1].id.clone());
        let mut threads = Vec::new();
        for chat in page.into_iter().take(limit) {
            threads.push(self.row_of(chat, &sessions, now).await?);
        }
        Ok(json!({ "threads": threads, "nextCursor": next_cursor }))
    }

    /// One thread as list_threads and create_thread report it.
    async fn row_of(
        &self,
        chat: &Chat,
        sessions: &[Session],
        now: DateTime<Utc>,
    ) -> Result<Value, String> {
        let summary = self
            .call_engine(methods::GET_THREAD_SUMMARY, json!({"chatId": chat.id}))
            .await?;
        Ok(json!({
            "id": chat.id,
            "title": chat.title,
            "status": status_of(sessions, &chat.id, now),
            "archived": chat.archived,
            "harness": chat.config.as_ref().map(|c| c.harness),
            "model": chat.config.as_ref().and_then(|c| c.model.as_deref()),
            "reasoning": chat.config.as_ref().and_then(|c| c.reasoning),
            "createdByChatId": chat.created_by_chat_id,
            "updatedAt": stamp(chat.last_message_at.unwrap_or(chat.created_at)),
            "isThisChat": chat.id == self.chat_id,
            "entryCount": summary["entryCount"],
            "lastRole": summary["lastRole"],
            "lastText": summary["lastText"],
        }))
    }

    async fn entries_of(&self, thread_id: &str) -> Result<Vec<Value>, String> {
        let snapshot: Value = self
            .snapshot(methods::WATCH_DOC_MESSAGES, json!({ "chatId": thread_id }))
            .await?;
        Ok(snapshot["reset"].as_array().cloned().unwrap_or_default())
    }

    pub async fn read_thread(&self, args: &Value) -> Result<Value, String> {
        let thread_id = Self::thread_id(args)?;
        self.peer(thread_id).await?;
        let limit = args["limit"]
            .as_u64()
            .map(|n| n as usize)
            .unwrap_or(DEFAULT_TAIL)
            .clamp(1, MAX_TAIL);
        let entries = self.entries_of(thread_id).await?;
        let end = match args["before"].as_str() {
            Some(before) => entries
                .iter()
                .position(|entry| entry["id"] == before)
                .ok_or_else(|| format!("no entry {before} in thread {thread_id}"))?,
            None => entries.len(),
        };
        let start = end.saturating_sub(limit);
        Ok(json!({
            "threadId": thread_id,
            "entries": entries[start..end].iter().map(render_entry).collect::<Vec<_>>(),
            "before": (start > 0).then(|| entries[start]["id"].clone()),
        }))
    }

    pub async fn create_thread(&self, args: &Value) -> Result<Value, String> {
        let prompt = args["prompt"]
            .as_str()
            .filter(|p| !p.trim().is_empty())
            .ok_or("prompt is required")?;
        self.spend_send()?;
        let created = self
            .call_engine(
                methods::CREATE_THREAD,
                json!({
                    "fromChatId": self.chat_id,
                    "prompt": prompt,
                    "title": args["title"].as_str(),
                    "harness": args["harness"].as_str(),
                    "model": args["model"].as_str(),
                    "reasoning": args["reasoning"].as_str(),
                }),
            )
            .await?;
        let thread_id = created["chatId"]
            .as_str()
            .ok_or("engine returned no chatId")?;
        let chat = self.peer(thread_id).await?;
        let sessions: Vec<Session> = self.snapshot(methods::WATCH_SESSIONS, json!({})).await?;
        let mut row = self.row_of(&chat, &sessions, Utc::now()).await?;
        row["threadId"] = json!(thread_id);
        Ok(row)
    }

    pub async fn send_message_to_thread(&self, args: &Value) -> Result<Value, String> {
        let thread_id = Self::thread_id(args)?;
        let text = args["text"]
            .as_str()
            .filter(|t| !t.trim().is_empty())
            .ok_or("text is required")?;
        if thread_id == self.chat_id {
            return Err("a thread cannot message itself".into());
        }
        self.peer(thread_id).await?;
        self.spend_send()?;
        let queued = self
            .call_engine(
                methods::SEND_THREAD_MESSAGE,
                json!({ "chatId": thread_id, "fromChatId": self.chat_id, "text": text }),
            )
            .await?;
        Ok(json!({ "threadId": thread_id, "messageId": queued["id"] }))
    }

    pub async fn set_thread_title(&self, args: &Value) -> Result<Value, String> {
        let thread_id = Self::thread_id(args)?;
        let title = args["title"]
            .as_str()
            .filter(|t| !t.trim().is_empty())
            .ok_or("title is required")?;
        self.peer(thread_id).await?;
        self.call_engine(
            methods::MUTATE,
            json!({ "op": "renameChat", "chatId": thread_id, "title": title }),
        )
        .await?;
        Ok(json!({ "threadId": thread_id, "title": title }))
    }

    pub async fn set_thread_archived(&self, args: &Value) -> Result<Value, String> {
        let archived = args["archived"].as_bool().ok_or("archived is required")?;
        let ids: Vec<&str> = match args["threadIds"].as_array() {
            Some(ids) => ids.iter().filter_map(Value::as_str).collect(),
            None => vec![Self::thread_id(args)?],
        };
        if ids.is_empty() {
            return Err("threadId or threadIds is required".into());
        }
        for id in &ids {
            self.peer(id).await?;
        }
        if archived {
            let sessions: Vec<Session> = self.snapshot(methods::WATCH_SESSIONS, json!({})).await?;
            let now = Utc::now();
            if let Some(id) = ids
                .iter()
                .find(|id| status_of(&sessions, id, now) == "working")
            {
                return Err(format!(
                    "thread {id} is working; archiving would hide a running turn, wait for it to end"
                ));
            }
        }
        for id in &ids {
            self.call_engine(
                methods::MUTATE,
                json!({ "op": "setChatArchived", "chatId": id, "archived": archived }),
            )
            .await?;
        }
        Ok(json!({ "threadIds": ids, "archived": archived }))
    }

    /// The inter-thread rows waiting on this chat, moved into its transcript.
    async fn take_messages(&self) -> Result<Vec<Value>, String> {
        let taken = self
            .call_engine(
                methods::TAKE_THREAD_MESSAGES,
                json!({ "chatId": self.chat_id }),
            )
            .await?;
        Ok(taken["messages"].as_array().cloned().unwrap_or_default())
    }

    pub async fn wait_threads(&self, args: &Value) -> Result<Value, String> {
        let ids: Vec<String> = match args.get("threadIds") {
            None => Vec::new(),
            Some(Value::Array(ids)) => ids
                .iter()
                .map(|id| {
                    id.as_str()
                        .filter(|id| !id.is_empty())
                        .map(str::to_owned)
                        .ok_or_else(|| "threadIds must contain nonempty strings".to_string())
                })
                .collect::<Result<_, _>>()?,
            _ => return Err("threadIds must be an array".into()),
        };
        let mode = match args.get("mode") {
            None => "any",
            Some(Value::String(mode)) if matches!(mode.as_str(), "any" | "all" | "message") => {
                mode.as_str()
            }
            _ => return Err("mode must be any, all or message".into()),
        };
        if mode == "all" && ids.is_empty() {
            return Err("all mode requires threadIds".into());
        }
        let (_, peers) = self.peers().await?;
        for id in &ids {
            if !peers.iter().any(|chat| chat.id == *id) {
                return Err(format!("no thread {id} in this project"));
            }
        }
        let timeout = Duration::from_secs(
            args["timeoutSeconds"]
                .as_u64()
                .unwrap_or(DEFAULT_WAIT_SECS)
                .min(MAX_WAIT_SECS),
        );
        let deadline = tokio::time::Instant::now() + timeout;
        let mut sessions = self
            .client
            .subscribe(methods::WATCH_SESSIONS, json!({}))
            .await
            .map_err(|e| format!("WatchSessions: {e}"))?;
        let mut queue = self
            .client
            .subscribe(methods::WATCH_QUEUE, json!({ "chatId": self.chat_id }))
            .await
            .map_err(|e| format!("WatchQueue: {e}"))?;
        // A message steered into this chat's live turn is in the transcript
        // already, never in the queue; the turn only folds it in once this
        // call returns, so the transcript is what wakes the wait.
        let mut transcript = self
            .client
            .subscribe(
                methods::WATCH_DOC_MESSAGES,
                json!({ "chatId": self.chat_id }),
            )
            .await
            .map_err(|e| format!("WatchDocMessages: {e}"))?;
        let mut seen: Option<std::collections::HashSet<String>> = None;
        let mut transcript_open = true;
        let statuses = |sessions: &[Session]| -> Value {
            let now = Utc::now();
            ids.iter()
                .map(|id| (id.clone(), Value::from(status_of(sessions, id, now))))
                .collect::<serde_json::Map<_, _>>()
                .into()
        };
        let mut latest: Option<Vec<Session>> = None;
        loop {
            if let Some(sessions) = &latest {
                let current = statuses(sessions);
                let stopped = |s: &Value| s.as_str() != Some("working");
                let ready = current.as_object().is_some_and(|map| match mode {
                    "any" => map.values().any(stopped),
                    "all" => map.values().all(stopped),
                    _ => false,
                });
                if ready {
                    return Ok(json!({ "reason": "status", "statuses": current }));
                }
            }
            tokio::select! {
                item = sessions.recv() => match item {
                    Some(item) => {
                        latest = Some(
                            serde_json::from_value(item).map_err(|e| format!("WatchSessions: {e}"))?,
                        );
                    }
                    None => return Err("WatchSessions ended".into()),
                },
                item = queue.recv(), if mode != "all" => match item {
                    Some(item) => {
                        let waiting = item["items"]
                            .as_array()
                            .is_some_and(|rows| rows.iter().any(|row| row["fromChatId"].is_string()));
                        if waiting {
                            let messages = self.take_messages().await?;
                            if !messages.is_empty() {
                                return Ok(json!({
                                    "reason": "message",
                                    "messages": messages,
                                    "statuses": latest.as_deref().map(statuses).unwrap_or(json!({})),
                                }));
                            }
                        }
                    }
                    None => return Err("WatchQueue ended".into()),
                },
                item = transcript.recv(), if transcript_open && mode != "all" => match item {
                    Some(frame) => {
                        let arrived = new_peer_entries(&frame, &mut seen);
                        if !arrived.is_empty() {
                            return Ok(json!({
                                "reason": "message",
                                "messages": arrived,
                                "statuses": latest.as_deref().map(statuses).unwrap_or(json!({})),
                            }));
                        }
                    }
                    None => transcript_open = false,
                },
                _ = tokio::time::sleep_until(deadline) => {
                    return Ok(json!({
                        "reason": "timeout",
                        "statuses": latest.as_deref().map(statuses).unwrap_or(json!({})),
                    }));
                }
                _ = tokio::time::sleep(STALE_RECHECK) => {}
            }
        }
    }
}

/// The peer-tagged user entries a `WatchDocMessages` frame carries that were
/// not in the transcript when the wait began. The opening reset seeds `seen`
/// and reports nothing.
fn new_peer_entries(
    frame: &Value,
    seen: &mut Option<std::collections::HashSet<String>>,
) -> Vec<Value> {
    let entries: Vec<&Value> = match frame.get("reset") {
        Some(reset) => reset.as_array().into_iter().flatten().collect(),
        None => frame["upsert"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|u| &u["entry"])
            .collect(),
    };
    let Some(seen) = seen else {
        *seen = Some(
            entries
                .iter()
                .filter_map(|e| e["id"].as_str().map(str::to_string))
                .collect(),
        );
        return Vec::new();
    };
    entries
        .into_iter()
        .filter(|e| e["role"] == "user" && e["fromChatId"].is_string())
        .filter(|e| {
            e["id"]
                .as_str()
                .is_some_and(|id| seen.insert(id.to_string()))
        })
        .map(|e| {
            let rendered = render_entry(e);
            json!({ "id": rendered["id"], "fromChatId": rendered["fromChatId"], "text": rendered["text"] })
        })
        .collect()
}

fn status_of(sessions: &[Session], chat_id: &str, now: DateTime<Utc>) -> &'static str {
    let session = sessions.iter().find(|session| session.chat_id == chat_id);
    match effective_indicator(session, now) {
        Indicator::None => "idle",
        Indicator::Working => "working",
        Indicator::AwaitingInput => "awaiting_input",
        Indicator::Errored => "errored",
    }
}

fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// One transcript entry as the agent reads it: prose kept, thinking dropped,
/// every other part named. Tool inputs stay render-only in the doc, so they
/// never reach a peer through here either.
fn render_entry(entry: &Value) -> Value {
    let mut lines: Vec<(String, usize)> = Vec::new();
    for part in entry["parts"].as_array().into_iter().flatten() {
        let line = match part["kind"].as_str().unwrap_or("") {
            "text" => match part["text"].as_str() {
                Some(text) => text.to_string(),
                None => continue,
            },
            "reasoning" => continue,
            "image" => format!("[image {}]", part["name"].as_str().unwrap_or("attachment")),
            "tool" => format!("[tool {}]", tool_label(part)),
            kind => match part["name"].as_str() {
                Some(name) => format!("[{kind} {name}]"),
                None => format!("[{kind}]"),
            },
        };
        match lines.last_mut() {
            Some((last, count)) if line.starts_with('[') && *last == line => *count += 1,
            _ => lines.push((line, 1)),
        }
    }
    let text = lines
        .into_iter()
        .map(|(line, count)| match count {
            1 => line,
            n => format!("{} ×{n}]", &line[..line.len() - 1]),
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut out = json!({
        "id": entry["id"],
        "role": entry["role"],
        "createdAt": entry["createdAt"]
            .as_i64()
            .and_then(DateTime::from_timestamp_millis)
            .map(stamp),
        "text": text,
    });
    if entry["fromChatId"].is_string() {
        out["fromChatId"] = entry["fromChatId"].clone();
    }
    if entry["status"].is_string() && entry["status"] != "complete" {
        out["status"] = entry["status"].clone();
    }
    out
}

#[cfg(test)]
fn preview(text: &str) -> String {
    const PREVIEW_CHARS: usize = 120;
    let mut out: String = text.chars().take(PREVIEW_CHARS).collect();
    if text.chars().nth(PREVIEW_CHARS).is_some() {
        out.push('…');
    }
    out
}

/// A tool part's one-word identity for the transcript tail: the decoded
/// call's kind plus its headline field (`exec ls -la`, `mcp zeron/read_thread`,
/// `readFile src/x.rs`), or the raw name older rows carry.
fn tool_label(part: &Value) -> String {
    let call = &part["call"];
    let kind = call["kind"].as_str();
    if kind == Some("unknown")
        && let Some(name) = call["name"].as_str()
    {
        return name.to_string();
    }
    let detail = match kind {
        Some("mcp") => match (call["server"].as_str(), call["tool"].as_str()) {
            (Some(server), Some(tool)) => Some(format!("{server}/{tool}")),
            (_, tool) => tool.map(str::to_string),
        },
        _ => ["command", "path", "pattern", "url", "name"]
            .iter()
            .find_map(|key| call[key].as_str().map(str::to_string)),
    };
    match (kind, detail, part["name"].as_str()) {
        (Some(kind), Some(detail), _) => format!("{kind} {}", detail.lines().next().unwrap_or("")),
        (Some(kind), None, _) => kind.to_string(),
        (None, _, Some(name)) => name.to_string(),
        (None, _, None) => "call".to_string(),
    }
}

/// Serve MCP over one line-delimited JSON-RPC pair until the reader closes.
pub async fn serve<R, W>(reader: R, mut writer: W, tools: &ThreadTools) -> std::io::Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut lines = reader.lines();
    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(line) {
            Ok(request) => match respond(&request, tools).await {
                Some(response) => response,
                None => continue,
            },
            Err(err) => error(Value::Null, -32700, format!("parse error: {err}")),
        };
        writer.write_all(response.to_string().as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
    }
    Ok(())
}

/// `None` for notifications (no id), which get no reply.
async fn respond(request: &Value, tools: &ThreadTools) -> Option<Value> {
    let id = request.get("id").filter(|id| !id.is_null())?.clone();
    let params = &request["params"];
    Some(match request["method"].as_str().unwrap_or("") {
        "initialize" => ok(
            id,
            json!({
                "protocolVersion": params["protocolVersion"].as_str().unwrap_or(PROTOCOL_VERSION),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "zeron", "version": env!("CARGO_PKG_VERSION") },
            }),
        ),
        "ping" => ok(id, json!({})),
        "tools/list" => ok(id, json!({ "tools": tool_list() })),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("");
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let (text, is_error) = match tools.call(name, &args).await {
                Ok(value) => (
                    serde_json::to_string_pretty(&value).unwrap_or_default(),
                    false,
                ),
                Err(message) => (message, true),
            };
            ok(
                id,
                json!({ "content": [{ "type": "text", "text": text }], "isError": is_error }),
            )
        }
        other => error(id, -32601, format!("method not found: {other}")),
    })
}

fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Value, code: i64, message: String) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use futures::StreamExt;
    use zeron_rpc::{RpcError, RpcReply, RpcService, memory_client};

    use super::*;

    fn chat(id: &str, space: Option<&str>, archived: bool, created_by: Option<&str>) -> Value {
        json!({
            "id": id, "deviceId": "dev-a", "spaceId": space, "archived": archived,
            "title": format!("Thread {id}"), "createdAt": "2026-09-18T10:00:00Z",
            "createdByChatId": created_by,
        })
    }

    fn session(chat_id: &str, status: &str, age_secs: i64) -> Value {
        let at = Utc::now() - chrono::Duration::seconds(age_secs);
        json!({ "chatId": chat_id, "deviceId": "dev-a", "status": status, "updatedAt": at })
    }

    fn entry(id: &str, role: &str, parts: Vec<Value>) -> Value {
        json!({ "id": id, "role": role, "parts": parts, "createdAt": 1, "deviceId": "dev-a" })
    }

    /// Canned watches (chats once, sessions and queue as timed snapshot
    /// sequences, doc messages once) plus a log of every unary call.
    struct Fake {
        chats: Vec<Value>,
        session_snapshots: Vec<Vec<Value>>,
        queue_snapshots: Vec<Vec<Value>>,
        entries: Vec<Value>,
        doc_frames: Vec<Value>,
        reads: Mutex<Vec<String>>,
        calls: Mutex<Vec<(String, Value)>>,
    }

    fn timed(snapshots: Vec<Value>) -> RpcReply {
        RpcReply::Stream(
            futures::stream::unfold(snapshots.into_iter(), |mut it| async move {
                let next = it.next()?;
                tokio::time::sleep(Duration::from_millis(20)).await;
                Some((next, it))
            })
            .chain(futures::stream::pending())
            .boxed(),
        )
    }

    #[async_trait]
    impl RpcService for Fake {
        async fn handle(&self, method: &str, params: Value) -> Result<RpcReply, RpcError> {
            match method {
                methods::WATCH_CHATS => Ok(RpcReply::Stream(
                    futures::stream::iter(vec![Value::Array(self.chats.clone())]).boxed(),
                )),
                methods::WATCH_SESSIONS => Ok(timed(
                    self.session_snapshots
                        .iter()
                        .cloned()
                        .map(Value::Array)
                        .collect(),
                )),
                methods::WATCH_QUEUE => Ok(timed(
                    self.queue_snapshots
                        .iter()
                        .cloned()
                        .map(|items| json!({ "items": items }))
                        .collect(),
                )),
                methods::GET_THREAD_SUMMARY => {
                    self.reads
                        .lock()
                        .unwrap()
                        .push(params["chatId"].as_str().unwrap().to_string());
                    let last = self.entries.last();
                    let text = last.map(|entry| {
                        entry["parts"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .filter(|part| part["kind"] == "text")
                            .filter_map(|part| part["text"].as_str())
                            .collect::<Vec<_>>()
                            .join("\n")
                    });
                    Ok(RpcReply::Value(json!({
                        "entryCount": self.entries.len(),
                        "lastRole": last.map(|entry| &entry["role"]),
                        "lastText": text.as_deref().map(preview),
                    })))
                }
                methods::WATCH_DOC_MESSAGES => {
                    self.reads.lock().unwrap().push("FULL_TRANSCRIPT".into());
                    Ok(timed(
                        std::iter::once(json!({ "reset": self.entries }))
                            .chain(self.doc_frames.iter().cloned())
                            .collect(),
                    ))
                }
                methods::CREATE_THREAD
                | methods::SEND_THREAD_MESSAGE
                | methods::TAKE_THREAD_MESSAGES
                | methods::MUTATE => {
                    self.calls
                        .lock()
                        .unwrap()
                        .push((method.to_string(), params.clone()));
                    Ok(RpcReply::Value(match method {
                        methods::CREATE_THREAD => json!({ "chatId": "new-1" }),
                        methods::SEND_THREAD_MESSAGE => json!({ "id": "q-1" }),
                        methods::TAKE_THREAD_MESSAGES => json!({
                            "messages": [{ "id": "q-9", "fromChatId": "b", "text": "done, 4 tests" }]
                        }),
                        _ => json!({}),
                    }))
                }
                other => Err(RpcError::UnknownMethod(other.into())),
            }
        }
    }

    struct Rig {
        tools: ThreadTools,
        calls: Arc<Fake>,
    }

    fn rig(fake: Fake) -> Rig {
        let fake = Arc::new(fake);
        Rig {
            tools: ThreadTools::new(memory_client(fake.clone()), "a"),
            calls: fake,
        }
    }

    fn fixture() -> Fake {
        let mut b = chat("b", Some("s1"), false, Some("a"));
        b["config"] = json!({
            "harness": "codex", "model": "gpt-5", "reasoning": "high", "sandbox": "workspace-write"
        });
        Fake {
            chats: vec![
                chat("a", Some("s1"), false, None),
                b,
                chat("c", Some("s2"), false, None),
                chat("d", Some("s1"), true, None),
                chat("e", Some("s1"), false, None),
                chat("new-1", Some("s1"), false, Some("a")),
            ],
            session_snapshots: vec![vec![
                session("b", "working", 0),
                session("e", "working", 120),
                session("new-1", "working", 0),
            ]],
            queue_snapshots: vec![vec![]],
            entries: (1..=5)
                .map(|n| {
                    entry(
                        &format!("m{n}"),
                        if n % 2 == 1 { "user" } else { "assistant" },
                        vec![
                            json!({ "kind": "text", "id": "t", "text": format!("line {n}") }),
                            json!({ "kind": "reasoning", "id": "r", "text": "hidden" }),
                            json!({ "kind": "tool", "id": "k", "name": "Bash" }),
                            json!({ "kind": "tool", "id": "m", "call": { "kind": "mcp", "server": "zeron", "tool": "read_thread" } }),
                        ],
                    )
                })
                .collect(),
            doc_frames: vec![],
            reads: Mutex::new(Vec::new()),
            calls: Mutex::new(Vec::new()),
        }
    }

    #[test]
    fn tool_list_names_every_thread_tool() {
        let list = tool_list();
        let names: Vec<&str> = list
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "list_threads",
                "read_thread",
                "create_thread",
                "send_message_to_thread",
                "wait_threads",
                "set_thread_title",
                "set_thread_archived",
            ]
        );
        assert!(
            names
                .iter()
                .all(|n| !n.contains("cancel") && !n.contains("respond"))
        );
    }

    #[tokio::test]
    async fn list_filters_and_pages_before_loading_summaries() {
        let rig = rig(fixture());
        let first = rig
            .tools
            .list_threads(&json!({"status": "working", "limit": 1}))
            .await
            .unwrap();
        assert_eq!(first["threads"].as_array().unwrap().len(), 1);
        assert_eq!(first["threads"][0]["id"], "b");
        assert_eq!(first["nextCursor"], "b");
        let second = rig
            .tools
            .list_threads(&json!({"status": "working", "limit": 1, "cursor": first["nextCursor"]}))
            .await
            .unwrap();
        assert_eq!(second["threads"][0]["id"], "new-1");
        assert!(second["nextCursor"].is_null());
        assert_eq!(*rig.calls.reads.lock().unwrap(), vec!["b", "new-1"]);
        assert!(
            rig.tools
                .list_threads(&json!({"status": "done"}))
                .await
                .is_err()
        );
        assert!(
            rig.tools
                .list_threads(&json!({"cursor": "missing"}))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn message_wait_does_not_return_for_an_idle_peer() {
        let mut fake = fixture();
        fake.session_snapshots = vec![vec![session("b", "idle", 0)]];
        fake.queue_snapshots = vec![
            vec![],
            vec![json!({"id": "q-9", "fromChatId": "b", "text": "reply"})],
        ];
        let out = rig(fake)
            .tools
            .wait_threads(&json!({"threadIds": ["b"], "mode": "message", "timeoutSeconds": 1}))
            .await
            .unwrap();
        assert_eq!(out["reason"], "message");
    }

    #[tokio::test]
    async fn wait_rejects_invalid_modes_and_out_of_scope_targets() {
        let rig = rig(fixture());
        for args in [
            json!({"mode": "bad"}),
            json!({"mode": "all"}),
            json!({"threadIds": ["c"]}),
            json!({"threadIds": [1]}),
            json!({"threadIds": "b"}),
        ] {
            assert!(rig.tools.wait_threads(&args).await.is_err(), "{args}");
        }
    }

    #[tokio::test]
    async fn all_wait_requires_every_peer_to_stop() {
        let mut fake = fixture();
        fake.queue_snapshots = vec![vec![
            json!({"id": "q-9", "fromChatId": "b", "text": "reply"}),
        ]];
        fake.session_snapshots = vec![
            vec![session("b", "idle", 0), session("e", "working", 0)],
            vec![session("b", "idle", 0), session("e", "errored", 0)],
        ];
        let out = rig(fake)
            .tools
            .wait_threads(&json!({"threadIds": ["b", "e"], "mode": "all", "timeoutSeconds": 1}))
            .await
            .unwrap();
        assert_eq!(out["statuses"]["e"], "errored");
    }

    #[tokio::test]
    async fn list_threads_scopes_to_the_callers_space_and_checks_staleness() {
        let out = rig(fixture()).tools.list_threads(&json!({})).await.unwrap();
        let rows: Vec<(String, String, bool)> = out["threads"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                (
                    row["id"].as_str().unwrap().to_string(),
                    row["status"].as_str().unwrap().to_string(),
                    row["isThisChat"].as_bool().unwrap(),
                )
            })
            .collect();
        // c is another space, d is archived; e's Working row is 2 minutes
        // old, so it reads idle rather than eternally working.
        assert_eq!(
            rows,
            [
                ("a".into(), "idle".into(), true),
                ("b".into(), "working".into(), false),
                ("e".into(), "idle".into(), false),
                ("new-1".into(), "working".into(), false),
            ]
        );
        assert_eq!(out["threads"][1]["archived"], false);
        assert_eq!(out["threads"][1]["createdByChatId"], "a");
        assert_eq!(out["threads"][1]["harness"], "codex");
        assert_eq!(out["threads"][1]["model"], "gpt-5");
        assert_eq!(out["threads"][1]["reasoning"], "high");
        assert!(
            out["threads"][0]["harness"].is_null(),
            "no config, no settings"
        );
        // The newest entry's role and a short preview ride along so a
        // overview needs no read_thread per row; role does not prove completion.
        assert_eq!(out["threads"][1]["entryCount"], 5);
        assert_eq!(out["threads"][1]["lastRole"], "user");
        assert_eq!(out["threads"][1]["lastText"], "line 5");
    }

    #[tokio::test]
    async fn list_threads_previews_are_bounded_and_empty_threads_have_none() {
        let mut fake = fixture();
        fake.entries = vec![entry(
            "m1",
            "assistant",
            vec![json!({ "kind": "text", "id": "t", "text": "x".repeat(300) })],
        )];
        let out = rig(fake).tools.list_threads(&json!({})).await.unwrap();
        let text = out["threads"][0]["lastText"].as_str().unwrap();
        assert_eq!(text.chars().count(), 121);
        assert!(text.ends_with('…'));

        let mut fake = fixture();
        fake.entries = vec![];
        let out = rig(fake).tools.list_threads(&json!({})).await.unwrap();
        assert_eq!(out["threads"][0]["entryCount"], 0);
        assert!(out["threads"][0]["lastRole"].is_null());
        assert!(out["threads"][0]["lastText"].is_null());
    }

    #[tokio::test]
    async fn list_threads_can_include_archived_rows_so_they_can_be_unarchived() {
        let out = rig(fixture())
            .tools
            .list_threads(&json!({ "includeArchived": true }))
            .await
            .unwrap();
        let archived: Vec<(&str, bool)> = out["threads"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                (
                    row["id"].as_str().unwrap(),
                    row["archived"].as_bool().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            archived,
            [
                ("a", false),
                ("b", false),
                ("d", true),
                ("e", false),
                ("new-1", false)
            ]
        );
    }

    #[tokio::test]
    async fn read_thread_collapses_repeated_tool_lines_and_names_unknown_calls() {
        let mut fake = fixture();
        let read = json!({ "kind": "tool", "id": "m", "call": { "kind": "mcp", "server": "zeron", "tool": "read_thread" } });
        let mut parts = vec![
            json!({ "kind": "tool", "id": "s", "name": "ToolSearch", "call": { "kind": "unknown", "name": "ToolSearch" } }),
        ];
        parts.extend(std::iter::repeat_n(read, 21));
        parts.push(json!({ "kind": "text", "id": "t", "text": "done" }));
        parts.push(json!({ "kind": "text", "id": "t2", "text": "done" }));
        fake.entries = vec![entry("m1", "assistant", parts)];
        let page = rig(fake)
            .tools
            .read_thread(&json!({ "threadId": "b" }))
            .await
            .unwrap();
        assert_eq!(
            page["entries"][0]["text"],
            "[tool ToolSearch]\n[tool mcp zeron/read_thread ×21]\ndone\ndone"
        );
    }

    #[test]
    fn the_schemas_reasoning_levels_are_real_ones() {
        for level in REASONING_LEVELS {
            serde_json::from_value::<zeron_proto::ReasoningLevel>(json!(level))
                .unwrap_or_else(|e| panic!("{level}: {e}"));
        }
    }

    #[tokio::test]
    async fn read_thread_returns_a_bounded_tail_with_a_cursor_inside_the_project() {
        let rig = rig(fixture());
        let page = rig
            .tools
            .read_thread(&json!({ "threadId": "b", "limit": 2 }))
            .await
            .unwrap();
        let texts: Vec<&str> = page["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["text"].as_str().unwrap())
            .collect();
        assert_eq!(
            texts,
            [
                "line 4\n[tool Bash]\n[tool mcp zeron/read_thread]",
                "line 5\n[tool Bash]\n[tool mcp zeron/read_thread]"
            ]
        );
        assert_eq!(page["before"], "m4");
        // Defaults stay off the wire: no null fromChatId, no "complete"
        // status, and the stamp reads as a date rather than epoch ms.
        assert_eq!(
            page["entries"][0],
            json!({
                "id": "m4", "role": "assistant", "createdAt": "1970-01-01T00:00:00Z",
                "text": "line 4\n[tool Bash]\n[tool mcp zeron/read_thread]"
            })
        );

        let mut fake = fixture();
        fake.entries = vec![{
            let mut e = entry("m1", "user", vec![]);
            e["fromChatId"] = json!("b");
            e["status"] = json!("streaming");
            e
        }];
        let tagged = self::tests::rig(fake)
            .tools
            .read_thread(&json!({ "threadId": "b" }))
            .await
            .unwrap();
        assert_eq!(tagged["entries"][0]["fromChatId"], "b");
        assert_eq!(tagged["entries"][0]["status"], "streaming");

        let older = rig
            .tools
            .read_thread(&json!({ "threadId": "b", "limit": 2, "before": "m4" }))
            .await
            .unwrap();
        let ids: Vec<&str> = older["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["m2", "m3"]);
        assert_eq!(older["before"], "m2");

        assert_eq!(
            rig.tools.read_thread(&json!({})).await.unwrap_err(),
            "threadId is required"
        );
        // Another project's chat is out of scope even though it exists.
        assert_eq!(
            rig.tools
                .read_thread(&json!({ "threadId": "c" }))
                .await
                .unwrap_err(),
            "no thread c in this project"
        );
    }

    #[tokio::test]
    async fn create_and_send_go_through_the_engine_with_the_callers_identity() {
        let rig = rig(fixture());
        let created = rig
            .tools
            .create_thread(&json!({
                "prompt": "write the tests", "harness": "codex", "model": "gpt-5", "reasoning": "xhigh"
            }))
            .await
            .unwrap();
        assert_eq!(created["threadId"], "new-1");
        assert_eq!(created["id"], "new-1");
        assert_eq!(created["title"], "Thread new-1");
        assert_eq!(created["status"], "working");
        assert_eq!(created["createdByChatId"], "a");
        let sent = rig
            .tools
            .send_message_to_thread(&json!({ "threadId": "b", "text": "cover the refresh race" }))
            .await
            .unwrap();
        assert_eq!(sent["messageId"], "q-1");

        let calls = rig.calls.calls.lock().unwrap().clone();
        assert_eq!(calls[0].0, methods::CREATE_THREAD);
        assert_eq!(calls[0].1["fromChatId"], "a");
        assert_eq!(calls[0].1["harness"], "codex");
        assert_eq!(calls[0].1["model"], "gpt-5");
        assert_eq!(calls[0].1["reasoning"], "xhigh");
        assert_eq!(calls[1].0, methods::SEND_THREAD_MESSAGE);
        assert_eq!(
            calls[1].1,
            json!({ "chatId": "b", "fromChatId": "a", "text": "cover the refresh race" })
        );

        assert_eq!(
            rig.tools
                .send_message_to_thread(&json!({ "threadId": "a", "text": "hi" }))
                .await
                .unwrap_err(),
            "a thread cannot message itself"
        );
        assert_eq!(
            rig.tools
                .send_message_to_thread(&json!({ "threadId": "c", "text": "hi" }))
                .await
                .unwrap_err(),
            "no thread c in this project"
        );
    }

    #[tokio::test]
    async fn the_send_budget_ends_a_ping_pong() {
        let rig = rig(fixture());
        for _ in 0..SEND_BUDGET - 1 {
            rig.tools
                .send_message_to_thread(&json!({ "threadId": "b", "text": "again" }))
                .await
                .unwrap();
        }
        rig.tools
            .create_thread(&json!({ "prompt": "last one" }))
            .await
            .unwrap();
        let err = rig
            .tools
            .send_message_to_thread(&json!({ "threadId": "b", "text": "one too many" }))
            .await
            .unwrap_err();
        assert!(err.contains("budget"), "{err}");
        assert_eq!(rig.calls.calls.lock().unwrap().len(), SEND_BUDGET);
    }

    #[tokio::test]
    async fn title_and_archive_are_registry_mutations_in_scope() {
        let rig = rig(fixture());
        rig.tools
            .set_thread_title(&json!({ "threadId": "b", "title": "Tests for login" }))
            .await
            .unwrap();
        rig.tools
            .set_thread_archived(&json!({ "threadId": "a", "archived": true }))
            .await
            .unwrap();
        let calls = rig.calls.calls.lock().unwrap().clone();
        assert_eq!(
            calls[0].1,
            json!({ "op": "renameChat", "chatId": "b", "title": "Tests for login" })
        );
        assert_eq!(
            calls[1].1,
            json!({ "op": "setChatArchived", "chatId": "a", "archived": true })
        );
        assert_eq!(
            rig.tools
                .set_thread_archived(&json!({ "threadId": "c", "archived": true }))
                .await
                .unwrap_err(),
            "no thread c in this project"
        );

        let out = rig
            .tools
            .set_thread_archived(&json!({ "threadIds": ["a", "e"], "archived": true }))
            .await
            .unwrap();
        assert_eq!(out["threadIds"], json!(["a", "e"]));
        let calls = rig.calls.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 4);
        assert_eq!(calls[2].1["chatId"], "a");
        assert_eq!(calls[3].1["chatId"], "e");
        // b's turn is running: archiving it would hide live work, so the
        // whole batch is refused. Unarchiving never needs the check.
        let err = rig
            .tools
            .set_thread_archived(&json!({ "threadIds": ["e", "b"], "archived": true }))
            .await
            .unwrap_err();
        assert!(err.starts_with("thread b is working"), "{err}");
        assert_eq!(rig.calls.calls.lock().unwrap().len(), 4);
        rig.tools
            .set_thread_archived(&json!({ "threadId": "b", "archived": false }))
            .await
            .unwrap();
        assert_eq!(rig.calls.calls.lock().unwrap().len(), 5);
        // One out-of-scope id fails the whole batch before any mutation.
        assert_eq!(
            rig.tools
                .set_thread_archived(&json!({ "threadIds": ["b", "c"], "archived": true }))
                .await
                .unwrap_err(),
            "no thread c in this project"
        );
        assert_eq!(rig.calls.calls.lock().unwrap().len(), 5);
        assert_eq!(
            rig.tools
                .set_thread_archived(&json!({ "threadIds": [], "archived": true }))
                .await
                .unwrap_err(),
            "threadId or threadIds is required"
        );
    }

    #[tokio::test]
    async fn wait_threads_returns_when_a_thread_leaves_working() {
        let mut fake = fixture();
        fake.session_snapshots = vec![
            vec![session("b", "working", 0)],
            vec![session("b", "working", 0)],
            vec![session("b", "idle", 0)],
        ];
        let out = rig(fake)
            .tools
            .wait_threads(&json!({ "threadIds": ["b"], "timeoutSeconds": 30 }))
            .await
            .unwrap();
        assert_eq!(out["reason"], "status");
        assert_eq!(out["statuses"]["b"], "idle");
    }

    #[tokio::test]
    async fn wait_threads_treats_a_stale_working_row_as_finished() {
        let mut fake = fixture();
        fake.session_snapshots = vec![vec![session("b", "working", 300)]];
        let out = rig(fake)
            .tools
            .wait_threads(&json!({ "threadIds": ["b"] }))
            .await
            .unwrap();
        assert_eq!(out["reason"], "status");
        assert_eq!(out["statuses"]["b"], "idle");
    }

    #[tokio::test]
    async fn wait_threads_returns_an_inbox_message_and_takes_it_into_the_transcript() {
        let mut fake = fixture();
        fake.session_snapshots = vec![vec![session("b", "working", 0)]];
        fake.queue_snapshots = vec![
            vec![json!({ "id": "q-8", "text": "typed by the human" })],
            vec![
                json!({ "id": "q-8", "text": "typed by the human" }),
                json!({ "id": "q-9", "text": "done, 4 tests", "fromChatId": "b" }),
            ],
        ];
        let rig = rig(fake);
        let out = rig
            .tools
            .wait_threads(&json!({ "threadIds": ["b"], "timeoutSeconds": 30 }))
            .await
            .unwrap();
        assert_eq!(out["reason"], "message");
        assert_eq!(out["messages"][0]["fromChatId"], "b");
        assert_eq!(out["statuses"]["b"], "working");
        let calls = rig.calls.calls.lock().unwrap().clone();
        assert_eq!(
            calls,
            vec![(
                methods::TAKE_THREAD_MESSAGES.to_string(),
                json!({ "chatId": "a" })
            )]
        );
    }

    #[tokio::test]
    async fn wait_threads_returns_a_message_steered_into_this_chats_transcript() {
        let mut fake = fixture();
        fake.session_snapshots = vec![vec![session("b", "working", 0)]];
        // One peer entry already in the transcript must not count as new.
        let mut earlier = entry(
            "m0",
            "user",
            vec![json!({ "kind": "text", "id": "t", "text": "old" })],
        );
        earlier["fromChatId"] = json!("b");
        fake.entries.insert(0, earlier);
        let mut steered = entry(
            "m9",
            "user",
            vec![json!({ "kind": "text", "id": "t", "text": "pong from codex" })],
        );
        steered["fromChatId"] = json!("b");
        fake.doc_frames = vec![
            json!({ "append": [{ "entry": "m5", "part": "t", "text": "…", "len": 7 }], "count": 6 }),
            json!({ "upsert": [{ "after": "m5", "entry": steered }], "count": 7 }),
        ];
        let rig = rig(fake);
        let out = rig
            .tools
            .wait_threads(&json!({ "threadIds": ["b"], "timeoutSeconds": 30 }))
            .await
            .unwrap();
        assert_eq!(out["reason"], "message");
        assert_eq!(
            out["messages"],
            json!([{ "id": "m9", "fromChatId": "b", "text": "pong from codex" }])
        );
        assert_eq!(out["statuses"]["b"], "working");
        assert!(
            rig.calls.calls.lock().unwrap().is_empty(),
            "nothing to take"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn wait_threads_times_out_with_the_last_known_statuses() {
        let mut fake = fixture();
        fake.session_snapshots = vec![vec![session("b", "working", 0)]];
        let out = rig(fake)
            .tools
            .wait_threads(&json!({ "threadIds": ["b"], "timeoutSeconds": 3 }))
            .await
            .unwrap();
        assert_eq!(out["reason"], "timeout");
        assert_eq!(out["statuses"]["b"], "working");
    }

    #[tokio::test]
    async fn serve_answers_initialize_list_and_call_over_one_pipe() {
        let tools = rig(fixture()).tools;
        let (client_side, server_side) = tokio::io::duplex(64 * 1024);
        let (server_read, server_write) = tokio::io::split(server_side);
        let server = tokio::spawn(async move {
            serve(tokio::io::BufReader::new(server_read), server_write, &tools).await
        });
        let (client_read, mut client_write) = tokio::io::split(client_side);
        let mut replies = tokio::io::BufReader::new(client_read).lines();
        let requests = [
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-03-26" } }),
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
            json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "read_thread", "arguments": { "threadId": "b", "limit": 1 } } }),
            json!({ "jsonrpc": "2.0", "id": 4, "method": "nope" }),
        ];
        for request in requests {
            client_write
                .write_all(format!("{request}\n").as_bytes())
                .await
                .unwrap();
        }
        let mut received = Vec::new();
        for _ in 0..4 {
            let line = replies.next_line().await.unwrap().unwrap();
            received.push(serde_json::from_str::<Value>(&line).unwrap());
        }
        let [init, list, call, unknown] = received.as_slice() else {
            panic!("expected four replies");
        };
        assert_eq!(init["id"], 1);
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(list["id"], 2);
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 7);
        assert_eq!(call["id"], 3);
        assert_eq!(call["result"]["isError"], false);
        assert!(
            call["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("line 5")
        );
        assert_eq!(unknown["error"]["code"], -32601);

        // Both halves must go for the duplex to close and the server to see EOF.
        drop(replies);
        drop(client_write);
        server.await.unwrap().unwrap();
    }
}
