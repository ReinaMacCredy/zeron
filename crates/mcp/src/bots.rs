use serde_json::{Value, json};
use zeron_rpc::methods;

use crate::ThreadTools;
use crate::plugins::tool;

pub(crate) const BOT_MEMORY: &str = "bot_memory";
pub(crate) const BOT_INBOX: &str = "bot_inbox";

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
        let method = if method == BOT_MEMORY {
            methods::BOT_MEMORY
        } else {
            methods::BOT_INBOX
        };
        self.client
            .call(method, args)
            .await
            .map_err(|e| e.to_string())
    }
}
