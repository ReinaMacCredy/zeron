//! The Review built-in's state (the chat header's Review button, the findings block in the transcript), kept here as zeron_proto's `ChatReview`, gpui's review chip's: the engine starts the reviewer (StartReview), the chats' watch says when it's done, its findings are read then (TaskStatus), each with how the agent settled it (the reviewer child's delegation `findingOutcomes`).

use super::*;
use zeron_proto::review::{self, ChatReview, ChipAction, ReviewRun, ReviewScope, RunStatus, StartedReview};

thread_local! {
    static REVIEWS: RefCell<HashMap<String, ChatReview>> = RefCell::default();
    static SHARED: RefCell<Option<Shared>> = const { RefCell::new(None) };
    static READING: RefCell<HashSet<String>> = RefCell::default();
    /// When each chat's review was asked here (Unix seconds), until its reviewer's delegation says.
    static ASKED: RefCell<HashMap<String, f64>> = RefCell::default();
    /// Each chat's block as its transcript last placed it, to place it again only as it moves.
    static PLACED: RefCell<HashMap<String, Option<(String, i64)>>> = RefCell::default();
    /// Each reviewed chat's turn as last seen (status, start): a turn starting or ending counts its changes again.
    static TURNS: RefCell<HashMap<String, String>> = RefCell::default();
    /// Each chat's last finished run while a newer one is busy: its block, outcomes and Fix stay until the new run ends.
    static LAST: RefCell<HashMap<String, ReviewRun>> = RefCell::default();
    /// Who StartReview said reviews, by reviewer task (gpui's `Reviewer::name`), for a child that lists no config.
    static NAMED: RefCell<HashMap<String, String>> = RefCell::default();
    /// Watched chats the chats' list did not hold yet: their scope is chosen, and counted, once it does.
    static UNSEEDED: RefCell<HashSet<String>> = RefCell::default();
    /// Chats whose transcript looked for a reviewer child of theirs (an unwatched chat's block), by how many chats there were then: a new chat, a reviewer an agent started there, looks again.
    static SEEN: RefCell<HashMap<String, usize>> = RefCell::default();
}

pub(super) fn install(host: &ScriptHost) {
    SHARED.with(|s| *s.borrow_mut() = Some(host.shared("review.chats", Value::record(Vec::<(&str, Value)>::new()))));
    host.async_action("review.watch", move |args| async move { watch(args).await });
    host.async_action("review.scope", move |args| async move {
        let chat = args.field("chat").map(text).unwrap_or_default();
        let scope = args.field("scope").map(text).and_then(|key| ReviewScope::from_key(&key)).ok_or_else(|| "unknown review scope".to_string())?;
        with(&chat, |r| r.scope = Some(scope));
        publish();
        load_stats(&chat, scope).await;
        Ok(Value::Nil)
    });
    host.async_action("review.start", move |args| async move {
        crate::plugins::preview::refuse("review.start")?;
        start(args).await
    });
    host.action("review.dismiss", move |args| {
        let chat = args.field("chat").map(text).unwrap_or_default();
        let id = args.field("id").map(text).unwrap_or_default();
        with(&chat, |r| r.dismiss(&id));
        publish();
    });
    host.async_action("review.fix", move |args| async move {
        crate::plugins::preview::refuse("review.fix")?;
        fix(args).await
    });
    host.async_action("review.reviewers", move |args| async move {
        let current = args.field("current").map(crate::plugins::value_json).unwrap_or(Json::Null);
        Ok::<_, String>(reviewers_value(&reviewer_options().await, &current))
    });
    // Settings → Routing's harness picks: the agents offered (ACP agents take no route), each with its reasoning levels.
    host.async_action("routing.harnesses", move |_| async move {
        let options = reviewer_options().await;
        let list = options
            .offered
            .iter()
            .filter(|h| **h != zeron_proto::HarnessId::Acp)
            .map(|h| {
                let id = crate::plugins::json_value(&serde_json::to_value(h).unwrap_or(Json::Null));
                let levels = options.levels.get(h).map(Vec::as_slice).unwrap_or_default();
                Value::record([("id", id), ("label", Value::from(h.display_name())), ("short", Value::from(zeron_proto::routing::short_name(*h))), ("levels", levels_value(levels))])
            })
            .collect::<Vec<_>>();
        Ok::<_, String>(Value::List(list.into()))
    });
    host.async_action("review.models", move |args| async move {
        let harness = args.field("harness").map(text).unwrap_or_default();
        let listed = engine::get().call("ListModels", json!({ "harness": harness, "force": false })).await.map_err(|e| e.to_string())?;
        Ok::<_, String>(models_value(&listed))
    });
}

/// gpui's `reviewer_options`: the agents this Mac offers (as its pickers do) and the saved profiles.
async fn reviewer_options() -> review::ReviewerOptions {
    let mock = std::env::var("ZERON_ZGX_MOCK").is_ok_and(|v| v == "1");
    let listed = engine::get().call("ListHarnesses", json!({ "includeRegistry": true })).await.unwrap_or(Json::Null);
    let profiles = engine::get().call("ListAgentProfiles", json!({})).await.ok();
    options_from(&listed, profiles.as_ref(), mock)
}

fn options_from(listed: &Json, profiles: Option<&Json>, mock: bool) -> review::ReviewerOptions {
    use zeron_proto::HarnessId;
    let mut options = review::ReviewerOptions::default();
    for h in listed.as_array().into_iter().flatten() {
        let Ok(id) = serde_json::from_value::<HarnessId>(h["id"].clone()) else { continue };
        let installed = h["installed"].as_bool() != Some(false);
        let enabled = h["enabled"].as_bool().unwrap_or(installed && id != HarnessId::Mock);
        if installed && (enabled || (mock && id == HarnessId::Mock)) {
            options.offered.push(id);
            options.levels.insert(id, serde_json::from_value(h["reasoningLevels"].clone()).unwrap_or_default());
        }
    }
    options.profiles = profiles
        .and_then(|p| serde_json::from_value::<zeron_proto::AgentProfilesSnapshot>(p.clone()).ok())
        .map(|s| s.profiles)
        .unwrap_or_default();
    options
}

fn levels_value(levels: &[zeron_proto::ReasoningLevel]) -> Value {
    Value::List(
        levels
            .iter()
            .map(|l| Value::record([("value", crate::plugins::json_value(&serde_json::to_value(l).unwrap_or(Json::Null))), ("label", Value::from(l.label()))]))
            .collect::<Vec<_>>()
            .into(),
    )
}

