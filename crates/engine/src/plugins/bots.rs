//! A Bot's own files: its memory (markdown notes it keeps between
//! conversations) and its Inbox (cards for what only its owner can do:
//! approve, decide, review). A Bot is a lead chat; its files live under
//! `{data_dir}/bots/<botId>/`, so they outlive any one of its sessions.

use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value as Json, json};
use zeron_rpc::methods;

const MEMORY_MAX_BYTES: usize = 64 * 1024;
const INBOX_MAX: usize = 200;
const KINDS: [&str; 5] = ["approval", "decision", "review", "fyi", "other"];
const URGENCIES: [&str; 3] = ["blocking", "important", "low"];

pub fn handles(method: &str) -> bool {
    matches!(
        method,
        methods::BOT_MEMORY | methods::BOT_INBOX | methods::BOT_BOARD
    )
}

fn text<'a>(params: &'a Json, key: &str) -> Option<&'a str> {
    params
        .get(key)
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn bot_dir(data_dir: &Path, params: &Json) -> Result<PathBuf, String> {
    let bot = text(params, "botId").ok_or("Say which Bot: `botId` (its lead chat's id).")?;
    if !bot
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("`botId` is a chat id: letters, digits, - and _".into());
    }
    Ok(data_dir.join("bots").join(bot))
}

/// A note's path inside the memory folder: relative, no `..`, markdown.
fn note_path(rel: &str) -> Result<PathBuf, String> {
    let path = Path::new(rel);
    let plain = path.components().all(|c| matches!(c, Component::Normal(_)));
    if !plain || !rel.ends_with(".md") {
        return Err(format!(
            "`{rel}`: a memory note is a relative .md path, like MEMORY.md or repos/zeron.md"
        ));
    }
    Ok(path.to_path_buf())
}

fn list_notes(root: &Path, at: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(at) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            list_notes(root, &path, out);
        } else if path.extension().is_some_and(|e| e == "md")
            && let Ok(rel) = path.strip_prefix(root)
        {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

fn memory(data_dir: &Path, params: &Json) -> Result<Json, String> {
    let root = bot_dir(data_dir, params)?.join("memory");
    match text(params, "action").unwrap_or("list") {
        "list" => {
            let mut notes = Vec::new();
            list_notes(&root, &root, &mut notes);
            notes.sort();
            Ok(json!({ "notes": notes }))
        }
        "read" => {
            let rel = text(params, "path").unwrap_or("MEMORY.md");
            let content = std::fs::read_to_string(root.join(note_path(rel)?)).unwrap_or_default();
            Ok(json!({ "path": rel, "content": content }))
        }
        "write" => {
            let rel = text(params, "path").unwrap_or("MEMORY.md");
            let content = params
                .get("content")
                .and_then(Json::as_str)
                .ok_or("`content`: the note's whole text")?;
            if content.len() > MEMORY_MAX_BYTES {
                return Err(format!("A note holds at most {MEMORY_MAX_BYTES} bytes"));
            }
            let path = root.join(note_path(rel)?);
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            let tmp = path.with_extension("md.tmp");
            std::fs::write(&tmp, content)
                .and_then(|()| std::fs::rename(&tmp, &path))
                .map_err(|e| e.to_string())?;
            Ok(json!({ "path": rel, "written": content.len() }))
        }
        "delete" => {
            let rel = text(params, "path").ok_or("`path`: the note to delete")?;
            let _ = std::fs::remove_file(root.join(note_path(rel)?));
            Ok(json!({ "path": rel, "deleted": true }))
        }
        other => Err(format!("`action` {other}: list, read, write or delete")),
    }
}

fn read_inbox(path: &Path) -> Vec<Json> {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_inbox(path: &Path, items: &[Json]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let bytes = serde_json::to_vec_pretty(items).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes)
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|e| e.to_string())
}

fn one_of(params: &Json, key: &str, allowed: &[&str], default: &str) -> Result<String, String> {
    let v = text(params, key).unwrap_or(default);
    if allowed.contains(&v) {
        Ok(v.to_owned())
    } else {
        Err(format!("`{key}`: one of {}", allowed.join(", ")))
    }
}

