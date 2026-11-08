//! Tasks: the engine's to-do list (`WatchTasks`, `Mutate` task ops,
//! crates/engine/src/rpc/tasks.rs), as the Tasks page, the Inbox page, the
//! card and quick capture read and write it (ui/screens/tasks).
//!
//! `tasks.list` is the list as scripts read it: `{ state, tasks }`, `state`
//! "idle" until the engine answers, then "ready". Each task is `{ id,
//! title, notes, project?, due?, chat?, completed? (ms), order, created
//! (ms), images, send_error?, plan?, depends }`, deleted ones left out.
//!
//!   tasks.create { title, notes?, project?, due?, images?, depends? }
//!                                  a new task at the top of To do: its id
//!   tasks.update { id, title?, notes?, due? ("" none), project? ("" none),
//!                  completed?, images? }
//!   tasks.delete { id }
//!   tasks.assign { id, chat, send? }   linked to a thread (sent its words
//!                                  with `send`); `chat` "" unlinks
//!   tasks.start { id, picks?, project? }  handed to a new chat on these picks
//!                                  (the new chat composer's), its id
//!   tasks.today { since }           today's runs and waits (`GetTodayRuns`)
//!   tasks.image { path }            an image of a task, here: { src }
//!   tasks.upload_image { path }     a file uploaded for a task: its path
//!
//! A mock Dev (`ZERON_ZGX_MOCK=1`) with `ZERON_ZGX_MOCK_TASKS=<n>` seeds n
//! tasks into an engine that has none.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use serde_json::{Value as Json, json};
use zgx_desktop::WindowContext;
use zgx_script::{ScriptHost, Shared, Value};

use crate::auth::text;
use crate::engine;

#[derive(Default)]
struct Tasks {
    list: RefCell<Json>,
    ready: Cell<bool>,
    seeded: Cell<bool>,
    shared: RefCell<Vec<(u64, Shared)>>,
    next_window: Cell<u64>,
    watching: Cell<bool>,
    watch: RefCell<Option<crate::engine::rpc::Watch>>,
}

thread_local! {
    static TASKS: Rc<Tasks> = Rc::new(Tasks::default());
}

/// A task as scripts read it, or none for a deleted one.
fn shaped(task: &Json) -> Option<Json> {
    let t: zeron_proto::Task = serde_json::from_value(task.clone()).ok()?;
    if t.deleted {
        return None;
    }
    Some(json!({
        "id": t.id,
        "title": t.title,
        "notes": t.notes,
        "project": t.space_id,
        "due": t.due_on,
        "chat": t.chat_id,
        "completed": t.completed_at.map(|at| at.timestamp_millis()),
        "order": t.order_key,
        "created": t.created_at.timestamp_millis(),
        "images": t.images,
        "send_error": t.send_error,
        "plan": t.mode == zeron_proto::TaskMode::Orchestrated || t.plan_pending,
        "depends": t.depends_on,
    }))
}

fn shaped_all(tasks: &Json) -> Json {
    Json::Array(tasks.as_array().into_iter().flatten().filter_map(shaped).collect())
}

/// An order key above every open task's, as the shipping app's quick add.
fn top_order_key(tasks: &Json, nonce: &str) -> Option<String> {
    let first = tasks
        .as_array()
        .into_iter()
        .flatten()
        .filter(|t| !t["deleted"].as_bool().unwrap_or(false) && t["completedAt"].is_null())
        .filter_map(|t| t["orderKey"].as_str())
        .filter(|k| !k.is_empty())
        .min();
    zeron_proto::pin_order_key_between(None, first, nonce).ok()
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.map(Value::list).unwrap_or_default().iter().map(text).filter(|s| !s.is_empty()).collect()
}

