use serde_json::{Value, json};
use zeron_rpc::methods;

use crate::ThreadTools;
use crate::plugins::tool;

pub(crate) const BOT_MEMORY: &str = "bot_memory";
pub(crate) const BOT_INBOX: &str = "bot_inbox";
pub(crate) const BOT_BOARD: &str = "bot_board";
pub(crate) const BOT_SCHEDULE: &str = "bot_schedule";

pub(crate) fn tools() -> Vec<Value> {
    let s = |what: &str| json!({ "type": "string", "description": what });
    vec![
        tool(
            BOT_MEMORY,
            "Your memory as a Bot: markdown notes kept between your conversations and shared by every session you start. MEMORY.md is the main file (how your owner wants you to work); a rule about one repository or workflow goes in its own file (repos/<name>.md). `list` the notes, `read` one, `write` its whole text, or `delete` it. Write verified gotchas and what you're told to remember; never secrets or task state.",
            json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["list", "read", "write", "delete"] },
                    "path": s("A note's relative .md path (default MEMORY.md)."),
                    "content": s("For write: the note's whole new text."),
                    "botId": s("Another Bot's id (its lead chat). Default: this chat.")
                },
                "required": ["action"]
            }),
        ),
        tool(
            BOT_INBOX,
            "Your owner's Inbox: one card per thing only they can do (approve, decide, review a PR). `create` a card (title: the action they take, never a question; kind approval|decision|review|fyi|other; for a decision 2-4 `options` and the one `recommended`), `update` it when the work under it changes, `close` it when it's answered or moot (`reason`), `resolve` it with their answer (`resolution`), `snooze` it, or `list` the open ones (state open|resolved|closed|snoozed|all). Take the default yourself when a choice can be undone; ask only what is theirs.",
            json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["list", "create", "update", "close", "resolve", "snooze"] },
                    "id": s("The card's id, from list or create."),
                    "title": s("6-8 words: the action the owner takes."),
                    "summary": s("One plain sentence."),
                    "details": s("1-2 short sentences with links."),
                    "kind": { "type": "string", "enum": ["approval", "decision", "review", "fyi", "other"] },
                    "options": { "type": "array", "items": { "type": "string" } },
                    "recommended": s("The option you recommend."),
                    "urgency": { "type": "string", "enum": ["blocking", "important", "low"] },
                    "effort": { "type": "string", "enum": ["quick", "short", "deep"] },
                    "priority": { "type": "number" },
                    "subject": s("The URL it is about: a PR, a doc."),
                    "resolution": s("For resolve: the owner's answer."),
                    "reason": s("For close: why."),
                    "until": s("For snooze: when to bring it back (ISO 8601)."),
                    "state": { "type": "string", "enum": ["open", "resolved", "closed", "snoozed", "all"] },
                    "botId": s("Another Bot's id (its lead chat). Default: this chat.")
                },
                "required": ["action"]
            }),
        ),
        tool(
            BOT_BOARD,
            "Your board as a Bot: each task's status (up_next, working, in_review, complete), the chat working it, its folder and a short status note. `claim` a task for the chat doing it (it moves to working; refused when another chat holds it unless `takeOver`), `release` it when that chat stops, set its `status` (in_review while a PR or handoff waits on someone, complete when it's done), leave a `note` (1-3 short lines: what was done, what to look at), file it in a `folder`, or `list` the tasks (optionally one folder's).",
            json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["list", "claim", "release", "status", "note", "folder"] },
                    "taskId": s("The task's id (from tasks)."),
                    "chatId": s("For claim: the chat working it."),
                    "takeOver": { "type": "boolean" },
                    "status": { "type": "string", "enum": ["up_next", "working", "in_review", "complete"] },
                    "note": s("For note: 1-3 short lines."),
                    "folder": s("For folder: its name; for list: only this folder."),
                    "botId": s("Another Bot's id (its lead chat). Default: this chat.")
                },
                "required": ["action"]
            }),
        ),
        tool(
            BOT_SCHEDULE,
            "Your schedules and promises as a Bot, kept and fired by Zeron's engine even with no window open: a `schedule` sends its `prompt` to this chat `at` a time (Unix ms) or every `everyMinutes`; a `promise` (kind promise) records an `outcome` you told someone they'll get, owed at `replyTo`, and reminds you at `dueAt`. `list` them, `pause`/`resume` one, `redate` a promise (new `at`), `close` it once delivered, or `delete` one.",
            json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["list", "create", "pause", "resume", "redate", "close", "delete"] },
                    "id": s("The schedule's id, from list or create."),
                    "kind": { "type": "string", "enum": ["schedule", "promise"] },
                    "prompt": s("For a schedule: what you're asked when it fires."),
                    "everyMinutes": { "type": "integer", "minimum": 1 },
                    "at": { "type": "integer", "description": "When (Unix ms)." },
                    "dueAt": { "type": "integer", "description": "For a promise: when it's owed (Unix ms)." },
                    "outcome": s("For a promise: what you said they'd get."),
                    "replyTo": s("For a promise: where the result is owed."),
                    "botId": s("Another Bot's id (its lead chat). Default: this chat.")
                },
                "required": ["action"]
            }),
        ),
    ]
}