/// Open first, then by priority (highest first), then the newest.
fn ordered(mut items: Vec<Json>) -> Vec<Json> {
    let key = |i: &Json| {
        (
            i["status"] != "open",
            -(i["priority"].as_f64().unwrap_or(0.0) * 1000.0) as i64,
            -i["createdAt"].as_i64().unwrap_or(0),
        )
    };
    items.sort_by_key(key);
    items
}

const EDITABLE: [&str; 7] = [
    "title",
    "summary",
    "details",
    "options",
    "recommended",
    "subject",
    "effort",
];

fn inbox(data_dir: &Path, params: &Json) -> Result<Json, String> {
    let path = bot_dir(data_dir, params)?.join("inbox.json");
    let mut items = read_inbox(&path);
    let find = |items: &mut Vec<Json>| -> Result<usize, String> {
        let id = text(params, "id").ok_or("`id`: the card's id, from list")?;
        items
            .iter()
            .position(|i| i["id"] == id)
            .ok_or_else(|| format!("No card {id}"))
    };
    match text(params, "action").unwrap_or("list") {
        "list" => {
            let state = text(params, "state").unwrap_or("open");
            let shown: Vec<Json> = items
                .into_iter()
                .filter(|i| state == "all" || i["status"] == state)
                .collect();
            Ok(json!({ "items": ordered(shown) }))
        }
        "create" => {
            let title = text(params, "title")
                .ok_or("`title`: the action the owner takes, never a question")?;
            if items.iter().filter(|i| i["status"] == "open").count() >= INBOX_MAX {
                return Err(format!("At most {INBOX_MAX} open cards"));
            }
            let now = now_ms();
            let id = format!("ask-{now:x}-{}", items.len());
            let mut card = json!({
                "id": id,
                "kind": one_of(params, "kind", &KINDS, "other")?,
                "title": title,
                "urgency": one_of(params, "urgency", &URGENCIES, "important")?,
                "priority": params.get("priority").and_then(Json::as_f64).unwrap_or(0.0),
                "status": "open",
                "createdAt": now,
                "engagedAt": now,
            });
            for key in EDITABLE {
                if let Some(v) = params.get(key).filter(|v| !v.is_null()) {
                    card[key] = v.clone();
                }
            }
            items.push(card.clone());
            write_inbox(&path, &items)?;
            Ok(card)
        }
        "update" => {
            let at = find(&mut items)?;
            if items[at]["status"] != "open" {
                return Err("That card is handled already".into());
            }
            for key in EDITABLE {
                if let Some(v) = params.get(key) {
                    items[at][key] = v.clone();
                }
            }
            if let Some(p) = params.get("priority").and_then(Json::as_f64) {
                items[at]["priority"] = json!(p);
            }
            if text(params, "urgency").is_some() {
                items[at]["urgency"] = json!(one_of(params, "urgency", &URGENCIES, "important")?);
            }
            let card = items[at].clone();
            write_inbox(&path, &items)?;
            Ok(card)
        }
        verb @ ("close" | "resolve" | "snooze") => {
            let at = find(&mut items)?;
            let status = match verb {
                "close" => "closed",
                "resolve" => "resolved",
                _ => "snoozed",
            };
            items[at]["status"] = json!(status);
            items[at]["handledAt"] = json!(now_ms());
            if let Some(v) = text(params, "resolution").or(text(params, "reason")) {
                items[at]["resolution"] = json!(v);
            }
            if let Some(until) = params.get("until").filter(|v| !v.is_null()) {
                items[at]["until"] = until.clone();
            }
            let card = items[at].clone();
            write_inbox(&path, &items)?;
            Ok(card)
        }
        other => Err(format!(
            "`action` {other}: list, create, update, close, resolve or snooze"
        )),
    }
}

const STATUSES: [&str; 4] = ["up_next", "working", "in_review", "complete"];