/// Settings → Code review's reviewer list as gpui's page builds it: Auto (naming whom it picks), each agent, each profile, and the saved pick in its own slot when the list lacks it.
fn reviewers_value(options: &review::ReviewerOptions, current: &Json) -> Value {
    use review::ReviewerChoice as C;
    let current: C = serde_json::from_value(current.clone()).unwrap_or_default();
    let same = |c: &C| match (c, &current) {
        (C::Harness { harness: a, .. }, C::Harness { harness: b, .. }) => a == b,
        _ => *c == current,
    };
    let mut choices = options.choices();
    if !choices.iter().any(same) {
        choices.insert(1, current.clone());
    }
    let selected = choices.iter().position(same).unwrap_or(0);
    let auto = review::resolve_reviewer(&C::Auto, None, options).name();
    let list: Vec<Value> = choices
        .iter()
        .map(|c| {
            let detail = match c {
                C::Auto => Value::from(auto.as_str()),
                C::Profile { .. } => Value::from("Profile"),
                C::Harness { .. } => Value::Nil,
            };
            Value::record([
                ("choice", crate::plugins::json_value(&serde_json::to_value(c).unwrap_or(Json::Null))),
                ("label", Value::from(options.label(c).as_str())),
                ("detail", detail),
            ])
        })
        .collect();
    let levels = match &current {
        C::Harness { harness, .. } => options.levels.get(harness).cloned().unwrap_or_default(),
        _ => Vec::new(),
    };
    Value::record([
        ("choices", Value::List(list.into())),
        ("selected", Value::Num((selected + 1) as f64)),
        ("levels", levels_value(&levels)),
    ])
}

fn models_value(listed: &Json) -> Value {
    let models: Vec<zeron_proto::Model> = serde_json::from_value(listed["models"].clone()).unwrap_or_default();
    Value::List(
        models
            .iter()
            .map(|m| Value::record([("id", Value::from(m.id.as_str())), ("label", Value::from(m.label.as_str())), ("levels", levels_value(&m.reasoning_levels))]))
            .collect::<Vec<_>>()
            .into(),
    )
}

fn with<R>(chat: &str, f: impl FnOnce(&mut ChatReview) -> R) -> R {
    REVIEWS.with(|all| f(all.borrow_mut().entry(chat.to_owned()).or_default()))
}

fn chats_now() -> Vec<zeron_proto::Chat> {
    let chats = CHATS.with(Rc::clone);
    let all = chats.chats.borrow();
    all.values().filter_map(|c| serde_json::from_value(c.clone()).ok()).collect()
}

fn scope_of(r: &ChatReview, chat: Option<&zeron_proto::Chat>) -> ReviewScope {
    r.scope.unwrap_or_else(|| ReviewScope::default_for(chat.is_some_and(|c| c.work_tree.is_some())))
}

async fn watch(args: Value) -> Result<Value, String> {
    let chat = args.field("chat").map(text).unwrap_or_default();
    let scope = watch_now(&chat);
    publish();
    chats_changed();
    if let Some(scope) = scope {
        load_stats(&chat, scope).await;
    }
    Ok(Value::Nil)
}

/// A watch's first step: the chat's reviewer adopted, its scope chosen once the chats' list holds it (a worktree chat's is Branch), and its turn as it stands now, so the next turn change counts.
fn watch_now(chat: &str) -> Option<ReviewScope> {
    let list = chats_now();
    let owner = list.iter().find(|c| c.id == chat).cloned();
    let scope = with(chat, |r| {
        keep_last(chat, r, |r| r.adopt(&list, chat));
        let scope = scope_of(r, owner.as_ref());
        if owner.is_none() {
            return None;
        }
        Some(*r.scope.get_or_insert(scope))
    });
    if scope.is_none() {
        UNSEEDED.with(|u| u.borrow_mut().insert(chat.to_owned()));
    }
    let turn = turn_of(chat);
    TURNS.with(|t| {
        t.borrow_mut().entry(chat.to_owned()).or_insert(turn);
    });
    scope
}

/// `f` moves the chat's run on; a finished one it replaces with a busy one stays shown until that ends.
fn keep_last<R>(chat: &str, r: &mut ChatReview, f: impl FnOnce(&mut ChatReview) -> R) -> R {
    let old = r.run.clone().filter(|run| !run.busy());
    let out = f(r);
    if let Some(old) = old.filter(|_| r.run.as_ref().is_some_and(ReviewRun::busy)) {
        LAST.with(|l| l.borrow_mut().insert(chat.to_owned(), old));
    }
    out
}

/// The run the block shows: the chat's own once it has finished, else the last finished one.
fn shown<'a>(r: &'a ChatReview, last: Option<&'a ReviewRun>) -> Option<&'a ReviewRun> {
    match r.run.as_ref() {
        Some(run) if run.busy() => last,
        run => run,
    }
}

/// `f` with the shown run as the chat's run (Fix acts on the block one sees, a new run busy or not).
fn with_shown<R>(chat: &str, f: impl FnOnce(&mut ChatReview) -> R) -> R {
    let last = LAST.with(|l| l.borrow_mut().remove(chat));
    with(chat, |r| match last {
        Some(last) if r.run.as_ref().is_some_and(ReviewRun::busy) => {
            let busy = r.run.replace(last);
            let out = f(r);
            if let Some(last) = std::mem::replace(&mut r.run, busy) {
                LAST.with(|l| l.borrow_mut().insert(chat.to_owned(), last));
            }
            out
        }
        _ => f(r),
    })
}

async fn diff(chat: &zeron_proto::Chat, scope: ReviewScope, commit: Option<&str>) -> Result<(zeron_proto::CheckoutDiff, Option<String>), String> {
    let cwd = chat.cwd.clone().unwrap_or_default();
    let branches: Vec<String> = match scope {
        ReviewScope::Branch => serde_json::from_value(engine::get().call("ListBranches", json!({ "repoPath": cwd })).await.map_err(|e| e.to_string())?).unwrap_or_default(),
        _ => Vec::new(),
    };
    let base = review::diff_base(scope, commit, &branches, chat.branch.as_deref())?;
    let value = engine::get()
        .call("GetCheckoutDiff", review::checkout_diff_params(&cwd, &chat.id, scope, commit, base.as_deref()))
        .await
        .map_err(|e| e.to_string())?;
    let diff = serde_json::from_value(value).map_err(|e| e.to_string())?;
    Ok((diff, base))
}

async fn load_stats(chat: &str, scope: ReviewScope) {
    let Some(owner) = chats_now().into_iter().find(|c| c.id == chat) else { return };
    let commit = with(chat, |r| r.commit.as_ref().map(|c| c.sha.clone()));
    let result = diff(&owner, scope, commit.as_deref()).await;
    apply_stats(chat, scope, result);
    publish();
}