/// `tasks.update`'s args as the engine's patch (`TaskPatch`): "" clears a
/// due day or a project.
fn patch_of(args: &Value) -> Json {
    let mut patch = json!({});
    for (key, wire) in [("title", "title"), ("notes", "notes")] {
        if let Some(v) = args.field(key) {
            patch[wire] = json!(text(v));
        }
    }
    for (key, wire) in [("due", "dueOn"), ("project", "spaceId")] {
        if let Some(v) = args.field(key) {
            let v = text(v);
            patch[wire] = if v.is_empty() { Json::Null } else { json!(v) };
        }
    }
    if let Some(v) = args.field("completed") {
        patch["completed"] = json!(v.truthy());
    }
    if args.field("images").is_some() {
        patch["images"] = json!(strings(args.field("images")));
    }
    patch
}

fn create_params(args: &Value, id: &str, order: Option<String>) -> Json {
    let field = |key: &str| args.field(key).map(text).filter(|s| !s.is_empty());
    let mut params = json!({
        "op": "createTask",
        "taskId": id,
        "title": field("title").unwrap_or_default().trim(),
        "notes": field("notes").unwrap_or_default(),
        "images": strings(args.field("images")),
        "dependsOn": strings(args.field("depends")),
    });
    if let Some(project) = field("project") {
        params["spaceId"] = json!(project);
    }
    if let Some(due) = field("due") {
        params["dueOn"] = json!(due);
    }
    if let Some(order) = order {
        params["orderKey"] = json!(order);
    }
    params
}

impl Tasks {
    fn value(&self) -> Value {
        let tasks = shaped_all(&self.list.borrow());
        let state = if self.ready.get() { "ready" } else { "idle" };
        Value::from_json(&json!({ "state": state, "tasks": tasks }).to_string()).unwrap_or(Value::Nil)
    }

    fn publish(&self) {
        let value = self.value();
        for (_, shared) in self.shared.borrow().iter() {
            shared.set(value.clone());
        }
    }

    fn heard(self: &Rc<Self>, item: Json) {
        *self.list.borrow_mut() = item;
        self.ready.set(true);
        self.publish();
        self.seed();
    }

    fn seed(self: &Rc<Self>) {
        let Some(n) = mock_tasks() else { return };
        if self.seeded.replace(true) || self.list.borrow().as_array().is_some_and(|l| !l.is_empty()) {
            return;
        }
        engine::get().spawn(async move {
            for (i, params) in seed_params(n).into_iter().enumerate() {
                if engine::get().call("Mutate", params).await.is_err() && i == 0 {
                    return;
                }
            }
        });
    }
}

fn mock_tasks() -> Option<usize> {
    if !std::env::var("ZERON_ZGX_MOCK").is_ok_and(|v| v == "1") {
        return None;
    }
    std::env::var("ZERON_ZGX_MOCK_TASKS").ok()?.parse().ok().filter(|n| *n > 0)
}

const SEED_TITLES: [&str; 8] = [
    "Fix the login redirect",
    "Port the sidebar fold",
    "Renew the signing certificate",
    "Draft the onboarding copy",
    "Review the sidebar PR",
    "Answer the design review",
    "Upgrade the build image",
    "Benchmark the parser",
];

/// A mock Dev's tasks: titles in turn, every fifth due yesterday, every
/// seventh done, keys in list order.
fn seed_params(n: usize) -> Vec<Json> {
    let mut out = Vec::new();
    let mut lower: Option<String> = None;
    for i in 0..n {
        let id = format!("mock-task-{i}");
        let key = zeron_proto::pin_order_key_between(lower.as_deref(), None, &format!("s{i}")).ok();
        lower = key.clone();
        let title = if n > SEED_TITLES.len() { format!("{} {}", SEED_TITLES[i % SEED_TITLES.len()], i + 1) } else { SEED_TITLES[i].into() };
        let mut p = json!({ "op": "createTask", "taskId": id, "title": title, "orderKey": key });
        if i % 5 == 2 {
            p["dueOn"] = json!("2000-01-01");
        }
        out.push(p);
        if i % 7 == 5 {
            out.push(json!({ "op": "updateTask", "taskId": id, "patch": { "completed": true } }));
        }
    }
    out
}

