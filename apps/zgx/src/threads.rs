use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
};

use serde_json::{Value as Json, json};
use zeron_proto::route_preview;
use zeron_proto::routing::Preview;
use zeron_proto::{Chat, ChatIndicator, Session, TaskStep, ThreadState};
use zgx_desktop::WindowContext;
use zgx_script::{ScriptHost, Shared, Value};

use crate::auth::text;
use crate::engine;

pub(crate) const PLAN_REQUEST: &str = "plan";

#[derive(Default)]
struct Threads {
    steps: RefCell<Vec<TaskStep>>,
    steps_watch: RefCell<Option<engine::rpc::Watch>>,
    watching: Cell<bool>,
    shared: RefCell<Vec<(u64, Shared)>>,
    next_window: Cell<u64>,
    leads: RefCell<HashMap<String, Lead>>,
    previews: RefCell<HashMap<String, (String, Option<Preview>)>>,
    followed: RefCell<HashMap<String, Option<String>>>,
    peers: RefCell<HashMap<String, Vec<String>>>,
}

thread_local! {
    static THREADS: Rc<Threads> = Rc::new(Threads::default());
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Lead {
    has_plan: bool,
    pending: bool,
    overview: String,
    steps: Vec<(String, String)>,
    states: Vec<ThreadState>,
    rows: Vec<Row>,
    unstarted: Option<(String, Vec<&'static str>)>,
}

#[derive(Clone, Debug, PartialEq)]
struct Row {
    key: String,
    kind: &'static str,
    chat: Option<String>,
    step: Option<String>,
    number: Option<usize>,
    title: String,
    step_title: Option<String>,
    state: ThreadState,
    harness: Option<String>,
    model: Option<String>,
    line: Option<String>,
    role: Option<String>,
    spec: Option<String>,
    locked: bool,
    updated: Option<i64>,
}

struct Inputs<'a> {
    chats: &'a Json,
    live: &'a dyn Fn(&str) -> Option<Session>,
    now: chrono::DateTime<chrono::Utc>,
    steps: &'a [TaskStep],
    peers: &'a HashMap<String, Vec<String>>,
    previews: &'a HashMap<String, (String, Option<Preview>)>,
}

fn word(state: ThreadState) -> String {
    serde_json::to_value(state).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

fn first_line(text: Option<&str>) -> Option<String> {
    text?.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_owned)
}

fn thread_row(chat: &Chat, kind: &'static str, state: ThreadState, step: Option<(&TaskStep, usize)>) -> Row {
    let config = chat.config.as_ref();
    let title = chat.title.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned);
    Row {
        key: format!("{kind}:{}", chat.id),
        kind,
        chat: Some(chat.id.clone()),
        step: step.map(|(s, _)| s.id.clone()),
        number: step.map(|(_, n)| n),
        title: title.or_else(|| step.map(|(s, _)| s.title.clone())).unwrap_or_else(|| chat.display_title()),
        step_title: step.map(|(s, _)| s.title.clone()),
        state,
        harness: config.map(|c| c.harness.as_str().to_owned()),
        model: config.and_then(|c| c.model.clone()),
        line: first_line(chat.last_message_preview.as_deref()),
        role: step.and_then(|(s, _)| s.role.clone()),
        spec: step.map(|(s, _)| s.notes.clone()),
        locked: step.is_some(),
        updated: Some(chat.last_message_at.unwrap_or(chat.created_at).timestamp()),
    }
}