/// The counts, and the shown findings placed again on the new diff: one whose line is gone reads outdated (proto `refresh`).
fn apply_stats(chat: &str, scope: ReviewScope, result: Result<(zeron_proto::CheckoutDiff, Option<String>), String>) {
    let patch = result.as_ref().ok().map(|(d, _)| d.patch.clone());
    with(chat, |r| r.apply_stats(scope, result));
    if let Some(patch) = patch {
        LAST.with(|l| {
            if let Some(last) = l.borrow_mut().get_mut(chat).filter(|run| run.scope == scope && run.status == RunStatus::Done) {
                review::refresh(&mut last.findings, &zeron_proto::patch::parse_patch(&patch));
            }
        });
    }
}

async fn start(args: Value) -> Result<Value, String> {
    let chat = args.field("chat").map(text).unwrap_or_default();
    let owner = chats_now().into_iter().find(|c| c.id == chat).ok_or_else(|| "chat not found".to_string())?;
    let mock = std::env::var("ZERON_ZGX_MOCK").is_ok_and(|v| v == "1");
    let Some((scope, choice, previous)) = begin_start(&chat, &owner, mock) else { return Ok(Value::Nil) };
    publish();
    let commit = with(&chat, |r| r.run.as_ref().and_then(|run| run.commit.clone()));
    let started = engine::get()
        .call("StartReview", review::start_params(&chat, scope, commit.as_deref(), choice.as_ref()))
        .await
        .map_err(|e| e.to_string())
        .and_then(|v| {
            let task_id = v["taskId"].as_str().ok_or_else(|| "The reviewer did not start".to_string())?.to_owned();
            if let Some(name) = v["reviewer"].as_str().filter(|n| !n.is_empty()) {
                NAMED.with(|n| n.borrow_mut().insert(task_id.clone(), name.to_owned()));
            }
            Ok(StartedReview { task_id, patch: v["patch"].as_str().unwrap_or_default().to_owned(), note: v["note"].as_str().map(str::to_owned) })
        });
    with(&chat, |r| {
        if let Some(run) = r.run.as_mut() {
            run.apply_start(started, previous);
        }
    });
    publish();
    chats_changed();
    Ok(Value::Nil)
}

/// A press's first step: none while a run is busy (two presses in one batch start one reviewer); else the run begun, where it was asked noted.
fn begin_start(chat: &str, owner: &zeron_proto::Chat, mock: bool) -> Option<(ReviewScope, Option<review::ReviewerChoice>, Option<ReviewRun>)> {
    let begun = with(chat, |r| {
        if r.run.as_ref().is_some_and(ReviewRun::busy) {
            return None;
        }
        let scope = scope_of(r, Some(owner));
        let choice = start_choice(r, owner.config.as_ref().map(|c| c.harness), mock);
        let named = choice.clone().unwrap_or_else(|| r.effective_reviewer(None));
        let previous = keep_last(chat, r, |r| r.begin(scope, named));
        Some((scope, choice, previous))
    });
    if begun.is_some() {
        ASKED.with(|a| a.borrow_mut().insert(chat.to_owned(), now_seconds() as f64));
    }
    begun
}

/// The reviewer StartReview is asked for: on a mock chat or a mock Dev the mock agent, always (Auto would pick a real agent this machine has); else one picked for this chat, else none, so the engine runs Settings → Code review's (an explicit Auto would override it).
fn start_choice(r: &ChatReview, owner: Option<zeron_proto::HarnessId>, mock: bool) -> Option<review::ReviewerChoice> {
    if mock || owner == Some(zeron_proto::HarnessId::Mock) {
        return Some(review::ReviewerChoice::Harness { harness: zeron_proto::HarnessId::Mock, model: None, reasoning: None });
    }
    r.reviewer.clone()
}

async fn fix(args: Value) -> Result<Value, String> {
    let chat = args.field("chat").map(text).unwrap_or_default();
    let only = args.field("id").map(text).filter(|id| !id.is_empty());
    let (comments, task, hand) = with_shown(&chat, |r| {
        let comments = r.take_fixable(only.as_deref());
        let run = r.run.as_ref();
        (comments, run.and_then(|run| run.task_id.clone()), run.and_then(|run| run.next_hand()))
    });
    publish();
    if let (Some(task), Some((ids, withdraw))) = (task, hand) {
        let sent = engine::get().call("HandReviewFindings", review::hand_params(&chat, &task, &ids, withdraw)).await;
        if sent.is_err() {
            with_shown(&chat, |r| r.run.as_mut().map(|run| run.hand_failed(&ids, withdraw)));
            publish();
        }
    }
    Ok(Value::List(comments.iter().map(|c| Value::record([("path", Value::from(c.path.as_str())), ("line", Value::Num(c.line as f64)), ("body", Value::from(c.body.as_str()))])).collect::<Vec<_>>().into()))
}

/// The chats' list moved: each review follows its reviewer, and one that's done is read.
pub(super) fn chats_changed() {
    let chats = CHATS.with(Rc::clone);
    let shown: Vec<String> = chats.views().iter().filter_map(|v| v.open.borrow().as_ref().map(|o| o.id.clone())).collect();
    let adopted = shown.iter().fold(false, |any, chat| adopt_unwatched(chat) | any);
    if REVIEWS.with(|all| all.borrow().is_empty()) {
        return;
    }
    let list = chats_now();
    let mut reads = Vec::new();
    let mut changed = adopted;
    let delegation = |owner: &str, run: Option<&ReviewRun>| {
        let task = run.and_then(|run| run.task_id.clone());
        task.as_deref().and_then(|t| review::reviewer_child(&list, owner, Some(t))).and_then(|c| c.delegation.clone())
    };
    let seeded: Vec<(String, ReviewScope)> = UNSEEDED.with(|u| {
        let mut u = u.borrow_mut();
        let known: Vec<String> = u.iter().filter(|id| list.iter().any(|c| &c.id == *id)).cloned().collect();
        known
            .into_iter()
            .map(|id| {
                u.remove(&id);
                let owner = list.iter().find(|c| c.id == id);
                let scope = with(&id, |r| *r.scope.get_or_insert(scope_of(r, owner)));
                (id, scope)
            })
            .collect()
    });
    for (chat, scope) in seeded {
        changed = true;
        engine::get().spawn(async move { load_stats(&chat, scope).await });
    }
    REVIEWS.with(|all| {
        for (owner, r) in all.borrow_mut().iter_mut() {
            changed |= keep_last(owner, r, |r| r.adopt(&list, owner));
            let working = chats.running(owner);
            let child = delegation(owner, r.run.as_ref());
            if let Some(run) = r.run.as_mut() {
                changed |= run.sync_child(child, working);
            }
            LAST.with(|l| {
                if let Some(last) = l.borrow_mut().get_mut(owner) {
                    changed |= last.sync_child(delegation(owner, Some(last)), working);
                }
            });
            if let Some(read) = r.finished(&list, owner) {
                reads.push((owner.clone(), read));
            }
        }
    });
    if changed {
        publish();
    }
    for (owner, read) in reads {
        if !READING.with(|r| r.borrow_mut().insert(read.task_id.clone())) {
            continue;
        }
        with(&owner, |r| r.run.as_mut().map(|run| run.status = RunStatus::Reading));
        publish();
        engine::get().spawn(async move {
            let view = engine::get().call("TaskStatus", json!({ "chatId": owner, "taskId": read.task_id })).await;
            let outcome = match view {
                Err(e) => review::ReadOutcome::Failed(e.to_string()),
                Ok(view) => review::read_task_view(&view),
            };
            with(&owner, |r| {
                if let Some(run) = r.run.as_mut().filter(|run| run.task_id.as_deref() == Some(read.task_id.as_str())) {
                    run.apply_read(outcome, read.reviewed.as_deref(), false);
                }
            });
            READING.with(|r| r.borrow_mut().remove(&read.task_id));
            publish();
        });
    }
}

