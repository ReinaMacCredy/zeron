//! zeron-mcp — the thread tools a chat's agent sees, served over MCP stdio.
//!
//! The engine spawns `zeron mcp --chat <id> --port <ipc>` next to each
//! harness process; the harness lists it as an MCP server, so the agent
//! calls these like any other tool. Newline-delimited JSON-RPC 2.0 on
//! stdin/stdout (the MCP stdio transport). Every tool is a thin read over
//! the engine's own RPC (d28); the caller's identity is the spawn argument,
//! never a tool input.

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
/// A Working row that stops heartbeating only turns stale by the clock, so a
/// wait re-evaluates its snapshot on a timer, not just on pushes.
const STALE_RECHECK: Duration = Duration::from_secs(5);

pub struct ThreadTools {
    client: RpcClient,
    chat_id: String,
}

/// The tool catalog as `tools/list` reports it.
pub fn tool_list() -> Value {
    json!([
        {
            "name": "list_threads",
            "description": "List the chats (threads) in this chat's project with their live status. Peers, not workers: every thread is a real sidebar chat the human can open.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        },
        {
            "name": "read_thread",
            "description": "Read the tail of a thread's transcript. Returns the newest `limit` entries (default 30, max 200) and a `before` cursor for older ones.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "threadId": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 200 },
                    "before": { "type": "string", "description": "An entry id from a previous page; returns entries older than it." }
                },
                "required": ["threadId"],
                "additionalProperties": false
            }
        },
        {
            "name": "wait_threads",
            "description": "Block until any listed thread is no longer working (idle, awaiting_input, errored) or the timeout passes (default 600s, max 1800s). Returns every listed thread's status either way.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "threadIds": { "type": "array", "items": { "type": "string" }, "minItems": 1 },
                    "timeoutSeconds": { "type": "integer", "minimum": 0, "maximum": 1800 }
                },
                "required": ["threadIds"],
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
        }
    }

    pub async fn call(&self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "list_threads" => self.list_threads().await,
            "read_thread" => self.read_thread(args).await,
            "wait_threads" => self.wait_threads(args).await,
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

    pub async fn list_threads(&self) -> Result<Value, String> {
        let chats: Vec<Chat> = self.snapshot(methods::WATCH_CHATS, json!({})).await?;
        let sessions: Vec<Session> = self.snapshot(methods::WATCH_SESSIONS, json!({})).await?;
        let caller = chats
            .iter()
            .find(|chat| chat.id == self.chat_id)
            .ok_or_else(|| format!("this chat ({}) is not in the registry", self.chat_id))?;
        let now = Utc::now();
        let threads: Vec<Value> = chats
            .iter()
            .filter(|chat| {
                !chat.archived
                    && chat.device_id == caller.device_id
                    && chat.space_id == caller.space_id
            })
            .map(|chat| {
                json!({
                    "id": chat.id,
                    "title": chat.title,
                    "status": status_of(&sessions, &chat.id, now),
                    "createdByChatId": chat.created_by_chat_id,
                    "updatedAt": stamp(chat.last_message_at.unwrap_or(chat.created_at)),
                    "isThisChat": chat.id == self.chat_id,
                })
            })
            .collect();
        Ok(json!({ "threads": threads }))
    }

    pub async fn read_thread(&self, args: &Value) -> Result<Value, String> {
        let thread_id = args["threadId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("threadId is required")?;
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
        if ids.is_empty() {
            return Err("threadIds is required".into());
        }
        let timeout = Duration::from_secs(
            args["timeoutSeconds"]
                .as_u64()
                .unwrap_or(DEFAULT_WAIT_SECS)
                .min(MAX_WAIT_SECS),
        );
        let deadline = tokio::time::Instant::now() + timeout;
        let mut items = self
            .client
            .subscribe(methods::WATCH_SESSIONS, json!({}))
            .await
            .map_err(|e| format!("WatchSessions: {e}"))?;
        let parse = |item: Value| -> Result<Vec<Session>, String> {
            serde_json::from_value(item).map_err(|e| format!("WatchSessions: {e}"))
        };
        let statuses = |sessions: &[Session]| -> Value {
            let now = Utc::now();
            ids.iter()
                .map(|id| (id.clone(), Value::from(status_of(sessions, id, now))))
                .collect::<serde_json::Map<_, _>>()
                .into()
        };
        let mut latest = match tokio::time::timeout_at(deadline, items.recv()).await {
            Ok(Some(item)) => parse(item)?,
            Ok(None) => return Err("WatchSessions ended".into()),
            Err(_) => return Ok(json!({ "reason": "timeout", "statuses": statuses(&[]) })),
        };
        loop {
            let current = statuses(&latest);
            if current.as_object().is_some_and(|map| {
                map.values().any(|status| status.as_str() != Some("working"))
            }) {
                return Ok(json!({ "reason": "status", "statuses": current }));
            }
            tokio::select! {
                item = items.recv() => match item {
                    Some(item) => latest = parse(item)?,
                    None => return Err("WatchSessions ended".into()),
                },
                _ = tokio::time::sleep_until(deadline) => {
                    return Ok(json!({ "reason": "timeout", "statuses": current }));
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
        "createdAt": entry["createdAt"],
        "status": entry["status"],
        "text": text,
    })
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
                Ok(value) => (serde_json::to_string_pretty(&value).unwrap_or_default(), false),
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
    use std::sync::Arc;

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

    /// Canned watches: chats once, sessions as a timed sequence of snapshots,
    /// doc messages once.
    struct Fake {
        chats: Vec<Value>,
        session_snapshots: Vec<Vec<Value>>,
        entries: Vec<Value>,
    }

    #[async_trait]
    impl RpcService for Fake {
        async fn handle(&self, method: &str, _params: Value) -> Result<RpcReply, RpcError> {
            match method {
                methods::WATCH_CHATS => Ok(RpcReply::Stream(
                    futures::stream::iter(vec![Value::Array(self.chats.clone())]).boxed(),
                )),
                methods::WATCH_SESSIONS => {
                    let snapshots = self.session_snapshots.clone();
                    Ok(RpcReply::Stream(
                        futures::stream::unfold(snapshots.into_iter(), |mut it| async move {
                            let next = it.next()?;
                            tokio::time::sleep(Duration::from_millis(20)).await;
                            Some((Value::Array(next), it))
                        })
                        .chain(futures::stream::pending())
                        .boxed(),
                    ))
                }
                methods::WATCH_DOC_MESSAGES => Ok(RpcReply::Stream(
                    futures::stream::iter(vec![json!({ "reset": self.entries })]).boxed(),
                )),
                other => Err(RpcError::UnknownMethod(other.into())),
            }
        }
    }

    fn tools(fake: Fake) -> ThreadTools {
        ThreadTools::new(memory_client(Arc::new(fake)), "a")
    }

    fn fixture() -> Fake {
        Fake {
            chats: vec![
                chat("a", Some("s1"), false, None),
                chat("b", Some("s1"), false, Some("a")),
                chat("c", Some("s2"), false, None),
                chat("d", Some("s1"), true, None),
                chat("e", Some("s1"), false, None),
            ],
            session_snapshots: vec![vec![session("b", "working", 0), session("e", "working", 120)]],
            entries: (1..=5)
                .map(|n| {
                    entry(
                        &format!("m{n}"),
                        if n % 2 == 1 { "user" } else { "assistant" },
                        vec![
                            json!({ "kind": "text", "id": "t", "text": format!("line {n}") }),
                            json!({ "kind": "reasoning", "id": "r", "text": "hidden" }),
                            json!({ "kind": "tool", "id": "k", "name": "Bash" }),
                        ],
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn tool_list_names_the_read_tools_only() {
        let list = tool_list();
        let names: Vec<&str> = list
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["list_threads", "read_thread", "wait_threads"]);
    }

    #[tokio::test]
    async fn list_threads_scopes_to_the_callers_space_and_checks_staleness() {
        let out = tools(fixture()).list_threads().await.unwrap();
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
    }

    #[tokio::test]
    async fn read_thread_returns_a_bounded_tail_with_a_cursor() {
        let tools = tools(fixture());
        let page = tools
            .read_thread(&json!({ "threadId": "b", "limit": 2 }))
            .await
            .unwrap();
        let texts: Vec<&str> = page["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["text"].as_str().unwrap())
            .collect();
        assert_eq!(texts, ["line 4\n[tool Bash]", "line 5\n[tool Bash]"]);
        assert_eq!(page["before"], "m4");

        let older = tools
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

        let err = tools.read_thread(&json!({})).await.unwrap_err();
        assert_eq!(err, "threadId is required");
    }

    #[tokio::test]
    async fn wait_threads_returns_when_a_thread_leaves_working() {
        let mut fake = fixture();
        fake.session_snapshots = vec![
            vec![session("b", "working", 0)],
            vec![session("b", "working", 0)],
            vec![session("b", "idle", 0)],
        ];
        let out = tools(fake)
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
        let out = tools(fake)
            .wait_threads(&json!({ "threadIds": ["b", "zz"] }))
            .await
            .unwrap();
        assert_eq!(out["reason"], "status");
        assert_eq!(out["statuses"]["b"], "idle");
        assert_eq!(out["statuses"]["zz"], "idle");
    }

    #[tokio::test(start_paused = true)]
    async fn wait_threads_times_out_with_the_last_known_statuses() {
        let mut fake = fixture();
        fake.session_snapshots = vec![vec![session("b", "working", 0)]];
        let out = tools(fake)
            .wait_threads(&json!({ "threadIds": ["b"], "timeoutSeconds": 3 }))
            .await
            .unwrap();
        assert_eq!(out["reason"], "timeout");
        assert_eq!(out["statuses"]["b"], "working");

        let err = tools(fixture())
            .wait_threads(&json!({ "threadIds": [] }))
            .await
            .unwrap_err();
        assert_eq!(err, "threadIds is required");
    }

    #[tokio::test]
    async fn serve_answers_initialize_list_and_call_over_one_pipe() {
        let tools = tools(fixture());
        let (client_side, server_side) = tokio::io::duplex(64 * 1024);
        let (server_read, server_write) = tokio::io::split(server_side);
        let server = tokio::spawn(async move {
            serve(tokio::io::BufReader::new(server_read), server_write, &tools).await
        });
        let (client_read, mut client_write) = tokio::io::split(client_side);
        let mut replies = tokio::io::BufReader::new(client_read).lines();
        let mut ask = |line: String| {
            let mut line = line;
            line.push('\n');
            line
        };
        client_write
            .write_all(
                ask(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-03-26" } }).to_string())
                    .as_bytes(),
            )
            .await
            .unwrap();
        client_write
            .write_all(ask(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string()).as_bytes())
            .await
            .unwrap();
        client_write
            .write_all(ask(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }).to_string()).as_bytes())
            .await
            .unwrap();
        client_write
            .write_all(
                ask(json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "read_thread", "arguments": { "threadId": "b", "limit": 1 } } }).to_string())
                    .as_bytes(),
            )
            .await
            .unwrap();
        client_write
            .write_all(ask(json!({ "jsonrpc": "2.0", "id": 4, "method": "nope" }).to_string()).as_bytes())
            .await
            .unwrap();

        let init: Value = serde_json::from_str(&replies.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(init["id"], 1);
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        let list: Value = serde_json::from_str(&replies.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(list["id"], 2);
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 3);
        let call: Value = serde_json::from_str(&replies.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(call["id"], 3);
        assert_eq!(call["result"]["isError"], false);
        assert!(call["result"]["content"][0]["text"].as_str().unwrap().contains("line 5"));
        let unknown: Value = serde_json::from_str(&replies.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(unknown["error"]["code"], -32601);

        // Both halves must go for the duplex to close and the server to see EOF.
        drop(replies);
        drop(client_write);
        server.await.unwrap().unwrap();
    }
}