fn join(i: &Inputs) -> HashMap<String, Lead> {
    let steps: Vec<&TaskStep> = i.steps.iter().filter(|s| !s.deleted).collect();
    let mut leads: HashSet<&str> = steps.iter().map(|s| s.owner_chat_id.as_str()).collect();
    let mut wanted: HashSet<&str> = steps.iter().filter_map(|s| s.chat_id.as_deref()).collect();
    for (id, peers) in i.peers {
        if !peers.is_empty() {
            leads.insert(id);
            wanted.extend(peers.iter().map(String::as_str));
        }
    }
    let rows = i.chats.as_array().map(Vec::as_slice).unwrap_or_default();
    for c in rows {
        let Some(id) = c["id"].as_str() else { continue };
        if let Some(owner) = c["delegation"]["delegatedByChatId"].as_str() {
            leads.insert(owner);
            wanted.insert(id);
        }
        if c["plan"].is_object() || c["planPending"].as_bool() == Some(true) {
            leads.insert(id);
        }
    }
    wanted.extend(leads.iter().copied());
    let chats: HashMap<&str, Chat> = rows
        .iter()
        .filter_map(|c| {
            let id = c["id"].as_str().filter(|id| wanted.contains(id))?;
            Some((id, serde_json::from_value(c.clone()).ok()?))
        })
        .collect();
    let indicator = |c: &Chat| indicator(c, (i.live)(&c.id).as_ref(), i.now);
    let mut children: HashMap<&str, Vec<&Chat>> = HashMap::new();
    for c in chats.values() {
        if let Some(owner) = c.delegated_by().filter(|_| !c.archived) {
            children.entry(owner).or_default().push(c);
        }
    }
    let mut out = HashMap::new();
    for lead_id in leads {
        let Some(lead) = chats.get(lead_id) else { continue };
        let plan: Vec<&TaskStep> = route_preview::plan_steps(lead_id, i.steps);
        let thread_of = |s: &TaskStep| s.chat_id.as_deref().and_then(|id| chats.get(id));
        let states: Vec<ThreadState> = plan.iter().map(|s| zeron_proto::step_state(s, thread_of(s), thread_of(s).map(indicator))).collect();
        let mut shows: HashMap<&str, usize> = HashMap::new();
        for (ix, s) in plan.iter().enumerate() {
            let Some(t) = thread_of(s) else { continue };
            if shows.get(t.id.as_str()).is_none_or(|&at| plan[at].outcome.is_some()) {
                shows.insert(&t.id, ix);
            }
        }
        let unstarted: Vec<&TaskStep> = plan.iter().copied().filter(|s| thread_of(s).is_none()).collect();
        let key = route_preview::preview_key(&unstarted);
        let preview = i.previews.get(lead_id).filter(|(k, _)| *k == key).and_then(|(_, p)| p.as_ref());
        let lead_target = lead.config.as_ref().map(route_preview::target_of);
        let same_agents = lead.plan.as_ref().is_some_and(|p| p.same_agents);
        let mut targets = match same_agents {
            true => vec![lead_target.clone(); unstarted.len()],
            false => route_preview::step_targets(&unstarted, lead_target.as_ref(), preview),
        }
        .into_iter();
        let mut rows = Vec::new();
        for (ix, s) in plan.iter().enumerate() {
            match thread_of(s) {
                Some(t) if shows.get(t.id.as_str()) == Some(&ix) => rows.push(thread_row(t, "owned", states[ix], Some((s, ix + 1)))),
                Some(_) => {}
                None => {
                    let target = targets.next().flatten();
                    let harness = target.as_ref().map(|t| t.harness.as_str().to_owned());
                    let model = target.and_then(|t| t.model);
                    rows.push(Row {
                        key: format!("step:{}", s.id),
                        kind: "step",
                        chat: None,
                        step: Some(s.id.clone()),
                        number: Some(ix + 1),
                        title: s.title.clone(),
                        step_title: None,
                        state: states[ix],
                        harness,
                        model,
                        line: first_line(Some(&s.notes)),
                        role: s.role.clone(),
                        spec: Some(s.notes.clone()),
                        locked: s.outcome.is_some(),
                        updated: None,
                    });
                }
            }
        }
        let mut own: Vec<&Chat> = children.get(lead_id).into_iter().flatten().copied().filter(|c| !shows.contains_key(c.id.as_str())).collect();
        own.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
        rows.extend(own.into_iter().map(|c| thread_row(c, "owned", zeron_proto::chat_state(c, indicator(c)), None)));
        for peer in i.peers.get(lead_id).into_iter().flatten().filter_map(|id| chats.get(id.as_str())).filter(|c| !c.archived) {
            rows.push(thread_row(peer, "peer", zeron_proto::chat_state(peer, indicator(peer)), None));
        }
        let roles = route_preview::preview_roles(&unstarted);
        out.insert(
            lead_id.to_owned(),
            Lead {
                has_plan: lead.has_plan(),
                pending: lead.plan_pending,
                overview: lead.plan.as_ref().map(|p| p.overview.clone()).unwrap_or_default(),
                steps: plan.iter().map(|s| (s.id.clone(), s.title.clone())).collect(),
                states,
                rows,
                unstarted: (!roles.is_empty() && !same_agents).then_some((key, roles)),
            },
        );
    }
    out
}