fn publish() {
    let list = chats_now();
    LAST.with(|l| l.borrow_mut().retain(|id, _| REVIEWS.with(|all| all.borrow().get(id).and_then(|r| r.run.as_ref()).is_some_and(ReviewRun::busy))));
    let value = REVIEWS.with(|all| {
        let all = all.borrow();
        let rows: Vec<(String, Value)> = LAST.with(|l| {
            let l = l.borrow();
            all.iter().map(|(id, r)| (id.clone(), state_of(r, l.get(id), list.iter().find(|c| &c.id == id), &list))).collect()
        });
        Value::record(rows.iter().map(|(k, v)| (k.as_str(), v.clone())).collect::<Vec<_>>())
    });
    SHARED.with(|s| {
        if let Some(s) = s.borrow().as_ref() {
            s.set(value);
        }
    });
    let blocks: Vec<(String, Option<(String, i64)>)> =
        REVIEWS.with(|all| LAST.with(|l| all.borrow().iter().map(|(id, r)| (id.clone(), block_of(id, r, l.borrow().get(id)).map(|(row, at)| (row, at as i64)))).collect()));
    let moved: Vec<String> = PLACED.with(|placed| {
        let mut placed = placed.borrow_mut();
        blocks.into_iter().filter(|(id, now)| placed.insert(id.clone(), now.clone()).unwrap_or(None) != *now).map(|(id, _)| id).collect()
    });
    if moved.is_empty() {
        return;
    }
    for view in CHATS.with(Rc::clone).views() {
        let shows = view.open.borrow().as_ref().is_some_and(|open| moved.contains(&open.id));
        if shows {
            view.publish_messages();
        }
    }
}

/// The shown run's block: its row's id, and when it was asked (Unix seconds: its reviewer's delegation's start, else when it was asked here).
fn block_of(chat: &str, r: &ChatReview, last: Option<&ReviewRun>) -> Option<(String, f64)> {
    let run = shown(r, last).filter(|run| !run.busy())?;
    let at = run
        .delegation
        .as_ref()
        .map(|d| d.started_at.timestamp_millis() as f64 / 1000.)
        .or_else(|| ASKED.with(|a| a.borrow().get(chat).copied()))
        .unwrap_or(f64::INFINITY);
    Some((format!("review:{}", run.task_id.as_deref().unwrap_or("start")), at))
}

/// The chat's review block, a row of its transcript where it was asked: after the last message made before then.
pub(super) fn place(chat: &str, list: &mut Vec<Value>) {
    if adopt_unwatched(chat) {
        engine::get().spawn(async { chats_changed() });
    }
    let Some((id, at)) = REVIEWS.with(|all| LAST.with(|l| all.borrow().get(chat).and_then(|r| block_of(chat, r, l.borrow().get(chat))))) else { return };
    place_at(list, Value::record([("id", Value::from(id.as_str())), ("role", Value::from("system")), ("line", Value::from("review")), ("chat", Value::from(chat))]), at);
}

/// A chat shown before it is watched (a split's other pane after a restart, a side chat whose agent asked for a review): once the chats' list is heard, a reviewer child of its is adopted, so its block comes back.
fn adopt_unwatched(chat: &str) -> bool {
    let chats = CHATS.with(Rc::clone);
    let count = chats.chats.borrow().len();
    if !chats.chats_heard.get() || REVIEWS.with(|all| all.borrow().contains_key(chat)) || SEEN.with(|s| s.borrow_mut().insert(chat.to_owned(), count)) == Some(count) {
        return false;
    }
    let list = chats_now();
    review::reviewer_child(&list, chat, None).is_some() && with(chat, |r| r.adopt(&list, chat))
}

fn place_at(list: &mut Vec<Value>, row: Value, at: f64) {
    let time = |r: &Value| r.field("created").or_else(|| r.field("started")).and_then(Value::as_f64);
    let index = if list.iter().any(|r| time(r).is_some()) { list.iter().rposition(|r| time(r).is_some_and(|t| t <= at)).map_or(0, |i| i + 1) } else { list.len() };
    list.insert(index, row);
}

/// The sessions moved: a shown, reviewed chat's turn starting or ending counts its changes again (gpui's `sync_review` stats key), so the button shows once there is something to review.
pub(super) fn sessions_changed() {
    if REVIEWS.with(|all| all.borrow().is_empty()) {
        return;
    }
    let chats = CHATS.with(Rc::clone);
    let shown: Vec<String> = chats.views().iter().filter_map(|v| v.open.borrow().as_ref().map(|o| o.id.clone())).collect();
    let ids: Vec<String> = REVIEWS.with(|all| all.borrow().keys().filter(|id| shown.contains(id)).cloned().collect());
    for chat in ids {
        if !turn_moved(&chat, turn_of(&chat)) {
            continue;
        }
        let owner = chats_now().into_iter().find(|c| c.id == chat);
        let scope = with(&chat, |r| scope_of(r, owner.as_ref()));
        engine::get().spawn(async move { load_stats(&chat, scope).await });
    }
}

/// The chat's turn as it stands (status, start).
fn turn_of(chat: &str) -> String {
    CHATS.with(|c| c.sessions.borrow().get(chat).map(|s| format!("{}|{}", s["status"], s["startedAt"])).unwrap_or_default())
}

/// Whether the chat's turn moved since last seen; it is noted as seen.
fn turn_moved(chat: &str, turn: String) -> bool {
    TURNS.with(|t| t.borrow_mut().insert(chat.to_owned(), turn.clone())).is_some_and(|was| was != turn)
}

