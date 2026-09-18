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
            "description": "List the chats (threads) in this chat's project with their live status. Threads are peers, not workers: every one is a real sidebar chat the human can open, and none can stop another.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
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
            "description": "Open a new peer thread in this project and give it `prompt` as its first message. It runs this chat's harness, model and reasoning level unless `harness`, `model` or `reasoning` pick others; an unknown harness or reasoning level is an error. Counts against this turn's send budget.",
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
            "description": "Send a message to another thread in this project. An idle thread starts a turn with it; a working thread receives it when its turn ends (or sooner, if it is waiting with wait_threads). Never to this chat itself. Counts against this turn's send budget.",
            "inputSchema": {
                "type": "object",
                "properties": { "threadId": thread_id, "text": { "type": "string" } },
                "required": ["threadId", "text"],
                "additionalProperties": false
            }
        },
        {
            "name": "wait_threads",
            "description": "Block until any listed thread is no longer working (idle, awaiting_input, errored), a message from another thread arrives for this chat, or the timeout passes (default 600s, max 1800s). Returns the listed threads' statuses either way.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "threadIds": { "type": "array", "items": { "type": "string" } },
                    "timeoutSeconds": { "type": "integer", "minimum": 0, "maximum": 1800 }
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
            "description": "Archive or unarchive a thread in this project. Archiving never cascades to other threads.",
            "inputSchema": {
                "type": "object",
                "properties": { "threadId": thread_id, "archived": { "type": "boolean" } },
                "required": ["threadId", "archived"],
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
            "list_threads" => self.list_threads().await,
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

    pub async fn list_threads(&self) -> Result<Value, String> {
        let (_, peers) = self.peers().await?;
        let sessions: Vec<Session> = self.snapshot(methods::WATCH_SESSIONS, json!({})).await?;
        let now = Utc::now();
        let threads: Vec<Value> = peers
            .iter()
            .filter(|chat| !chat.archived)
            .map(|chat| {
                json!({
                    "id": chat.id,
                    "title": chat.title,
                    "status": status_of(&sessions, &chat.id, now),
                    "harness": chat.config.as_ref().map(|c| c.harness),
                    "model": chat.config.as_ref().and_then(|c| c.model.as_deref()),
                    "reasoning": chat.config.as_ref().and_then(|c| c.reasoning),
                    "createdByChatId": chat.created_by_chat_id,
                    "updatedAt": stamp(chat.last_message_at.unwrap_or(chat.created_at)),
                    "isThisChat": chat.id == self.chat_id,
                })
            })
            .collect();
        Ok(json!({ "threads": threads }))
    }

    pub async fn read_thread(&self, args: &Value) -> Result<Value, String> {
        let thread_id = Self::thread_id(args)?;
        self.peer(thread_id).await?;
        let limit = args["limit"]
            .as_u64()
            .map(|n| n as usize)
            .unwrap_or(DEFAULT_TAIL)
            .clamp(1, MAX_TAIL);
        let snapshot: Value = self
            .snapshot(methods::WATCH_DOC_MESSAGES, json!({ "chatId": thread_id }))
            .await?;
        let entries = snapshot["reset"].as_array().cloned().unwrap_or_default();
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
        Ok(json!({ "threadId": created["chatId"] }))
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
        let thread_id = Self::thread_id(args)?;
        let archived = args["archived"].as_bool().ok_or("archived is required")?;
        self.peer(thread_id).await?;
        self.call_engine(
            methods::MUTATE,
            json!({ "op": "setChatArchived", "chatId": thread_id, "archived": archived }),
        )
        .await?;
        Ok(json!({ "threadId": thread_id, "archived": archived }))
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
        let ids: Vec<String> = args["threadIds"]
            .as_array()
            .map(|ids| {
                ids.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
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
                if current
                    .as_object()
                    .is_some_and(|map| map.values().any(|s| s.as_str() != Some("working")))
                {
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
                item = queue.recv() => match item {
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
    let text = entry["parts"]
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| match part["kind"].as_str().unwrap_or("") {
                    "text" => part["text"].as_str().map(str::to_string),
                    "reasoning" => None,
                    "image" => Some(format!(
                        "[image {}]",
                        part["name"].as_str().unwrap_or("attachment")
                    )),
                    "tool" => Some(format!("[tool {}]", tool_label(part))),
                    kind => Some(match part["name"].as_str() {
                        Some(name) => format!("[{kind} {name}]"),
                        None => format!("[{kind}]"),
                    }),
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    json!({
        "id": entry["id"],
        "role": entry["role"],
        "fromChatId": entry["fromChatId"],
        "createdAt": entry["createdAt"],
        "status": entry["status"],
        "text": text,
    })
}

/// A tool part's one-word identity for the transcript tail: the decoded
/// call's kind plus its headline field (`exec ls -la`, `mcp zeron/read_thread`,
/// `readFile src/x.rs`), or the raw name older rows carry.
fn tool_label(part: &Value) -> String {
    let call = &part["call"];
    let kind = call["kind"].as_str();
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
                methods::WATCH_DOC_MESSAGES => Ok(RpcReply::Stream(
                    futures::stream::iter(vec![json!({ "reset": self.entries })]).boxed(),
                )),
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
            ],
            session_snapshots: vec![vec![
                session("b", "working", 0),
                session("e", "working", 120),
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
    async fn list_threads_scopes_to_the_callers_space_and_checks_staleness() {
        let out = rig(fixture()).tools.list_threads().await.unwrap();
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
            ]
        );
        assert_eq!(out["threads"][1]["createdByChatId"], "a");
        assert_eq!(out["threads"][1]["harness"], "codex");
        assert_eq!(out["threads"][1]["model"], "gpt-5");
        assert_eq!(out["threads"][1]["reasoning"], "high");
        assert!(
            out["threads"][0]["harness"].is_null(),
            "no config, no settings"
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
            .wait_threads(&json!({ "threadIds": ["b", "zz"] }))
            .await
            .unwrap();
        assert_eq!(out["reason"], "status");
        assert_eq!(out["statuses"]["b"], "idle");
        assert_eq!(out["statuses"]["zz"], "idle");
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