fn row_value(r: &Row) -> Value {
    let opt = |v: &Option<String>| v.as_deref().map_or(Value::Nil, Value::from);
    let mark = crate::sessions::brand_mark(r.harness.as_deref(), r.model.as_deref());
    Value::record([
        ("key", Value::from(r.key.as_str())),
        ("kind", Value::from(r.kind)),
        ("read_only", Value::Bool(r.kind == "peer")),
        ("chat", opt(&r.chat)),
        ("step", opt(&r.step)),
        ("number", r.number.map_or(Value::Nil, |n| Value::from(n as f64))),
        ("title", Value::from(r.title.as_str())),
        ("step_title", opt(&r.step_title)),
        ("state", Value::from(word(r.state).as_str())),
        ("harness", opt(&r.harness)),
        ("mark", opt(&mark)),
        ("model", opt(&r.model)),
        ("line", opt(&r.line)),
        ("role", opt(&r.role)),
        ("spec", opt(&r.spec)),
        ("locked", Value::Bool(r.locked)),
        ("updated", r.updated.map_or(Value::Nil, |at| Value::from(at as f64))),
    ])
}

fn lead_value(lead: &Lead) -> Value {
    let plan = if lead.has_plan || !lead.steps.is_empty() {
        let progress = zeron_proto::plan_progress(&lead.states);
        Value::record([
            ("overview", Value::from(lead.overview.as_str())),
            ("pending", Value::Bool(lead.pending)),
            ("order", Value::List(lead.steps.iter().map(|(id, _)| Value::from(id.as_str())).collect())),
            ("progress", Value::record([("done", Value::from(progress.done as f64)), ("total", Value::from(progress.total as f64))])),
        ])
    } else {
        Value::Nil
    };
    Value::record([("plan", plan), ("rows", Value::List(lead.rows.iter().map(row_value).collect()))])
}

fn leads_value(leads: &HashMap<String, Lead>) -> Value {
    let mut ids: Vec<&String> = leads.keys().collect();
    ids.sort();
    Value::record(ids.into_iter().map(|id| (id.as_str(), lead_value(&leads[id]))))
}

pub(crate) fn indicator(chat: &Chat, session: Option<&Session>, now: chrono::DateTime<chrono::Utc>) -> ChatIndicator {
    zeron_proto::view::task_thread_status(zeron_proto::view::display_status(chat, session, now), session, now)
}

pub(crate) fn changed() {
    THREADS.with(|t| t.recompute());
}

pub(crate) fn steps() -> Vec<TaskStep> {
    THREADS.with(|t| t.steps.borrow().clone())
}

pub(crate) fn waiting_plans() -> Vec<(String, Vec<String>)> {
    THREADS.with(|t| {
        let mut out: Vec<(String, Vec<String>)> = t
            .leads
            .borrow()
            .iter()
            .filter(|(_, lead)| lead.pending && !lead.steps.is_empty())
            .map(|(id, lead)| (id.clone(), lead.steps.iter().map(|(_, title)| title.clone()).collect()))
            .collect();
        out.sort();
        out
    })
}

impl Threads {
    fn recompute(self: &Rc<Self>) {
        let leads = {
            let steps = self.steps.borrow();
            let peers = self.peers.borrow();
            let previews = self.previews.borrow();
            crate::sessions::with_chats(|chats, live, now| {
                self.follow_peers(chats);
                join(&Inputs { chats, live, now, steps: &steps, peers: &peers, previews: &previews })
            })
        };
        self.fetch_previews(&leads);
        let value = leads_value(&leads);
        let moved = *self.leads.borrow() != leads;
        *self.leads.borrow_mut() = leads;
        for (_, shared) in self.shared.borrow().iter() {
            shared.set(value.clone());
        }
        crate::tasks::publish();
        if moved {
            crate::chat::publish_waiting();
        }
    }