async fn mutate(params: Json) -> Result<Value, String> {
    engine::get().call("Mutate", params).await.map(|_| Value::Nil)
}

pub fn install(cx: &mut WindowContext, host: &ScriptHost) {
    let t = TASKS.with(Rc::clone);
    let window = t.next_window.get();
    t.next_window.set(window + 1);
    t.shared.borrow_mut().push((window, host.shared("tasks.list", t.value())));
    let me = Rc::downgrade(&t);
    cx.on_closed(move || {
        if let Some(t) = me.upgrade() {
            t.shared.borrow_mut().retain(|(w, _)| *w != window);
        }
    });
    if !t.watching.replace(true) {
        let weak = Rc::downgrade(&t);
        engine::get().on_connect(move |rpc| {
            let Some(t) = weak.upgrade() else { return };
            let heard = Rc::downgrade(&t);
            *t.watch.borrow_mut() = Some(rpc.watch("WatchTasks", json!({}), move |item| {
                if let Some(t) = heard.upgrade() {
                    t.heard(item);
                }
            }));
        });
    }
    let a = t.clone();
    host.async_action("tasks.create", move |args| {
        let a = a.clone();
        async move {
            crate::plugins::preview::refuse("tasks.create")?;
            let title = args.field("title").map(text).unwrap_or_default();
            if title.trim().is_empty() {
                return Err("empty".to_string());
            }
            let id = crate::sessions::new_id();
            let order = top_order_key(&a.list.borrow(), &id[..12.min(id.len())]);
            mutate(create_params(&args, &id, order)).await?;
            Ok(Value::from(id.as_str()))
        }
    });
    host.async_action("tasks.update", |args| async move {
        crate::plugins::preview::refuse("tasks.update")?;
        let id = args.field("id").map(text).unwrap_or_default();
        mutate(json!({ "op": "updateTask", "taskId": id, "patch": patch_of(&args) })).await
    });
    host.async_action("tasks.delete", |args| async move {
        crate::plugins::preview::refuse("tasks.delete")?;
        let id = args.field("id").map(text).unwrap_or_default();
        mutate(json!({ "op": "deleteTask", "taskId": id })).await
    });
    host.async_action("tasks.assign", |args| async move {
        crate::plugins::preview::refuse("tasks.assign")?;
        let id = args.field("id").map(text).unwrap_or_default();
        let chat = args.field("chat").map(text).unwrap_or_default();
        if chat.is_empty() {
            return mutate(json!({ "op": "updateTask", "taskId": id, "patch": { "chatId": null, "sendError": null } })).await;
        }
        let send = args.field("send").is_some_and(Value::truthy);
        mutate(json!({ "op": "assignTask", "taskId": id, "chatId": chat, "send": send })).await
    });
    host.async_action("tasks.start", |args| async move {
        crate::plugins::preview::refuse("tasks.start")?;
        let id = args.field("id").map(text).unwrap_or_default();
        let picks = args.field("picks").cloned().unwrap_or(Value::Nil);
        let config = crate::chat::draft_config(&picks);
        if config["harness"].is_null() {
            return Err("no agent".to_string());
        }
        let chat = crate::sessions::new_id();
        let mut params = json!({ "op": "startTask", "taskId": id, "chatId": chat, "config": config });
        if let Some(project) = args.field("project").map(text).filter(|s| !s.is_empty()) {
            params["spaceId"] = json!(project);
        }
        mutate(params).await?;
        Ok(Value::from(chat.as_str()))
    });
    host.async_action("tasks.today", |args| async move {
        let since = args.field("since").and_then(Value::as_f64).unwrap_or(0.0) as i64;
        let answer = engine::get().call("GetTodayRuns", json!({ "sinceMs": since })).await?;
        Ok(Value::from_json(&answer.to_string()).unwrap_or(Value::Nil))
    });
    host.async_action("tasks.image", |args| async move {
        let path = args.field("path").map(text).unwrap_or_default();
        crate::files::attachment_here(&path).await
    });
    host.async_action("tasks.upload_image", |args| async move {
        crate::plugins::preview::refuse("tasks.upload_image")?;
        let path = args.field("path").map(text).unwrap_or_default();
        let name = std::path::Path::new(&path).file_name().and_then(|n| n.to_str()).unwrap_or("image").to_string();
        crate::chat::upload(&path, &name, None).await.map(|p| Value::from(p.as_str())).map_err(String::from)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_task_reads_as_the_page_reads_it_and_a_deleted_one_not_at_all() {
        let tasks = json!([
            { "id": "a", "title": "Fix it", "orderKey": "a0", "createdAt": "2026-11-08T10:00:00Z", "completedAt": "2026-11-08T10:00:01.5Z", "spaceId": "p1", "dueOn": "2026-11-08", "chatId": "c1", "images": ["/u/a.png"], "dependsOn": ["b"] },
            { "id": "b", "title": "Gone", "deleted": true },
            { "id": "c", "title": "Plan", "mode": "orchestrated", "sendError": "offline" },
        ]);
        let out = shaped_all(&tasks);
        let list = out.as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["project"], "p1");
        assert_eq!(list[0]["due"], "2026-11-08");
        assert_eq!(list[0]["chat"], "c1");
        assert_eq!(list[0]["created"], 1_794_132_000_000i64);
        assert_eq!(list[0]["completed"], 1_794_132_001_500i64);
        assert_eq!(list[0]["images"], json!(["/u/a.png"]));
        assert_eq!(list[0]["depends"], json!(["b"]));
        assert!(list[1]["completed"].is_null());
        assert_eq!(list[1]["plan"], true);
        assert_eq!(list[1]["send_error"], "offline");
    }

    #[test]
    fn an_update_is_the_engines_patch_and_empty_clears() {
        let args = Value::from_json(r#"{"id":"a","title":"New","due":"","project":"p2","completed":true,"images":["/x.png"]}"#).unwrap();
        let patch: zeron_proto::TaskPatch = serde_json::from_value(patch_of(&args)).unwrap();
        assert_eq!(patch.title.as_deref(), Some("New"));
        assert_eq!(patch.due_on, Some(None));
        assert_eq!(patch.space_id, Some(Some("p2".into())));
        assert_eq!(patch.completed, Some(true));
        assert_eq!(patch.images.as_deref(), Some(&["/x.png".to_string()][..]));
        let only = Value::from_json(r#"{"id":"a","notes":"n"}"#).unwrap();
        assert_eq!(patch_of(&only), json!({ "notes": "n" }));
    }

    #[test]
    fn a_new_task_goes_above_every_open_one() {
        let tasks = json!([
            { "id": "a", "orderKey": "m" },
            { "id": "b", "orderKey": "c" },
            { "id": "d", "orderKey": "a", "completedAt": "2026-11-08T10:00:00Z" },
        ]);
        let key = top_order_key(&tasks, "n1").unwrap();
        assert!(key.as_str() < "c", "{key}");
        let args = Value::from_json(r#"{"title":"  Ship it ","project":"p","due":"2026-11-09","images":["/i.png"]}"#).unwrap();
        let p = create_params(&args, "t1", Some(key.clone()));
        assert_eq!(p["title"], "Ship it");
        assert_eq!(p["spaceId"], "p");
        assert_eq!(p["dueOn"], "2026-11-09");
        assert_eq!(p["orderKey"], json!(key));
        assert_eq!(p["images"], json!(["/i.png"]));
        assert!(create_params(&Value::from_json(r#"{"title":"x"}"#).unwrap(), "t2", None).get("spaceId").is_none());
    }

    #[test]
    fn the_mock_seed_keeps_list_order_and_marks_some_done() {
        let seed = seed_params(20);
        let keys: Vec<&str> = seed.iter().filter(|p| p["op"] == "createTask").map(|p| p["orderKey"].as_str().unwrap()).collect();
        assert_eq!(keys.len(), 20);
        assert!(keys.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(seed.iter().filter(|p| p["op"] == "updateTask").count(), 3);
    }
}