/// A Bot's board over its tasks: each task's status (Up next, Working, In
/// review, Complete), the chat working it, its folder and a short note,
/// by task id. A task with no row reads Up next.
fn board(data_dir: &Path, params: &Json) -> Result<Json, String> {
    let path = bot_dir(data_dir, params)?.join("board.json");
    let mut rows: serde_json::Map<String, Json> = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let action = text(params, "action").unwrap_or("list");
    if action == "list" {
        let folder = text(params, "folder");
        let tasks: Vec<Json> = rows
            .iter()
            .filter(|(_, r)| folder.is_none_or(|f| r["folder"] == f))
            .map(|(id, r)| {
                let mut r = r.clone();
                r["taskId"] = json!(id);
                r
            })
            .collect();
        return Ok(json!({ "tasks": tasks }));
    }
    let task = text(params, "taskId")
        .ok_or("`taskId`: the task's id")?
        .to_owned();
    let row = rows
        .entry(task.clone())
        .or_insert_with(|| json!({ "status": "up_next" }));
    match action {
        "claim" => {
            let by = text(params, "chatId").ok_or("`chatId`: the chat taking the task")?;
            if let Some(held) = row["claimedBy"].as_str().filter(|h| *h != by)
                && !params
                    .get("takeOver")
                    .and_then(Json::as_bool)
                    .unwrap_or(false)
            {
                return Err(format!(
                    "{task} is claimed by {held}; pass takeOver to take it"
                ));
            }
            row["claimedBy"] = json!(by);
            if row["status"] == "up_next" {
                row["status"] = json!("working");
            }
        }
        "release" => {
            if let Some(obj) = row.as_object_mut() {
                obj.remove("claimedBy");
            }
        }
        "status" => row["status"] = json!(one_of(params, "status", &STATUSES, "working")?),
        "note" => row["note"] = json!(text(params, "note").unwrap_or("")),
        "folder" => row["folder"] = json!(text(params, "folder").ok_or("`folder`: its name")?),
        other => {
            return Err(format!(
                "`action` {other}: list, claim, release, status, note or folder"
            ));
        }
    }
    row["updatedAt"] = json!(now_ms());
    let mut out = row.clone();
    out["taskId"] = json!(task);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let bytes = serde_json::to_vec_pretty(&rows).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes)
        .and_then(|()| std::fs::rename(&tmp, &path))
        .map_err(|e| e.to_string())?;
    Ok(out)
}

const MINUTE_MS: i64 = 60_000;

/// A Bot's schedules: a prompt sent to the Bot's own chat at a time
/// (`at`, Unix ms) or every `everyMinutes`, and promises (`kind: promise`:
/// an `outcome` owed by `dueAt`, the Bot reminded then). The engine sends
/// what is due (see [`fire_due`]), so they run with no window open.
fn schedule(data_dir: &Path, params: &Json) -> Result<Json, String> {
    let path = bot_dir(data_dir, params)?.join("schedules.json");
    let mut items = read_inbox(&path);
    let action = text(params, "action").unwrap_or("list");
    if action == "list" {
        return Ok(json!({ "schedules": items }));
    }
    if action == "create" {
        let now = now_ms();
        let kind = one_of(params, "kind", &["schedule", "promise"], "schedule")?;
        let every = params
            .get("everyMinutes")
            .and_then(Json::as_i64)
            .filter(|m| *m >= 1);
        let at = params
            .get("at")
            .or(params.get("dueAt"))
            .and_then(Json::as_i64);
        let prompt = if kind == "promise" {
            let outcome =
                text(params, "outcome").ok_or("`outcome`: what you told them they'd get")?;
            format!(
                "A promise is due: {outcome}. Deliver it where it is owed ({}), or tell them where it stands and re-date it.",
                text(params, "replyTo").unwrap_or("where you said")
            )
        } else {
            text(params, "prompt")
                .ok_or("`prompt`: what the Bot is asked when it fires")?
                .to_owned()
        };
        let next = match (at, every) {
            (Some(at), _) => at,
            (None, Some(m)) => now + m * MINUTE_MS,
            (None, None) => return Err("Say when: `at` (Unix ms) or `everyMinutes`".into()),
        };
        let item = json!({
            "id": format!("sch-{now:x}-{}", items.len()),
            "kind": kind,
            "prompt": prompt,
            "outcome": text(params, "outcome"),
            "replyTo": text(params, "replyTo"),
            "everyMinutes": every,
            "nextAt": next,
            "enabled": true,
            "createdAt": now,
        });
        items.push(item.clone());
        write_inbox(&path, &items)?;
        return Ok(item);
    }
    let id = text(params, "id").ok_or("`id`: the schedule's id, from list")?;
    let at = items
        .iter()
        .position(|i| i["id"] == id)
        .ok_or_else(|| format!("No schedule {id}"))?;
    let out = match action {
        "pause" | "resume" => {
            items[at]["enabled"] = json!(action == "resume");
            items[at].clone()
        }
        "redate" => {
            let next = params
                .get("at")
                .or(params.get("dueAt"))
                .and_then(Json::as_i64)
                .ok_or("`at`: the new time (Unix ms)")?;
            items[at]["nextAt"] = json!(next);
            items[at]["enabled"] = json!(true);
            items[at].clone()
        }
        "delete" | "close" => items.remove(at),
        other => {
            return Err(format!(
                "`action` {other}: list, create, pause, resume, redate, close or delete"
            ));
        }
    };
    write_inbox(&path, &items)?;
    Ok(out)
}