    fn fetch_previews(self: &Rc<Self>, leads: &HashMap<String, Lead>) {
        for (id, lead) in leads {
            let Some((key, roles)) = &lead.unstarted else { continue };
            if self.previews.borrow().get(id).is_some_and(|(k, _)| k == key) {
                continue;
            }
            self.previews.borrow_mut().insert(id.clone(), (key.clone(), None));
            let (me, id, key) = (self.clone(), id.clone(), key.clone());
            let params = json!({ "ownerChatId": id, "roles": roles, "steps": true });
            engine::get().spawn(async move {
                let Ok(answer) = engine::get().call("PreviewRoutes", params).await else { return };
                let Ok(preview) = serde_json::from_value::<Preview>(answer) else { return };
                let fresh = me.previews.borrow().get(&id).is_some_and(|(k, _)| *k == key);
                if fresh {
                    me.previews.borrow_mut().insert(id, (key, Some(preview)));
                    me.recompute();
                }
            });
        }
    }

    fn follow_peers(self: &Rc<Self>, chats: &Json) {
        let here = crate::sessions::here();
        let mut followed = self.followed.borrow_mut();
        for (id, read_at) in followed.iter_mut() {
            let chat = chats.as_array().into_iter().flatten().find(|c| c["id"].as_str() == Some(id));
            if !chat.is_some_and(|c| here.is_some() && c["deviceId"].as_str() == here.as_deref() && !c["archived"].as_bool().unwrap_or(false)) {
                continue;
            }
            let said = chat.and_then(|c| c["lastMessageAt"].as_str()).map(str::to_owned);
            if *read_at == said && self.peers.borrow().contains_key(id) {
                continue;
            }
            *read_at = said;
            let (me, id) = (self.clone(), id.clone());
            engine::get().spawn(async move {
                let watching: Vec<String> = match engine::get().call("SetThreadWatches", json!({ "chatId": id })).await {
                    Ok(answer) => answer["watching"].as_array().into_iter().flatten().filter_map(Json::as_str).map(str::to_owned).collect(),
                    Err(_) => Vec::new(),
                };
                if !me.followed.borrow().contains_key(&id) {
                    return;
                }
                let moved = me.peers.borrow().get(&id) != Some(&watching);
                me.peers.borrow_mut().insert(id, watching);
                if moved {
                    me.recompute();
                }
            });
        }
    }

    fn follow(self: &Rc<Self>, chats: Vec<String>) {
        self.followed.borrow_mut().retain(|id, _| chats.contains(id));
        self.peers.borrow_mut().retain(|id, _| chats.contains(id));
        for id in chats {
            self.followed.borrow_mut().entry(id).or_insert(None);
        }
        self.recompute();
    }

    fn heard_steps(self: &Rc<Self>, item: Json) {
        *self.steps.borrow_mut() = item.as_array().into_iter().flatten().filter_map(|s| serde_json::from_value(s.clone()).ok()).collect();
        self.recompute();
    }
}

async fn mutate(params: Json) -> Result<Value, String> {
    engine::get().call("Mutate", params).await.map(|_| Value::Nil)
}

pub(crate) async fn approve(chat: String) -> Result<Value, String> {
    mutate(json!({ "op": "approvePlan", "chatId": chat })).await?;
    crate::tasks::approved(&chat).await;
    Ok(Value::Nil)
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.map(Value::list).unwrap_or_default().iter().map(text).filter(|s| !s.is_empty()).collect()
}

fn edit_params(args: &Value) -> Json {
    let mut params = json!({
        "op": "editPlan",
        "chatId": args.field("chat").map(text).unwrap_or_default(),
        "removeStepIds": strings(args.field("remove")),
        "order": strings(args.field("order")),
        "steps": args.field("steps").map(Value::list).unwrap_or_default().iter().map(|s| {
            let mut step = json!({ "title": s.field("title").map(text).unwrap_or_default(), "notes": s.field("spec").map(text).unwrap_or_default() });
            for (key, wire) in [("id", "stepId"), ("role", "role")] {
                if let Some(v) = s.field(key).map(text).filter(|v| !v.is_empty()) {
                    step[wire] = json!(v);
                }
            }
            if s.field("images").is_some() {
                step["images"] = json!(strings(s.field("images")));
            }
            step
        }).collect::<Vec<_>>(),
    });
    if let Some(overview) = args.field("overview") {
        params["overview"] = json!(text(overview));
    }
    if args.field("overview_images").is_some() {
        params["overviewImages"] = json!(strings(args.field("overview_images")));
    }
    params
}