impl ThreadTools {
    pub(crate) async fn bot_files(&self, method: &str, args: &Value) -> Result<Value, String> {
        let mut args = if args.is_null() {
            json!({})
        } else {
            args.clone()
        };
        let Some(map) = args.as_object_mut() else {
            return Err("invalid arguments: expected an object".into());
        };
        if !map
            .get("botId")
            .is_some_and(|v| v.as_str().is_some_and(|s| !s.trim().is_empty()))
        {
            map.insert("botId".into(), json!(self.chat_id));
        }
        let method = match method {
            BOT_MEMORY => methods::BOT_MEMORY,
            BOT_BOARD => methods::BOT_BOARD,
            BOT_SCHEDULE => methods::BOT_SCHEDULE,
            _ => methods::BOT_INBOX,
        };
        self.client
            .call(method, args)
            .await
            .map_err(|e| e.to_string())
    }

    /// What a Bot is told as its session starts (MCP `instructions`): its
    /// memory's MEMORY.md and its open Inbox cards. None for a chat with
    /// neither.
    pub(crate) async fn bot_instructions(&self) -> Option<String> {
        let me = json!({ "botId": self.chat_id });
        let mut read = me.clone();
        read["action"] = json!("read");
        let memory = self.client.call(methods::BOT_MEMORY, read).await.ok()?;
        let memory = memory["content"].as_str().unwrap_or("").trim().to_owned();
        let mut list = me;
        list["action"] = json!("list");
        let open = self.client.call(methods::BOT_INBOX, list).await.ok();
        let cards: Vec<String> = open
            .as_ref()
            .and_then(|o| o["items"].as_array())
            .map(|items| {
                items
                    .iter()
                    .map(|i| {
                        format!(
                            "- {} ({}, {}): {}",
                            i["id"].as_str().unwrap_or(""),
                            i["kind"].as_str().unwrap_or(""),
                            i["urgency"].as_str().unwrap_or(""),
                            i["title"].as_str().unwrap_or("")
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        if memory.is_empty() && cards.is_empty() {
            return None;
        }
        let mut out = String::from(
            "You are a Bot: a lead that keeps its own memory (bot_memory), files cards for what only your owner can do (bot_inbox) and tracks its tasks on a board (bot_board).",
        );
        if !memory.is_empty() {
            out.push_str("\n\nYour memory (MEMORY.md):\n");
            out.push_str(&memory);
        }
        if !cards.is_empty() {
            out.push_str("\n\nOpen Inbox cards:\n");
            out.push_str(&cards.join("\n"));
        }
        Some(out)
    }
}
