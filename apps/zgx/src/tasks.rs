//! The engine's tasks (`WatchTasks`, `Mutate` task ops) as `tasks.list` and the `tasks.*` actions ui/screens/tasks reads and calls.

use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    rc::Rc,
};

use serde_json::{Value as Json, json};
use zeron_proto::{Chat, ChatIndicator, NeedsYou, Session, TaskStatus, TaskStep};
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
fn shaped(task: &Json, chats: &[Chat], steps: &[TaskStep], indicator: &dyn Fn(&Chat) -> ChatIndicator) -> Option<Json> {
    let t: zeron_proto::Task = serde_json::from_value(task.clone()).ok()?;
    if t.deleted {
        return None;
    }
    let rollup = zeron_proto::mission::rollup(&t, chats, steps, indicator, |_| ());
    let column = match rollup.status {
        TaskStatus::ToDo => "todo",
        TaskStatus::Working => "working",
        TaskStatus::NeedsYou => "needs",
        TaskStatus::Done => "done",
    };
    let (needs, needs_step) = match rollup.needs {
        Some(NeedsYou::NotSent) => (Some("not_sent"), None),
        Some(NeedsYou::LeadAsks) => (Some("asks"), None),
        Some(NeedsYou::LeadErrored) => (Some("failed"), None),
        Some(NeedsYou::StepAsks(n)) => (Some("step_asks"), Some(n)),
        Some(NeedsYou::StepFailed(n)) => (Some("step_failed"), Some(n)),
        Some(NeedsYou::PlanReady) => (Some("plan_ready"), None),
        Some(NeedsYou::Review) => (Some("review"), None),
        None => (None, None),
    };
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
        "column": column,
        "needs": needs,
        "needs_step": needs_step,
        "progress": (!rollup.steps.is_empty()).then_some(rollup.progress),
        "depends": t.depends_on,
    }))
}

fn shaped_all(tasks: &Json, chats: &Json, steps: &[TaskStep], live: &dyn Fn(&str) -> Option<Session>, now: chrono::DateTime<chrono::Utc>) -> Json {
    let tasks = tasks.as_array().map(Vec::as_slice).unwrap_or_default();
    let linked: HashSet<&str> = tasks.iter().filter_map(|t| t["chatId"].as_str()).collect();
    let mut wanted = linked.clone();
    wanted.extend(steps.iter().filter(|s| linked.contains(s.owner_chat_id.as_str())).filter_map(|s| s.chat_id.as_deref()));
    let chats: Vec<Chat> = chats
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["id"].as_str().is_some_and(|id| wanted.contains(id)))
        .filter_map(|c| serde_json::from_value(c.clone()).ok())
        .collect();
    let indicator = |c: &Chat| crate::threads::indicator(c, live(&c.id).as_ref(), now);
    Json::Array(tasks.iter().filter_map(|t| shaped(t, &chats, steps, &indicator)).collect())
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
        let steps = crate::threads::steps();
        let tasks = crate::sessions::with_chats(|chats, live, now| shaped_all(&self.list.borrow(), chats, &steps, live, now));
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
        if self.seeded.replace(true) {
            return;
        }
        let empty = self.list.borrow().as_array().is_none_or(Vec::is_empty);
        let tasks = mock_tasks().filter(|_| empty);
        let has = |id: &str| self.list.borrow().as_array().into_iter().flatten().any(|t| t["id"] == id);
        let plan = mock_seeds().0 && !has(MOCK_PLAN_TASK);
        let now = mock_seeds().1 && !has(MOCK_NOW_TASK);
        if tasks.is_none() && !plan && !now {
            return;
        }
        engine::get().spawn(async move {
            for (i, params) in seed_params(tasks.unwrap_or(0)).into_iter().enumerate() {
                if engine::get().call("Mutate", params).await.is_err() && i == 0 {
                    return;
                }
            }
            if plan {
                for params in plan_seed_params() {
                    if engine::get().call("Mutate", params).await.is_err() {
                        return;
                    }
                }
            }
            if now {
                let (made, runs) = now_seed_params();
                for params in made {
                    if engine::get().call("Mutate", params).await.is_err() {
                        return;
                    }
                }
                for params in runs {
                    if engine::get().call("DelegateTask", params).await.is_err() {
                        return;
                    }
                }
            }
        });
    }
}

fn mock() -> bool {
    std::env::var("ZERON_ZGX_MOCK").is_ok_and(|v| v == "1")
}

fn mock_seeds() -> (bool, bool) {
    match std::env::var("ZERON_ZGX_MOCK_PLAN").ok().filter(|_| mock()).as_deref() {
        Some("1") => (true, true),
        Some("plan") => (true, false),
        Some("now") => (false, true),
        _ => (false, false),
    }
}

fn mock_tasks() -> Option<usize> {
    if !mock() {
        return None;
    }
    std::env::var("ZERON_ZGX_MOCK_TASKS").ok()?.parse().ok().filter(|n| *n > 0)
}