fn mark_params(args: &Value) -> Result<Json, String> {
    let outcome = match args.field("state").map(text).as_deref() {
        Some("done") => json!("done"),
        Some("skipped") => json!("skipped"),
        Some("open") => Json::Null,
        _ => return Err("state must be done, skipped or open".to_string()),
    };
    Ok(json!({ "op": "updateTaskStep", "stepId": args.field("step").map(text).unwrap_or_default(), "patch": { "outcome": outcome } }))
}

pub fn install(cx: &mut WindowContext, host: &ScriptHost) {
    let t = THREADS.with(Rc::clone);
    let window = t.next_window.get();
    t.next_window.set(window + 1);
    t.shared.borrow_mut().push((window, host.shared("chat.threads", leads_value(&t.leads.borrow()))));
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
            t.previews.borrow_mut().clear();
            t.followed.borrow_mut().values_mut().for_each(|at| *at = None);
            let heard = Rc::downgrade(&t);
            *t.steps_watch.borrow_mut() = Some(rpc.watch("WatchTaskSteps", json!({}), move |item| {
                if let Some(t) = heard.upgrade() {
                    t.heard_steps(item);
                }
            }));
        });
    }
    let f = t.clone();
    host.action("threads.follow", move |args| f.follow(strings(args.field("chats"))));
    host.async_action("threads.approve", |args| async move {
        crate::plugins::preview::refuse("threads.approve")?;
        approve(args.field("chat").map(text).unwrap_or_default()).await
    });
    host.async_action("threads.edit", |args| async move {
        crate::plugins::preview::refuse("threads.edit")?;
        mutate(edit_params(&args)).await
    });
    host.async_action("threads.mark", |args| async move {
        crate::plugins::preview::refuse("threads.mark")?;
        mutate(mark_params(&args)?).await
    });
    host.async_action("threads.retry", |args| async move {
        crate::plugins::preview::refuse("threads.retry")?;
        mutate(json!({ "op": "retryStep", "stepId": args.field("step").map(text).unwrap_or_default() })).await
    });
}