/// The prompts due by `now` across every Bot, as `(botId, prompt)`: each
/// moved on (an `everyMinutes` one to its next time; a one-off disabled).
pub fn take_due(data_dir: &Path, now: i64) -> Vec<(String, String)> {
    let mut due = Vec::new();
    let Ok(bots) = std::fs::read_dir(data_dir.join("bots")) else {
        return due;
    };
    for bot in bots.flatten() {
        let path = bot.path().join("schedules.json");
        let mut items = read_inbox(&path);
        let mut moved = false;
        for item in items.iter_mut() {
            let next = item["nextAt"].as_i64().unwrap_or(i64::MAX);
            if item["enabled"] != true || next > now {
                continue;
            }
            due.push((
                bot.file_name().to_string_lossy().into_owned(),
                item["prompt"].as_str().unwrap_or("").to_owned(),
            ));
            match item["everyMinutes"].as_i64().filter(|m| *m >= 1) {
                Some(m) => {
                    item["nextAt"] =
                        json!(next + ((now - next) / (m * MINUTE_MS) + 1) * m * MINUTE_MS)
                }
                None => item["enabled"] = json!(false),
            }
            moved = true;
        }
        if moved {
            let _ = write_inbox(&path, &items);
        }
    }
    due
}

/// `method` with `params`, over the files in `data_dir`.
pub fn answer(data_dir: &Path, method: &str, params: &Json) -> Result<Json, String> {
    match method {
        methods::BOT_MEMORY => memory(data_dir, params),
        methods::BOT_INBOX => inbox(data_dir, params),
        methods::BOT_BOARD => board(data_dir, params),
        methods::BOT_SCHEDULE => schedule(data_dir, params),
        _ => Err(format!("{method}: not a Bot method")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(dir: &Path, method: &str, params: Json) -> Result<Json, String> {
        answer(dir, method, &params)
    }

    #[test]
    fn memory_keeps_notes_inside_the_bot_s_folder() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        call(d, methods::BOT_MEMORY, json!({ "botId": "b1", "action": "write", "path": "repos/zeron.md", "content": "- rule" })).unwrap();
        let read = call(
            d,
            methods::BOT_MEMORY,
            json!({ "botId": "b1", "action": "read", "path": "repos/zeron.md" }),
        )
        .unwrap();
        assert_eq!(read["content"], "- rule");
        let listed = call(d, methods::BOT_MEMORY, json!({ "botId": "b1" })).unwrap();
        assert_eq!(listed["notes"], json!(["repos/zeron.md"]));
        for bad in ["../x.md", "/etc/x.md", "notes.txt"] {
            assert!(
                call(
                    d,
                    methods::BOT_MEMORY,
                    json!({ "botId": "b1", "action": "write", "path": bad, "content": "" })
                )
                .is_err(),
                "{bad}"
            );
        }
        assert!(
            call(
                d,
                methods::BOT_MEMORY,
                json!({ "botId": "../b", "action": "list" })
            )
            .is_err()
        );
    }

    #[test]
    fn board_claims_once_and_moves_to_working() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let at = |p: Json| call(d, methods::BOT_BOARD, p);
        let row =
            at(json!({ "botId": "b", "action": "claim", "taskId": "t1", "chatId": "c1" })).unwrap();
        assert_eq!(
            (row["status"].as_str(), row["claimedBy"].as_str()),
            (Some("working"), Some("c1"))
        );
        assert!(
            at(json!({ "botId": "b", "action": "claim", "taskId": "t1", "chatId": "c2" })).is_err()
        );
        at(json!({ "botId": "b", "action": "status", "taskId": "t1", "status": "in_review" }))
            .unwrap();
        at(json!({ "botId": "b", "action": "folder", "taskId": "t1", "folder": "Zeron" })).unwrap();
        let listed = at(json!({ "botId": "b", "folder": "Zeron" })).unwrap();
        assert_eq!(listed["tasks"][0]["status"], "in_review");
        assert!(
            at(json!({ "botId": "b", "action": "status", "taskId": "t1", "status": "done" }))
                .is_err()
        );
    }

    #[test]
    fn schedules_fire_once_or_on_their_interval() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let at = |p: Json| call(d, methods::BOT_SCHEDULE, p);
        at(json!({ "botId": "b", "action": "create", "prompt": "Sweep", "at": 1_000 })).unwrap();
        at(json!({ "botId": "b", "action": "create", "prompt": "Weekly", "everyMinutes": 10, "at": 1_000 })).unwrap();
        at(json!({ "botId": "b", "action": "create", "kind": "promise", "outcome": "the fix PR", "dueAt": 5_000_000 })).unwrap();
        assert!(at(json!({ "botId": "b", "action": "create", "prompt": "x" })).is_err());
        let due = take_due(d, 2_000);
        assert_eq!(
            due,
            [
                ("b".to_string(), "Sweep".to_string()),
                ("b".to_string(), "Weekly".to_string())
            ]
        );
        assert!(take_due(d, 2_000).is_empty());
        let due = take_due(d, 1_000 + 10 * MINUTE_MS);
        assert_eq!(due.len(), 1);
        assert!(
            take_due(d, 5_000_000)
                .iter()
                .any(|(_, said)| said.contains("the fix PR"))
        );
    }

    #[test]
    fn inbox_orders_open_cards_and_keeps_handled_ones() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let low = call(d, methods::BOT_INBOX, json!({ "botId": "b", "action": "create", "title": "Merge the dead code removal", "kind": "review" })).unwrap();
        let high = call(d, methods::BOT_INBOX, json!({ "botId": "b", "action": "create", "title": "Pick the launch tone", "kind": "decision", "options": ["A", "B"], "priority": 2 })).unwrap();
        let open = call(d, methods::BOT_INBOX, json!({ "botId": "b" })).unwrap();
        assert_eq!(open["items"][0]["id"], high["id"]);
        call(
            d,
            methods::BOT_INBOX,
            json!({ "botId": "b", "action": "resolve", "id": high["id"], "resolution": "A" }),
        )
        .unwrap();
        let open = call(d, methods::BOT_INBOX, json!({ "botId": "b" })).unwrap();
        assert_eq!(open["items"].as_array().unwrap().len(), 1);
        assert_eq!(open["items"][0]["id"], low["id"]);
        let all = call(
            d,
            methods::BOT_INBOX,
            json!({ "botId": "b", "state": "all" }),
        )
        .unwrap();
        assert_eq!(all["items"][1]["resolution"], "A");
        assert!(
            call(
                d,
                methods::BOT_INBOX,
                json!({ "botId": "b", "action": "create", "title": "x", "kind": "nope" })
            )
            .is_err()
        );
    }
}