const MOCK_PLAN_TASK: &str = "mock-plan-task";
const MOCK_PLAN_LEAD: &str = "mock-plan-lead";
const MOCK_PLAN_STEPS: [(&str, &str); 5] = [
    ("Add CSV export to orders", "Mock step: an Export button and a CSV writer on the orders page"),
    ("Add rate limiting to checkout", "Mock step: a token bucket on the checkout API"),
    ("Fix flaky cart total test", "Mock step: find the cause, fix it, 50 green runs"),
    ("Upgrade the date library", "Mock step: move every call site to v4"),
    ("Write the release notes", "Mock step: summarize what shipped"),
];
const MOCK_PLAN_RUNS: usize = 4;

fn plan_seed_params() -> Vec<Json> {
    vec![
        json!({ "op": "createTask", "taskId": MOCK_PLAN_TASK, "title": "Storefront" }),
        json!({ "op": "startTask", "taskId": MOCK_PLAN_TASK, "chatId": MOCK_PLAN_LEAD, "orchestrate": true, "approvePlan": true, "sameAgents": false, "planDoc": false }),
        json!({
            "op": "planSteps",
            "chatId": MOCK_PLAN_LEAD,
            "overview": "Four independent changes run in parallel, each in its own thread. Release notes are written once they land.",
            "steps": MOCK_PLAN_STEPS.iter().map(|(title, notes)| json!({ "title": title, "notes": notes })).collect::<Vec<_>>(),
        }),
    ]
}

const MOCK_NOW_TASK: &str = "mock-now-task";
const MOCK_NOW_LEAD: &str = "mock-now-lead";
const MOCK_NOW_THREADS: [&str; 4] = [
    "Mock step: fix the flaky cart total test",
    "Mock ask: add rate limiting to checkout",
    "Mock done: add CSV export to orders",
    "Mock done: upgrade the date library",
];

fn now_seed_params() -> (Vec<Json>, Vec<Json>) {
    (
        vec![
            json!({ "op": "createTask", "taskId": MOCK_NOW_TASK, "title": "Checkout" }),
            json!({ "op": "startTask", "taskId": MOCK_NOW_TASK, "chatId": MOCK_NOW_LEAD }),
        ],
        MOCK_NOW_THREADS.iter().map(|task| json!({ "ownerChatId": MOCK_NOW_LEAD, "task": task, "harness": "mock" })).collect(),
    )
}

fn mock_runs(chat: &str, steps: &[&TaskStep]) -> Vec<Json> {
    if chat != MOCK_PLAN_LEAD {
        return Vec::new();
    }
    steps
        .iter()
        .filter(|s| s.chat_id.is_none() && s.outcome.is_none())
        .take(MOCK_PLAN_RUNS)
        .map(|s| json!({ "ownerChatId": chat, "task": s.notes, "stepId": s.id, "harness": "mock" }))
        .collect()
}

pub(crate) async fn approved(chat: &str) {
    if !mock_seeds().0 {
        return;
    }
    let steps = crate::threads::steps();
    for params in mock_runs(chat, &zeron_proto::route_preview::plan_steps(chat, &steps)) {
        if engine::get().call("DelegateTask", params).await.is_err() {
            return;
        }
    }
}