#[cfg(test)]
pub(crate) fn join_for_tests(chats: Json, steps: Vec<TaskStep>) {
    THREADS.with(|t| {
        let leads = join(&Inputs {
            chats: &chats,
            live: &|_| None,
            now: chrono::Utc::now(),
            steps: &steps,
            peers: &HashMap::new(),
            previews: &HashMap::new(),
        });
        *t.leads.borrow_mut() = leads;
    });
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn chat(id: &str, extra: Json) -> Json {
        let mut c = json!({
            "id": id, "deviceId": "mac", "archived": false, "createdAt": "2026-11-14T10:00:00Z",
            "config": { "harness": "claude-code", "model": "claude-opus-5-5", "sandbox": "workspace-write" },
        });
        for (k, v) in extra.as_object().into_iter().flatten() {
            c[k] = v.clone();
        }
        c
    }

    pub(crate) fn owned(id: &str, lead: &str, status: &str) -> Json {
        chat(id, json!({
            "title": id,
            "lastMessagePreview": format!("{id} says\nmore"),
            "delegation": { "delegatedByChatId": lead, "depth": 1, "anchor": "m1", "wake": "always", "status": status, "startedAt": "2026-11-14T10:00:00Z" },
        }))
    }

    pub(crate) const TITLES: [&str; 5] = ["Add CSV export", "Add rate limiting", "Fix the flaky test", "Upgrade the date library", "Write the release notes"];

    pub(crate) fn plan(lead: &str, threads: &[Option<&str>]) -> Vec<TaskStep> {
        TITLES
            .iter()
            .enumerate()
            .map(|(i, title)| {
                serde_json::from_value(json!({
                    "id": format!("s{}", i + 1), "ownerChatId": lead, "title": title, "notes": format!("spec of {title}"),
                    "orderKey": format!("a{}", i + 1), "chatId": threads.get(i).copied().flatten(),
                }))
                .unwrap()
            })
            .rev()
            .collect()
    }

    fn inputs<'a>(chats: &'a Json, steps: &'a [TaskStep], peers: &'a HashMap<String, Vec<String>>, previews: &'a HashMap<String, (String, Option<Preview>)>, live: &'a dyn Fn(&str) -> Option<Session>) -> Inputs<'a> {
        Inputs { chats, live, now: chrono::Utc::now(), steps, peers, previews }
    }

    fn joined(chats: Json, steps: &[TaskStep], peers: HashMap<String, Vec<String>>, live: &dyn Fn(&str) -> Option<Session>) -> HashMap<String, Lead> {
        join(&inputs(&chats, steps, &peers, &HashMap::new(), live))
    }

    fn summary(lead: &Lead) -> Vec<(&'static str, Option<usize>, String, ThreadState)> {
        lead.rows.iter().map(|r| (r.kind, r.number, r.title.clone(), r.state)).collect()
    }

    #[test]
    fn a_planning_chat_publishes_its_plan_and_its_steps_in_order_then_four_run_once_approved() {
        let pending = json!([chat("lead", json!({ "title": "Storefront", "plan": { "overview": "Four changes" }, "planPending": true }))]);
        let leads = joined(pending.clone(), &plan("lead", &[]), HashMap::new(), &|_| None);
        let lead = &leads["lead"];
        assert_eq!(summary(lead), TITLES.iter().enumerate().map(|(i, t)| ("step", Some(i + 1), t.to_string(), ThreadState::Planned)).collect::<Vec<_>>());
        assert_eq!(lead.rows[0].spec.as_deref(), Some("spec of Add CSV export"));
        assert_eq!(lead.rows[0].line.as_deref(), Some("spec of Add CSV export"));
        assert_eq!(lead.rows[0].harness.as_deref(), Some("claude-code"), "a step with no role runs on its lead's agent");
        let value = lead_value(lead);
        let plan_of = value.field("plan").unwrap();
        assert_eq!(plan_of.field("overview").map(text).as_deref(), Some("Four changes"));
        assert_eq!(plan_of.field("pending"), Some(&Value::Bool(true)));
        assert_eq!(plan_of.field("order").unwrap().list().iter().map(text).collect::<Vec<_>>(), ["s1", "s2", "s3", "s4", "s5"]);
        assert_eq!(plan_of.field("progress").unwrap().field("total").and_then(Value::as_f64), Some(5.));
        join_for_tests(pending, plan("lead", &[]));
        assert_eq!(waiting_plans(), [("lead".to_string(), TITLES.iter().map(|t| t.to_string()).collect::<Vec<_>>())]);

        let mut running = vec![chat("lead", json!({ "title": "Storefront", "plan": { "overview": "Four changes" } }))];
        running.extend(["k1", "k2", "k3", "k4"].map(|k| owned(k, "lead", "running")));
        let working = |id: &str| (id == "k2").then(|| serde_json::from_value(json!({ "chatId": id, "deviceId": "mac", "status": "working", "updatedAt": chrono::Utc::now() })).unwrap());
        let steps = plan("lead", &[Some("k1"), Some("k2"), Some("k3"), Some("k4")]);
        let leads = joined(Json::Array(running.clone()), &steps, HashMap::new(), &working);
        let lead = &leads["lead"];
        assert_eq!(
            summary(lead),
            [
                ("owned", Some(1), "k1".to_string(), ThreadState::Working),
                ("owned", Some(2), "k2".to_string(), ThreadState::Working),
                ("owned", Some(3), "k3".to_string(), ThreadState::Working),
                ("owned", Some(4), "k4".to_string(), ThreadState::Working),
                ("step", Some(5), TITLES[4].to_string(), ThreadState::Planned),
            ]
        );
        assert_eq!(lead.rows[1].chat.as_deref(), Some("k2"));
        assert_eq!(lead.rows[1].step.as_deref(), Some("s2"));
        assert_eq!(lead.rows[1].line.as_deref(), Some("k2 says"));
        assert!(lead.rows[1].locked && !lead.rows[4].locked);
        join_for_tests(Json::Array(running), steps);
        assert!(waiting_plans().is_empty(), "an approved plan waits on no one");
    }

    #[test]
    fn a_thread_that_ran_two_steps_shows_the_open_one_and_an_unbound_owned_thread_has_its_own_row() {
        let mut chats = vec![chat("lead", json!({ "plan": {} }))];
        chats.push(owned("k1", "lead", "running"));
        chats.push(owned("free", "lead", "completed"));
        let mut settled = owned("done", "lead", "completed");
        settled["lastMessageAt"] = json!("2026-11-14T10:05:00Z");
        settled["settle"] = json!({ "settled": true, "at": "2026-11-14T10:06:00Z" });
        chats.push(settled);
        let mut steps = plan("lead", &[Some("k1"), None, Some("k1")]);
        steps.iter_mut().find(|s| s.id == "s1").unwrap().outcome = Some(zeron_proto::StepOutcome::Done);
        let leads = joined(Json::Array(chats), &steps, HashMap::new(), &|_| None);
        let rows = summary(&leads["lead"]);
        assert_eq!(rows[0], ("step", Some(2), TITLES[1].to_string(), ThreadState::Planned), "step 1 is done on k1, which now runs step 3");
        assert_eq!(rows[1], ("owned", Some(3), "k1".to_string(), ThreadState::Working));
        assert_eq!(&rows[4..], [("owned", None, "done".to_string(), ThreadState::Done), ("owned", None, "free".to_string(), ThreadState::ReadyForReview)]);
        assert_eq!(leads["lead"].states[0], ThreadState::Done);
    }

    #[test]
    fn a_watched_peer_comes_as_a_read_only_row() {
        let chats = json!([chat("lead", json!({})), chat("peer", json!({ "title": "Their thread", "createdByChatId": "elsewhere" })), owned("k1", "lead", "running")]);
        let peers = HashMap::from([("lead".to_string(), vec!["peer".to_string(), "gone".to_string()])]);
        let leads = joined(chats, &[], peers, &|_| None);
        let lead = &leads["lead"];
        assert_eq!(summary(lead), [("owned", None, "k1".to_string(), ThreadState::Working), ("peer", None, "Their thread".to_string(), ThreadState::ReadyForReview)]);
        let read_only: Vec<bool> = lead.rows.iter().map(|r| row_value(r).field("read_only") == Some(&Value::Bool(true))).collect();
        assert_eq!(read_only, [false, true]);
        assert!(lead_value(lead).field("plan") == Some(&Value::Nil), "no plan, no plan bar");
        assert_eq!(row_value(&lead.rows[1]).field("state").map(text).as_deref(), Some("readyForReview"));
    }

    #[test]
    fn an_unstarted_step_shows_the_agent_it_will_run_on() {
        let mut steps = plan("lead", &[]);
        steps.iter_mut().for_each(|s| s.role = Some("validator".into()));
        let preview: Preview = serde_json::from_value(json!({ "rows": [{ "role": "validator", "source": "routed", "target": { "harness": "codex", "model": "gpt-6.1-terra" } }] })).unwrap();
        let key = route_preview::preview_key(&route_preview::plan_steps("lead", &steps));
        let previews = HashMap::from([("lead".to_string(), (key, Some(preview)))]);
        let at = |plan: Json| {
            let chats = json!([chat("lead", json!({ "plan": plan }))]);
            let leads = join(&inputs(&chats, &steps, &HashMap::new(), &previews, &|_| None));
            let row = &leads["lead"].rows[0];
            (row.harness.clone(), row.model.clone(), leads["lead"].unstarted.is_some())
        };
        assert_eq!(at(json!({})), (Some("codex".into()), Some("gpt-6.1-terra".into()), true), "its role's route");
        assert_eq!(at(json!({ "sameAgents": true })), (Some("claude-code".into()), Some("claude-opus-5-5".into()), false), "the lead's own agent, no route asked for");
    }

    #[test]
    fn the_plan_writes_are_the_engines_ops() {
        let args = Value::from_json(r#"{"chat":"lead","overview":"New","steps":[{"id":"s2","title":"Renamed","spec":"why","role":"design"},{"title":"Added"}],"remove":["s4"],"order":["s2","s1"]}"#).unwrap();
        assert_eq!(
            edit_params(&args),
            json!({ "op": "editPlan", "chatId": "lead", "overview": "New", "removeStepIds": ["s4"], "order": ["s2", "s1"],
                "steps": [{ "stepId": "s2", "title": "Renamed", "notes": "why", "role": "design" }, { "title": "Added", "notes": "" }] })
        );
        let mark = |state: &str| mark_params(&Value::from_json(&format!(r#"{{"step":"s1","state":"{state}"}}"#)).unwrap());
        assert_eq!(mark("done").unwrap()["patch"], json!({ "outcome": "done" }));
        assert_eq!(mark("open").unwrap()["patch"], json!({ "outcome": null }));
        assert!(mark("later").is_err());
    }
}