/// Who reviewed: the reviewer child's agent and model, else whom StartReview named (gpui's `Reviewer::name`), else the agent or profile picked; never Auto, which names no one.
fn reviewer_name(run: &ReviewRun, chat: Option<&zeron_proto::Chat>, list: &[zeron_proto::Chat]) -> Option<String> {
    let child = chat.and_then(|c| review::reviewer_child(list, &c.id, run.task_id.as_deref())).filter(|_| run.task_id.is_some());
    if let Some(config) = child.and_then(|c| c.config.as_ref()) {
        return Some(match config.model.as_deref().filter(|m| !m.is_empty()) {
            Some(model) => format!("{} · {model}", config.harness.display_name()),
            None => config.harness.display_name().to_string(),
        });
    }
    let named = run.task_id.as_deref().and_then(|t| NAMED.with(|n| n.borrow().get(t).cloned()));
    named.or_else(|| run.choice.as_ref().filter(|c| **c != review::ReviewerChoice::Auto).map(|c| review::ReviewerOptions::default().label(c)))
}

fn outcome_key(state: review::FindingState) -> &'static str {
    match state {
        review::FindingState::Open => "open",
        review::FindingState::AgentOnIt => "agent_on_it",
        review::FindingState::Fixed => "fixed",
        review::FindingState::Skipped => "skipped",
        review::FindingState::NoChange => "no_change",
    }
}

