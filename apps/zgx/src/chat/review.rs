//! gpui's composer review chip, its state kept here as zeron_proto's `ChatReview`: the engine starts the reviewer (StartReview), the chats' watch says when it's done, its findings are read then (TaskStatus), each with how the agent settled it.

use super::*;
use zeron_proto::review::{self, ChatReview, ChipAction, ReviewScope, RunStatus, StartedReview};

thread_local! {
    static REVIEWS: RefCell<HashMap<String, ChatReview>> = RefCell::default();
    static SHARED: RefCell<Option<Shared>> = const { RefCell::new(None) };
    static READING: RefCell<HashSet<String>> = RefCell::default();
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
    let list = chats_now();
    let owner = list.iter().find(|c| c.id == chat).cloned();
    let scope = with(&chat, |r| {
        r.adopt(&list, &chat);
        let scope = scope_of(r, owner.as_ref());
        r.scope.get_or_insert(scope);
        scope
    });
    publish();
    chats_changed();
    load_stats(&chat, scope).await;
    Ok(Value::Nil)
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
    with(chat, |r| r.apply_stats(scope, result));
    publish();
}

async fn start(args: Value) -> Result<Value, String> {
    let chat = args.field("chat").map(text).unwrap_or_default();
    let owner = chats_now().into_iter().find(|c| c.id == chat).ok_or_else(|| "chat not found".to_string())?;
    let (scope, choice, previous) = with(&chat, |r| {
        let scope = scope_of(r, Some(&owner));
        let choice = r.effective_reviewer(None);
        let previous = r.begin(scope, choice.clone());
        (scope, choice, previous)
    });
    publish();
    let commit = with(&chat, |r| r.run.as_ref().and_then(|run| run.commit.clone()));
    let started = engine::get()
        .call("StartReview", review::start_params(&chat, scope, commit.as_deref(), Some(&choice)))
        .await
        .map_err(|e| e.to_string())
        .and_then(|v| {
            let task_id = v["taskId"].as_str().ok_or_else(|| "The reviewer did not start".to_string())?.to_owned();
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

async fn fix(args: Value) -> Result<Value, String> {
    let chat = args.field("chat").map(text).unwrap_or_default();
    let only = args.field("id").map(text).filter(|id| !id.is_empty());
    let (comments, task, hand) = with(&chat, |r| {
        let comments = r.take_fixable(only.as_deref());
        let run = r.run.as_ref();
        (comments, run.and_then(|run| run.task_id.clone()), run.and_then(|run| run.next_hand()))
    });
    publish();
    if let (Some(task), Some((ids, withdraw))) = (task, hand) {
        let sent = engine::get().call("HandReviewFindings", review::hand_params(&chat, &task, &ids, withdraw)).await;
        if sent.is_err() {
            with(&chat, |r| r.run.as_mut().map(|run| run.hand_failed(&ids, withdraw)));
            publish();
        }
    }
    Ok(Value::List(comments.iter().map(|c| Value::record([("path", Value::from(c.path.as_str())), ("line", Value::Num(c.line as f64)), ("body", Value::from(c.body.as_str()))])).collect::<Vec<_>>().into()))
}

/// The chats' list moved: each review follows its reviewer, and one that's done is read.
pub(super) fn chats_changed() {
    if REVIEWS.with(|all| all.borrow().is_empty()) {
        return;
    }
    let list = chats_now();
    let chats = CHATS.with(Rc::clone);
    let mut reads = Vec::new();
    let mut changed = false;
    REVIEWS.with(|all| {
        for (owner, r) in all.borrow_mut().iter_mut() {
            changed |= r.adopt(&list, owner);
            let task = r.run.as_ref().and_then(|run| run.task_id.clone());
            let delegation = task.as_deref().and_then(|t| review::reviewer_child(&list, owner, Some(t))).and_then(|c| c.delegation.clone());
            let working = chats.running(owner);
            if let Some(run) = r.run.as_mut() {
                changed |= run.sync_child(delegation, working);
            }
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
    let value = REVIEWS.with(|all| {
        let all = all.borrow();
        let rows: Vec<(String, Value)> = all.iter().map(|(id, r)| (id.clone(), state_of(r, list.iter().find(|c| &c.id == id)))).collect();
        Value::record(rows.iter().map(|(k, v)| (k.as_str(), v.clone())).collect::<Vec<_>>())
    });
    SHARED.with(|s| {
        if let Some(s) = s.borrow().as_ref() {
            s.set(value);
        }
    });
}

fn state_of(r: &ChatReview, chat: Option<&zeron_proto::Chat>) -> Value {
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
    let run = r.run.as_ref().filter(|run| run.status == RunStatus::Done);
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
                        ("reason", run.reason(&f.id).as_deref().map_or(Value::Nil, Value::from)),
                    ])
                })
                .collect()
        })
        .unwrap_or_default();
    let fixable = run.map_or(0, |run| run.fixable().count());
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
        ("outcomes", r.run.as_ref().filter(|run| run.status == RunStatus::Done).map_or(Value::Nil, |run| Value::from(run.outcome_counts().as_str()))),
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
        let busy = state_of(&r, None);
        assert_eq!(busy.field("label").map(text).as_deref(), Some("Reviewing"));
        assert_eq!(busy.field("busy"), Some(&Value::Bool(true)));
        let summary = "Done.\n\n```zeron-review\n[{\"path\":\"src/a.rs\",\"line\":3,\"severity\":\"bug\",\"title\":\"off by one\",\"body\":\"The loop skips the last row.\"},{\"path\":\"src/b.rs\",\"line\":9,\"severity\":\"nit\",\"title\":\"name\"}]\n```";
        let read = review::read_task_view(&json!({ "status": "completed", "summary": summary }));
        let run = r.run.as_mut().unwrap();
        run.apply_read(read, None, false);
        let mut delegation: zeron_proto::Delegation = serde_json::from_value(json!({
            "delegatedByChatId": "c1", "depth": 1, "anchor": "m1", "wake": "always", "status": "completed", "startedAt": "2026-11-01T10:00:00Z",
            "findingOutcomes": { "F1": { "outcome": "fixed", "commit": "abcdef1234" } }
        }))
        .unwrap_or_else(|e| panic!("delegation: {e}"));
        delegation.client_request_id = Some("zeron-review:uncommitted:n".into());
        run.sync_child(Some(delegation), false);
        let done = state_of(&r, None);
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
    }
}