pub(crate) fn publish() {
    TASKS.with(|t| t.publish());
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

/// A mock Dev's tasks: titles in turn, every fifth overdue, every seventh done, keys in list order.
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

/// Unlinked as the shipping app's Unassign: the link alone, a send error kept.
fn unassign_params(id: &str) -> Json {
    json!({ "op": "updateTask", "taskId": id, "patch": { "chatId": null } })
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
            return mutate(unassign_params(&id)).await;
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
        let plan = args.field("orchestrate").is_some_and(Value::truthy);
        let mut params = json!({
            "op": "startTask", "taskId": id, "chatId": chat, "config": config,
            "orchestrate": plan, "approvePlan": true, "sameAgents": false, "planDoc": plan,
            "size": crate::chat::plan_size(args.field("size").map(text).as_deref()),
        });
        if let Some(project) = args.field("project").map(text) {
            params["spaceId"] = if project.is_empty() { Json::Null } else { json!(project) };
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
            { "id": "c", "title": "Plan", "chatId": "lead", "sendError": "offline" },
        ]);
        let out = shaped_all(&tasks, &json!([]), &[], &|_| None, chrono::Utc::now());
        let list = out.as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["project"], "p1");
        assert_eq!(list[0]["due"], "2026-11-08");
        assert_eq!(list[0]["chat"], "c1");
        assert_eq!(list[0]["created"], 1_794_132_000_000i64);
        assert_eq!(list[0]["completed"], 1_794_132_001_500i64);
        assert_eq!(list[0]["images"], json!(["/u/a.png"]));
        assert_eq!(list[0]["depends"], json!(["b"]));
        assert_eq!(list[0]["column"], "done");
        assert!(list[1]["completed"].is_null());
        assert_eq!(list[1]["send_error"], "offline");
        assert_eq!((&list[1]["column"], &list[1]["needs"]), (&json!("needs"), &json!("not_sent")));
        assert!(list[1]["progress"].is_null());
        let patch: zeron_proto::TaskPatch = serde_json::from_value(unassign_params("a")["patch"].clone()).unwrap();
        assert_eq!(patch.chat_id, Some(None));
        assert_eq!(patch.send_error, None);
    }

    #[test]
    fn a_linked_to_do_carries_its_chats_plan_progress_and_why_it_needs_you() {
        use crate::threads::tests::{chat, owned, plan};
        let now = chrono::Utc::now();
        let task = |id: &str, chat: &str| json!({ "id": id, "title": id, "chatId": chat, "createdAt": "2026-11-08T10:00:00Z" });
        let tasks = json!([task("pending", "lead"), task("planning", "fresh"), task("asking", "busy"), task("plain", "idle"), task("loose", "gone")]);
        let mut steps = plan("lead", &[]);
        steps.iter_mut().find(|s| s.id == "s1").unwrap().outcome = Some(zeron_proto::StepOutcome::Done);
        steps.extend(plan("busy", &[Some("k1"), Some("k2")]));
        steps.sort_by(|a, b| (&a.owner_chat_id, &a.order_key).cmp(&(&b.owner_chat_id, &b.order_key)));
        let chats = json!([
            chat("lead", json!({ "plan": { "overview": "Five" }, "planPending": true })),
            chat("fresh", json!({ "plan": {}, "planPending": true })),
            chat("busy", json!({ "plan": {} })),
            owned("k1", "busy", "running"),
            owned("k2", "busy", "running"),
            chat("idle", json!({})),
        ]);
        let live = |id: &str| -> Option<Session> {
            (id == "k2").then(|| serde_json::from_value(json!({ "chatId": id, "deviceId": "mac", "status": "awaitingInput", "updatedAt": now })).unwrap())
        };
        let out = shaped_all(&tasks, &chats, &steps, &live, now);
        let read = |at: usize| (out[at]["column"].clone(), out[at]["needs"].clone(), out[at]["needs_step"].clone(), out[at]["progress"].clone());
        assert_eq!(read(0), (json!("needs"), json!("plan_ready"), Json::Null, json!({ "done": 1, "total": 5 })), "Plan ready, 1 of 5");
        assert_eq!(read(1), (json!("working"), Json::Null, Json::Null, Json::Null), "a Plan first chat with no steps yet is planning");
        assert_eq!(read(2), (json!("needs"), json!("step_asks"), json!(2), json!({ "done": 0, "total": 5 })));
        assert_eq!(read(3), (json!("needs"), json!("review"), Json::Null, Json::Null));
        assert_eq!(read(4), (json!("todo"), Json::Null, Json::Null, Json::Null), "a chat gone reads as no link");
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
    fn the_mock_planning_chat_waits_with_five_steps_and_approving_it_starts_four() {
        let seed = plan_seed_params();
        assert_eq!(seed.iter().map(|p| p["op"].as_str().unwrap()).collect::<Vec<_>>(), ["createTask", "startTask", "planSteps"]);
        assert_eq!((seed[1]["chatId"].as_str(), &seed[1]["orchestrate"], &seed[1]["approvePlan"]), (Some(MOCK_PLAN_LEAD), &json!(true), &json!(true)));
        assert_eq!(seed[2]["steps"].as_array().unwrap().len(), 5);
        let mut steps = crate::threads::tests::plan(MOCK_PLAN_LEAD, &[Some("ran")]);
        steps.sort_by(|a, b| a.order_key.cmp(&b.order_key));
        let refs: Vec<&TaskStep> = steps.iter().collect();
        let runs = mock_runs(MOCK_PLAN_LEAD, &refs);
        assert_eq!(runs.iter().map(|r| r["stepId"].as_str().unwrap()).collect::<Vec<_>>(), ["s2", "s3", "s4", "s5"], "the four not started yet, in plan order");
        assert_eq!((runs[0]["ownerChatId"].as_str(), runs[0]["harness"].as_str()), (Some(MOCK_PLAN_LEAD), Some("mock")));
        assert!(mock_runs("another", &refs).is_empty());
        let (made, runs) = now_seed_params();
        assert_eq!(made.iter().map(|p| p["op"].as_str().unwrap()).collect::<Vec<_>>(), ["createTask", "startTask"]);
        assert!(made[1].get("orchestrate").is_none(), "the lead that hands work out at once has no plan");
        assert_eq!(runs.len(), 4);
        assert!(runs.iter().all(|r| r["ownerChatId"] == MOCK_NOW_LEAD && r.get("stepId").is_none()));
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