fn state_of(r: &ChatReview, last: Option<&ReviewRun>, chat: Option<&zeron_proto::Chat>, list: &[zeron_proto::Chat]) -> Value {
    let scope = scope_of(r, chat);
    let picked = r.commit.as_ref().map(|c| c.short().to_owned());
    let look = r.look(scope);
    let scopes: Vec<Value> = r
        .scope_rows(chat.and_then(|c| c.branch.as_deref()))
        .into_iter()
        .map(|row| {
            Value::record([
                ("key", Value::from(row.scope.key())),
                ("label", Value::from(row.label.as_str())),
                ("add", row.counts.map_or(Value::Nil, |(a, _)| Value::Num(a as f64))),
                ("del", row.counts.map_or(Value::Nil, |(_, d)| Value::Num(d as f64))),
                ("note", row.note.as_deref().map_or(Value::Nil, Value::from)),
                ("empty", Value::Bool(row.empty)),
                ("picked", Value::Bool(row.scope == scope)),
            ])
        })
        .collect();
    let shown = shown(r, last);
    let run = shown.filter(|run| run.status == RunStatus::Done);
    let findings: Vec<Value> = run
        .map(|run| {
            run.findings
                .iter()
                .map(|f| {
                    Value::record([
                        ("id", Value::from(f.id.as_str())),
                        ("severity", Value::from(f.severity.word())),
                        ("title", Value::from(f.title.as_str())),
                        ("body", Value::from(f.body.as_str())),
                        ("location", Value::from(f.location().as_str())),
                        ("path", Value::from(f.path.as_str())),
                        ("line", Value::Num(f.line as f64)),
                        ("state", Value::from(run.state_label(&f.id).as_str())),
                        ("open", Value::Bool(run.state(&f.id) == review::FindingState::Open)),
                        ("outcome", Value::from(outcome_key(run.state(&f.id)))),
                        ("fixable", Value::Bool(run.fixable().any(|x| x.id == f.id))),
                        ("outdated", Value::Bool(f.placement == review::Placement::Outdated)),
                        ("commit", run.outcome(&f.id).and_then(|o| o.commit.as_deref()).map_or(Value::Nil, |sha| Value::from(review::short_sha(sha)))),
                        ("reason", run.reason(&f.id).as_deref().map_or(Value::Nil, Value::from)),
                    ])
                })
                .collect()
        })
        .unwrap_or_default();
    let fixable = run.map_or(0, |run| run.fixable().count());
    let block = shown.and_then(|run| {
        let (status, message) = match &run.status {
            RunStatus::Done if run.findings.is_empty() => ("clean", None),
            RunStatus::Done => ("done", None),
            RunStatus::Unreadable => ("unreadable", None),
            RunStatus::Stopped => ("stopped", None),
            RunStatus::Failed(message) => ("failed", Some(message.as_str())),
            _ => return None,
        };
        Some(Value::record([("status", Value::from(status)), ("message", message.map_or(Value::Nil, Value::from))]))
    });
    // (Something to review: changes counted in its scope, or a run of its own.)
    let reviewable = matches!(&look, Some(l) if !matches!(l, review::Look::Empty(_)));
    Value::record([
        ("scope", Value::from(scope.key())),
        ("scopes", Value::List(scopes.into())),
        ("start_label", Value::from(r.start_label(scope).as_str())),
        ("label", look.as_ref().map_or(Value::Nil, |l| Value::from(l.label(scope, picked.as_deref()).as_str()))),
        ("tip", look.as_ref().map_or(Value::Nil, |l| Value::from(l.tip(scope).as_str()))),
        (
            "action",
            Value::from(match look.as_ref().map(|l| l.action()) {
                Some(ChipAction::OpenFindings) => "open",
                Some(ChipAction::Start) => "start",
                _ => "inert",
            }),
        ),
        ("busy", Value::Bool(matches!(look, Some(review::Look::Busy)))),
        ("add", look.as_ref().and_then(|l| l.counts()).map_or(Value::Nil, |(a, _)| Value::Num(a as f64))),
        ("del", look.as_ref().and_then(|l| l.counts()).map_or(Value::Nil, |(_, d)| Value::Num(d as f64))),
        ("findings", Value::List(findings.into())),
        ("fix", if fixable > 0 { Value::from(review::fix_remaining(fixable).as_str()) } else { Value::Nil }),
        ("outcomes", run.map_or(Value::Nil, |run| Value::from(run.outcome_counts().as_str()))),
        ("outdated", Value::Num(run.map_or(0, ReviewRun::outdated) as f64)),
        ("reviewable", Value::Bool(reviewable)),
        ("reviewer", shown.and_then(|run| reviewer_name(run, chat, list)).map_or(Value::Nil, |n| Value::from(n.as_str()))),
        ("block", block.unwrap_or(Value::Nil)),
        ("fix_open", Value::Num(fixable as f64)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reviewer_list_is_gpui_s_auto_each_agent_each_profile_and_the_saved_pick() {
        let listed = json!([
            { "id": "claude-code", "installed": true, "enabled": true, "reasoningLevels": ["low", "high"] },
            { "id": "codex", "installed": true, "reasoningLevels": ["medium"] },
            { "id": "cursor", "installed": false },
            { "id": "mock", "installed": true },
        ]);
        let profiles = json!({ "profiles": [{ "id": "p1", "name": "Reviewer", "harness": "codex" }] });
        let options = options_from(&listed, Some(&profiles), false);
        let f = |v: &Value, k: &str| v.field(k).map(text).unwrap_or_default();
        let at = |v: &Value| v.field("selected").and_then(Value::as_f64).unwrap_or(0.0);
        let labels = |v: &Value| -> Vec<String> { v.field("choices").unwrap().list().iter().map(|c| f(c, "label")).collect() };
        let auto = reviewers_value(&options, &Json::Null);
        assert_eq!(labels(&auto), ["Auto", "Claude Code", "Codex", "Reviewer"], "mock only on a mock Dev, an agent not installed never");
        assert_eq!(f(&auto.field("choices").unwrap().list()[0], "detail"), "Reviewer", "Auto names whom it picks: the profile named reviewer");
        assert_eq!(at(&auto), 1.0);
        let picked = reviewers_value(&options, &json!({ "kind": "harness", "harness": "claude-code", "model": "opus" }));
        assert_eq!(at(&picked), 2.0, "the agent's slot, whatever its model");
        let levels: Vec<String> = picked.field("levels").unwrap().list().iter().map(|l| f(l, "label")).collect();
        assert_eq!(levels, ["Low", "High"]);
        let gone = reviewers_value(&options, &json!({ "kind": "profile", "id": "deleted" }));
        assert_eq!((labels(&gone)[1].as_str(), at(&gone)), ("Missing profile", 2.0), "a pick the list lacks keeps a slot of its own");
        assert_eq!(labels(&reviewers_value(&options_from(&listed, None, true), &Json::Null)).len(), 4, "a mock Dev offers the mock agent");
        let models = models_value(&json!({ "models": [{ "id": "gpt", "label": "GPT", "reasoningLevels": ["high"] }] }));
        assert_eq!((f(&models.list()[0], "id"), f(&models.list()[0].field("levels").unwrap().list()[0], "value")), ("gpt".into(), "high".into()));
    }

    #[test]
    fn a_finished_review_reads_as_its_findings_each_with_how_it_was_settled() {
        let mut r = ChatReview { scope: Some(ReviewScope::Uncommitted), ..Default::default() };
        r.apply_stats(ReviewScope::Uncommitted, Err("not loaded".into()));
        let previous = r.begin(ReviewScope::Uncommitted, Default::default());
        r.run.as_mut().unwrap().apply_start(Ok(StartedReview { task_id: "t1".into(), patch: String::new(), note: None }), previous);
        let busy = state_of(&r, None, None, &[]);
        assert_eq!(busy.field("label").map(text).as_deref(), Some("Reviewing"));
        assert_eq!(busy.field("busy"), Some(&Value::Bool(true)));
        let read = review::read_task_view(&json!({ "status": "completed", "summary": SUMMARY }));
        let run = r.run.as_mut().unwrap();
        run.apply_read(read, Some(PATCH), false);
        let mut delegation: zeron_proto::Delegation = serde_json::from_value(json!({
            "delegatedByChatId": "c1", "depth": 1, "anchor": "m1", "wake": "always", "status": "completed", "startedAt": "2026-11-01T10:00:00Z",
            "findingOutcomes": { "F1": { "outcome": "fixed", "commit": "abcdef1234" } }
        }))
        .unwrap_or_else(|e| panic!("delegation: {e}"));
        delegation.client_request_id = Some("zeron-review:uncommitted:n".into());
        run.sync_child(Some(delegation), false);
        let done = state_of(&r, None, None, &[]);
        let findings = match done.field("findings") {
            Some(Value::List(list)) => list.to_vec(),
            other => panic!("findings: {other:?}"),
        };
        assert_eq!(findings.len(), 2, "{done:?}");
        let row = |i: usize, k: &str| findings[i].field(k).map(text).unwrap_or_default();
        assert_eq!((row(0, "severity"), row(0, "title"), row(0, "state")), ("Bug".into(), "off by one".into(), "Fixed · abcdef1".into()));
        assert_eq!((row(1, "severity"), row(1, "state")), ("Nit".into(), "Open".into()));
        assert_eq!(done.field("label").map(text).as_deref(), Some("1 finding"));
        assert_eq!(done.field("action").map(text).as_deref(), Some("open"));
        assert_eq!(done.field("fix").map(text).as_deref(), Some("Fix 1 remaining"));
        assert_eq!((row(0, "outcome"), row(0, "commit"), row(1, "outcome")), ("fixed".into(), "abcdef1".into(), "open".into()), "each finding's outcome from the reviewer's delegation");
        assert_eq!((findings[0].field("fixable"), findings[1].field("fixable")), (Some(&Value::Bool(false)), Some(&Value::Bool(true))));
        assert_eq!(done.field("block").and_then(|b| b.field("status")).map(text).as_deref(), Some("done"));
        assert_eq!(busy.field("block"), Some(&Value::Nil), "no block while the reviewer works: the button says so");
        assert_eq!(done.field("reviewable"), Some(&Value::Bool(true)));
        let comments = r.take_fixable(None);
        assert_eq!(comments.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(), ["src/b.rs"], "Fix all open: the open one only, not the fixed");
        let staged = state_of(&r, None, None, &[]);
        let staged = staged.field("findings").unwrap().list()[1].clone();
        assert_eq!((staged.field("outcome").map(text).as_deref(), staged.field("fixable")), (Some("open"), Some(&Value::Bool(false))), "staged by Fix: still open, not fixable twice");
    }

    const SUMMARY: &str = "Done.\n\n```zeron-review\n[{\"path\":\"src/a.rs\",\"line\":3,\"severity\":\"bug\",\"title\":\"off by one\",\"body\":\"The loop skips the last row.\"},{\"path\":\"src/b.rs\",\"line\":9,\"severity\":\"nit\",\"title\":\"name\"}]\n```";
    const PATCH: &str = "diff --git a/src/b.rs b/src/b.rs\n--- a/src/b.rs\n+++ b/src/b.rs\n@@ -8,2 +8,3 @@\n let a = 1;\n+let name = 2;\n let c = 3;\n";

    fn diff_of(patch: &str, add: u32) -> Result<(zeron_proto::CheckoutDiff, Option<String>), String> {
        let diff = json!({ "checkoutId": "k", "deviceId": "d", "cwd": "/w", "patch": patch, "files": [], "additions": add, "deletions": 0, "truncated": false, "checksum": "", "updatedAt": "2026-11-01T10:00:00Z" });
        Ok((serde_json::from_value(diff).unwrap_or_else(|e| panic!("diff: {e}")), None))
    }

    fn chat_of(id: &str, harness: &str, more: Json) -> zeron_proto::Chat {
        let mut chat = json!({ "id": id, "deviceId": "me", "archived": false, "createdAt": "2026-11-01T09:00:00Z", "config": { "harness": harness, "sandbox": "workspace-write" } });
        for (k, v) in more.as_object().into_iter().flatten() {
            chat[k] = v.clone();
        }
        serde_json::from_value(chat).unwrap_or_else(|e| panic!("chat: {e}"))
    }

    /// A chat whose review landed: two findings, F2 open at src/b.rs:9.
    fn landed(chat: &str, task: &str) {
        with(chat, |r| {
            r.scope = Some(ReviewScope::Uncommitted);
            r.apply_stats(ReviewScope::Uncommitted, diff_of(PATCH, 1));
            let previous = r.begin(ReviewScope::Uncommitted, Default::default());
            let run = r.run.as_mut().unwrap();
            run.apply_start(Ok(StartedReview { task_id: task.into(), patch: PATCH.into(), note: None }), previous);
            run.apply_read(review::read_task_view(&json!({ "status": "completed", "summary": SUMMARY })), Some(PATCH), false);
        });
    }

    fn state_now(chat: &str) -> Value {
        REVIEWS.with(|all| LAST.with(|l| state_of(all.borrow().get(chat).unwrap(), l.borrow().get(chat), None, &[])))
    }

    fn block_now(chat: &str) -> Option<(String, f64)> {
        REVIEWS.with(|all| LAST.with(|l| block_of(chat, all.borrow().get(chat).unwrap(), l.borrow().get(chat))))
    }

    #[test]
    fn a_finding_whose_lines_went_away_says_it_is_outdated_as_gpui_s_meta_does() {
        landed("o", "t1");
        let open = state_now("o");
        let f2 = open.field("findings").unwrap().list()[1].clone();
        assert_eq!((f2.field("outdated"), f2.field("fixable")), (Some(&Value::Bool(false)), Some(&Value::Bool(true))));
        assert_eq!(open.field("outdated").and_then(Value::as_f64), Some(0.));
        apply_stats("o", ReviewScope::Uncommitted, diff_of("", 0));
        let after = state_now("o");
        let f2 = after.field("findings").unwrap().list()[1].clone();
        assert_eq!((f2.field("outdated"), f2.field("fixable"), f2.field("outcome").map(text).as_deref()), (Some(&Value::Bool(true)), Some(&Value::Bool(false)), Some("open")), "committed away: open, no Fix, and it says why");
        assert_eq!(after.field("outdated").and_then(Value::as_f64), Some(1.), "the head counts it, as gpui's pill does");
    }

    #[test]
    fn the_button_s_reviewable_rule_follows_the_counts_and_the_run() {
        let reviewable = |r: &ChatReview| state_of(r, None, None, &[]).field("reviewable").cloned();
        let mut r = ChatReview { scope: Some(ReviewScope::Uncommitted), ..Default::default() };
        assert_eq!(reviewable(&r), Some(Value::Bool(false)), "counts not loaded yet");
        r.apply_stats(ReviewScope::Uncommitted, diff_of("", 0));
        assert_eq!(reviewable(&r), Some(Value::Bool(false)), "no changes: nothing to review, no button");
        r.apply_stats(ReviewScope::Uncommitted, diff_of(PATCH, 1));
        assert_eq!(reviewable(&r), Some(Value::Bool(true)));
        r.begin(ReviewScope::Uncommitted, Default::default());
        r.apply_stats(ReviewScope::Uncommitted, diff_of("", 0));
        assert_eq!(reviewable(&r), Some(Value::Bool(true)), "busy: the button stays, saying Reviewing");
    }

    #[test]
    fn a_new_review_keeps_the_last_block_its_outcomes_and_fix_until_it_ends() {
        landed("n", "t1");
        let owner = chat_of("n", "codex", json!({}));
        assert_eq!(block_now("n").map(|b| b.0).as_deref(), Some("review:t1"));
        let begun = begin_start("n", &owner, false);
        assert!(begun.is_some());
        let busy = state_now("n");
        assert_eq!(busy.field("busy"), Some(&Value::Bool(true)));
        assert_eq!(block_now("n").map(|b| b.0).as_deref(), Some("review:t1"), "the old block stays while the new run is busy");
        assert_eq!(busy.field("block").and_then(|b| b.field("status")).map(text).as_deref(), Some("done"));
        assert_eq!(busy.field("findings").map(|f| f.list().len()), Some(2));
        assert_eq!(busy.field("fix_open").and_then(Value::as_f64), Some(2.), "its Fix all open still there");
        let comments = with_shown("n", |r| r.take_fixable(Some("F2")));
        assert_eq!(comments.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(), ["src/b.rs"], "Fix on the old block stages its finding");
        assert_eq!(state_now("n").field("fix_open").and_then(Value::as_f64), Some(1.));
        with("n", |r| {
            let run = r.run.as_mut().unwrap();
            run.apply_start(Ok(StartedReview { task_id: "t2".into(), patch: PATCH.into(), note: None }), begun.and_then(|b| b.2));
            run.apply_read(review::read_task_view(&json!({ "status": "completed", "summary": "Done.\n\n```zeron-review\n[]\n```" })), Some(PATCH), false);
        });
        publish();
        assert_eq!(block_now("n").map(|b| b.0).as_deref(), Some("review:t2"), "the new run ended: its block replaces the old");
        assert!(LAST.with(|l| l.borrow().get("n").is_none()));
        assert_eq!(state_now("n").field("block").and_then(|b| b.field("status")).map(text).as_deref(), Some("clean"));
    }

    #[test]
    fn a_second_press_while_a_review_is_busy_starts_nothing() {
        let owner = chat_of("p", "codex", json!({}));
        with("p", |r| r.scope = Some(ReviewScope::Uncommitted));
        assert!(begin_start("p", &owner, false).is_some());
        assert!(begin_start("p", &owner, false).is_none(), "one reviewer, however many presses land before the first publish");
    }

    #[test]
    fn a_mock_chat_or_a_mock_dev_reviews_with_the_mock_agent_whatever_settings_say() {
        let mock = Some(review::ReviewerChoice::Harness { harness: zeron_proto::HarnessId::Mock, model: None, reasoning: None });
        let mut r = ChatReview::default();
        assert_eq!(start_choice(&r, Some(zeron_proto::HarnessId::Mock), false), mock, "a mock chat: Auto would pick a real agent this machine has");
        assert_eq!(start_choice(&r, Some(zeron_proto::HarnessId::ClaudeCode), true), mock, "a mock Dev (ZERON_ZGX_MOCK=1)");
        r.reviewer = Some(review::ReviewerChoice::Harness { harness: zeron_proto::HarnessId::ClaudeCode, model: None, reasoning: None });
        assert_eq!(start_choice(&r, Some(zeron_proto::HarnessId::Mock), false), mock, "not even a real agent picked for it");
        let owner = chat_of("m", "mock", json!({}));
        let (scope, choice, _) = begin_start("m", &owner, false).unwrap();
        let params = review::start_params("m", scope, None, choice.as_ref());
        assert_eq!((params["reviewer"]["kind"].as_str(), params["reviewer"]["harness"].as_str()), (Some("harness"), Some("mock")), "{params}");
        assert_eq!(with("m", |r| r.run.as_ref().and_then(|run| run.choice.clone())), mock);
    }

    #[test]
    fn the_head_names_who_reviewed_never_auto() {
        let mut run = review::ReviewRun::new(ReviewScope::Uncommitted);
        run.choice = Some(review::ReviewerChoice::Auto);
        run.task_id = Some("t9".into());
        assert_eq!(reviewer_name(&run, None, &[]), None, "Auto names no one");
        NAMED.with(|n| n.borrow_mut().insert("t9".into(), "Codex".into()));
        assert_eq!(reviewer_name(&run, None, &[]).as_deref(), Some("Codex"), "whom StartReview named");
        run.task_id = None;
        run.choice = Some(review::ReviewerChoice::Harness { harness: zeron_proto::HarnessId::Mock, model: None, reasoning: None });
        assert_eq!(reviewer_name(&run, None, &[]).as_deref(), Some("Mock"));
    }

    #[test]
    fn a_watch_before_the_chats_list_waits_for_it_and_counts_from_the_turn_it_saw() {
        let chats = CHATS.with(Rc::clone);
        chats.sessions.borrow_mut().insert("w".into(), json!({ "chatId": "w", "status": "running", "startedAt": "2026-11-01T10:00:00Z" }));
        assert_eq!(watch_now("w"), None, "no chat yet: no scope fixed at Uncommitted");
        assert_eq!(with("w", |r| r.scope), None);
        chats.sessions.borrow_mut().insert("w".into(), json!({ "chatId": "w", "status": "idle", "startedAt": "2026-11-01T10:00:00Z" }));
        assert!(turn_moved("w", turn_of("w")), "the first turn change after the watch counts");
        chats.chats.borrow_mut().insert("w".into(), serde_json::to_value(chat_of("w", "codex", json!({ "workTree": { "path": "/w/tree", "state": { "kind": "fresh" } } }))).unwrap());
        chats_changed();
        assert_eq!(with("w", |r| r.scope), Some(ReviewScope::Branch), "a worktree chat's scope, once its chat is known");
    }

    #[test]
    fn a_chat_shown_before_it_is_watched_gets_its_review_back() {
        let chats = CHATS.with(Rc::clone);
        let child = chat_of("rv", "mock", json!({ "delegation": { "delegatedByChatId": "u", "depth": 1, "anchor": "m1", "wake": "never", "status": "completed", "startedAt": "2026-11-01T10:00:00Z", "clientRequestId": "zeron-review:uncommitted:n" } }));
        for chat in [chat_of("u", "mock", json!({})), child] {
            chats.chats.borrow_mut().insert(chat.id.clone(), serde_json::to_value(&chat).unwrap());
        }
        assert!(!adopt_unwatched("u"), "not before the chats' list is heard");
        SEEN.with(|s| s.borrow_mut().clear());
        chats.chats_heard.set(true);
        assert!(adopt_unwatched("u"));
        assert_eq!(with("u", |r| r.run.as_ref().and_then(|run| run.task_id.clone())).as_deref(), Some("rv"));
        assert!(!adopt_unwatched("u"), "once");
    }

    #[test]
    fn a_shown_chat_whose_review_starts_later_adopts_it_when_its_reviewer_appears() {
        let chats = CHATS.with(Rc::clone);
        SEEN.with(|s| s.borrow_mut().clear());
        chats.chats_heard.set(true);
        chats.chats.borrow_mut().insert("side".into(), serde_json::to_value(chat_of("side", "mock", json!({}))).unwrap());
        assert!(!adopt_unwatched("side"), "no reviewer yet");
        assert!(!adopt_unwatched("side"), "nothing new: not looked at again");
        let child = chat_of("rv2", "mock", json!({ "delegation": { "delegatedByChatId": "side", "depth": 1, "anchor": "m1", "wake": "never", "status": "completed", "startedAt": "2026-11-01T10:00:00Z", "clientRequestId": "zeron-review:uncommitted:n" } }));
        chats.chats.borrow_mut().insert(child.id.clone(), serde_json::to_value(&child).unwrap());
        assert!(adopt_unwatched("side"), "its reviewer, started from elsewhere, is adopted");
        assert_eq!(with("side", |r| r.run.as_ref().and_then(|run| run.task_id.clone())).as_deref(), Some("rv2"));
    }

    #[test]
    fn the_review_runs_the_reviewer_settings_chose_unless_this_chat_picked_one() {
        let mut r = ChatReview::default();
        assert_eq!(start_choice(&r, Some(zeron_proto::HarnessId::ClaudeCode), false), None, "none sent: the engine runs Settings → Code review's reviewer (an explicit Auto would override it)");
        r.reviewer = Some(review::ReviewerChoice::Harness { harness: zeron_proto::HarnessId::Codex, model: None, reasoning: None });
        assert_eq!(start_choice(&r, Some(zeron_proto::HarnessId::ClaudeCode), false), r.reviewer);
    }

    #[test]
    fn the_findings_block_stands_in_the_transcript_where_the_review_was_asked() {
        let row = |id: &str, k: &str, t: f64| Value::record([("id", Value::from(id)), (k, Value::Num(t))]);
        let ids = |list: &[Value]| list.iter().map(|r| r.field("id").map(text).unwrap_or_default()).collect::<Vec<_>>();
        let block = || Value::record([("id", Value::from("review:t1"))]);
        let seam = Value::record([("id", Value::from("seam")), ("role", Value::from("switch"))]);
        let mut list = vec![row("u1", "created", 10.), row("a1", "started", 11.), seam.clone(), row("u2", "created", 30.), row("a2", "started", 31.)];
        place_at(&mut list, block(), 20.);
        assert_eq!(ids(&list), ["u1", "a1", "review:t1", "seam", "u2", "a2"], "after the last message made before it, the later turn under it");
        let mut list = vec![row("u1", "created", 10.), row("a1", "started", 11.)];
        place_at(&mut list, block(), 50.);
        assert_eq!(ids(&list), ["u1", "a1", "review:t1"]);
        let mut list = vec![row("u1", "created", 10.)];
        place_at(&mut list, block(), 5.);
        assert_eq!(ids(&list), ["review:t1", "u1"]);
        let mut list = vec![seam];
        place_at(&mut list, block(), 5.);
        assert_eq!(ids(&list), ["seam", "review:t1"], "no times at all: at the end");
        let mut r = ChatReview::default();
        assert!(block_of("c", &r, None).is_none());
        r.run = Some(review::ReviewRun::new(ReviewScope::Uncommitted));
        assert!(block_of("c", &r, None).is_none(), "starting, with no block before: none yet");
        r.run.as_mut().unwrap().status = RunStatus::Failed("Nothing to review".into());
        ASKED.with(|a| a.borrow_mut().insert("c".into(), 42.));
        assert_eq!(block_of("c", &r, None), Some(("review:start".into(), 42.)), "a start that failed says so where it was asked");
    }
}
