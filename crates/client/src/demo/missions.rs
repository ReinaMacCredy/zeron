use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use zeron_doc::RegistryDoc;
use zeron_proto::supervisor::{
    InputAsker, InterventionKind, ItemWhy, NeedsYouInput, NeedsYouRow, NextCheck, NextCheckKind,
    PlanApproval, Raiser, SupervisorEventKind, SupervisorEventSummary, SupervisorIntervention,
    SupervisorItem, SupervisorSettings, SupervisorSnapshot, SupervisorState, SupervisorWake,
    WakeDelivery, is_supervisor_chat,
};
use zeron_proto::{Task, TaskPatch, TaskStep, TaskStepPatch, UserInputQuestion};

use super::fixtures::{MAC, MINI};
use crate::client::ClientInner;
use crate::error::{ClientError, Result};
use crate::lock;

pub(crate) const PLAN_READY: &str = "task-ios-install";
pub(crate) const RUNNING: &str = "task-settle-shelf";
pub(crate) const HELD: &str = "task-widget";
pub(crate) const SHELF_QUESTION: &str = "req-shelf";

fn task(id: &str, title: &str, fields: Value, now: DateTime<Utc>) -> Task {
    let mut value = json!({
        "id": id,
        "title": title,
        "createdAt": now - Duration::hours(6),
        "updatedAt": now,
    });
    if let (Value::Object(base), Value::Object(extra)) = (&mut value, fields) {
        base.extend(extra);
    }
    serde_json::from_value(value).expect("demo task")
}

fn step(
    task_id: &str,
    n: usize,
    title: &str,
    notes: &str,
    chat: Option<&str>,
    done: bool,
) -> TaskStep {
    serde_json::from_value(json!({
        "id": format!("{task_id}-s{n}"),
        "taskId": task_id,
        "title": title,
        "notes": notes,
        "chatId": chat,
        "outcome": done.then_some("done"),
        "createdAt": Utc::now() - Duration::hours(2),
        "updatedAt": Utc::now(),
    }))
    .expect("demo step")
}

pub(crate) fn seed(doc: &mut RegistryDoc, now: DateTime<Utc>) -> Result<(), zeron_doc::DocError> {
    let local = now.with_timezone(&chrono::Local).date_naive();
    let day = |offset: i64| {
        (local + Duration::days(offset))
            .format("%Y-%m-%d")
            .to_string()
    };
    let tasks = [
        task(
            PLAN_READY,
            "Build iOS App Install",
            json!({
                "spaceId": "space-zeron",
                "chatId": "chat-lead-ios",
                "mode": "orchestrated",
                "planPending": true,
                "plan": "## Build iOS App Install\n\nShip an install sheet, a sign-in screen and a TestFlight build, then push, the widget and release notes.",
            }),
            now,
        ),
        task(
            RUNNING,
            "Inbox settle shelf",
            json!({
                "spaceId": "space-fork",
                "chatId": "chat-lead-shelf",
                "mode": "orchestrated",
                "plan": "Settled chats move to a shelf after the host's settle_after_days.",
            }),
            now,
        ),
        task(
            HELD,
            "Needs you widget",
            json!({
                "spaceId": "space-zeron",
                "chatId": "chat-lead-widget",
                "mode": "orchestrated",
                "plan": "## Needs you widget\n\n- A small widget with the **Needs you** count\n- A medium widget listing the first three asks\n- Deep links into the Tasks tab",
            }),
            now,
        ),
        task(
            "task-notes-115",
            "Release notes v0.2.115",
            json!({
                "spaceId": "space-blog",
                "chatId": "chat-blog",
                "mode": "orchestrated",
                "completedAt": now - Duration::minutes(1),
            }),
            now,
        ),
        task(
            "task-signin-fix",
            "Pick the sign-in fix",
            json!({ "spaceId": "space-zeron", "chatId": "chat-picker" }),
            now,
        ),
        task(
            "task-login-loop",
            "Fix login loop",
            json!({ "spaceId": "space-zeron", "chatId": "chat-veil" }),
            now,
        ),
        task(
            "task-whats-new",
            "Write What's New for v0.2.115",
            json!({
                "spaceId": "space-zeron",
                "notes": "Cover the Tasks tab and the merged Needs you sheet.",
                "dueOn": day(0),
            }),
            now,
        ),
        task(
            "task-relay-docs",
            "Update relay docs",
            json!({ "spaceId": "space-edge", "dueOn": day(-1) }),
            now,
        ),
        task("task-usage", "Trim the Usage sheet", json!({}), now),
        task(
            "task-capsule",
            "Capsule overlap",
            json!({ "spaceId": "space-zeron", "completedAt": now - Duration::hours(3) }),
            now,
        ),
        task(
            "task-notes-114",
            "Release notes v0.2.114",
            json!({ "spaceId": "space-zeron", "completedAt": now - Duration::days(1) }),
            now,
        ),
    ];
    for task in &tasks {
        doc.create_task(task)?;
    }
    let steps = [
        step(
            PLAN_READY,
            1,
            "Install sheet",
            "Show the install sheet on first launch.",
            None,
            false,
        ),
        step(
            PLAN_READY,
            2,
            "Sign-in screen",
            "Build the sign-in screen and store the session token.",
            None,
            false,
        ),
        step(
            PLAN_READY,
            3,
            "TestFlight upload",
            "Archive and upload with the team from the project file.",
            None,
            false,
        ),
        step(
            PLAN_READY,
            4,
            "Push notifications",
            "Register for remote notifications.",
            None,
            false,
        ),
        step(
            PLAN_READY,
            5,
            "Widget extension",
            "Add the Needs you widget.",
            None,
            false,
        ),
        step(
            PLAN_READY,
            6,
            "Release notes",
            "Write the TestFlight notes.",
            None,
            false,
        ),
        step(
            RUNNING,
            1,
            "Settled fold",
            "Add the Settled fold under Inbox.",
            Some("chat-shelf-1"),
            true,
        ),
        step(
            RUNNING,
            2,
            "Slim rows",
            "Settled rows drop the preview line.",
            Some("chat-shelf-2"),
            true,
        ),
        step(
            RUNNING,
            3,
            "Swipe Settle and Unsettle",
            "Swipe actions on Inbox rows.",
            Some("chat-shelf-3"),
            false,
        ),
        step(
            RUNNING,
            4,
            "Settle after days",
            "Read settle_after_days from the host.",
            Some("chat-shelf-4"),
            false,
        ),
        step(RUNNING, 5, "Docs", "Update the Inbox article.", None, false),
        step(HELD, 1, "Small widget", "The Needs you count.", None, false),
        step(
            HELD,
            2,
            "Medium widget",
            "The first three asks.",
            None,
            false,
        ),
        step(
            HELD,
            3,
            "Deep links",
            "Open the Tasks tab at the ask.",
            None,
            false,
        ),
    ];
    for mut step in steps {
        if step.task_id == PLAN_READY {
            step.role = Some(plan_role(&step.id).into());
        }
        doc.create_task_step(&step)?;
    }
    Ok(())
}

fn plan_role(step_id: &str) -> &'static str {
    match step_id.rsplit('-').next() {
        Some("s3") => "validator",
        Some("s6") => "general",
        _ => "worker",
    }
}

fn route_preview(params: &Value) -> Value {
    let roles: Vec<&str> = params["roles"]
        .as_array()
        .map(|roles| roles.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let rows: Vec<Value> = roles
        .into_iter()
        .map(|role| match role {
            "worker" => json!({
                "role": role,
                "source": "routed",
                "target": { "harness": "codex", "model": "gpt-5.5" },
                "multiplier": 0.4,
            }),
            "validator" => json!({
                "role": role,
                "source": "routed",
                "target": { "harness": "codex", "model": "gpt-5.5" },
                "multiplier": 0.4,
                "fallback": {
                    "target": { "harness": "claude-code", "model": "claude-sonnet-5" },
                    "reason": "Codex out of quota",
                    "multiplier": 0.6,
                },
            }),
            _ => json!({ "role": role, "source": "off", "multiplier": 1.0 }),
        })
        .collect();
    json!({
        "base": { "harness": "claude-code", "model": "claude-opus-5" },
        "rows": rows,
        "guidance": "",
    })
}

pub(crate) fn shelf_questions() -> Vec<UserInputQuestion> {
    vec![UserInputQuestion {
        id: "q1".into(),
        header: "Settle after".into(),
        question: "Where should the phone read settle_after_days from?".into(),
        options: vec![
            "The host's settings (Recommended)".into(),
            "A phone-only setting".into(),
        ],
        multi_select: false,
        kind: None,
        prefill: None,
        multiline: false,
        approval: false,
    }]
}

fn wake(
    id: &str,
    at: DateTime<Utc>,
    kinds: &[SupervisorEventKind],
    delivery: WakeDelivery,
) -> SupervisorWake {
    SupervisorWake {
        id: id.into(),
        at,
        events: kinds
            .iter()
            .map(|kind| SupervisorEventSummary {
                kind: *kind,
                key: format!("{id}-{kind:?}"),
                task_id: None,
                step_id: None,
                thread_id: None,
                root_chat_id: None,
                label: String::new(),
                at,
            })
            .collect(),
        dropped: Vec::new(),
        delivery,
    }
}

fn item(
    id: &str,
    at: DateTime<Utc>,
    raiser: &str,
    task: &str,
    title: &str,
    why: ItemWhy,
    detail: &str,
    options: &[&str],
) -> NeedsYouRow {
    NeedsYouRow::Item(SupervisorItem {
        id: id.into(),
        at,
        raised_by: Raiser::Lead,
        raiser_chat_id: raiser.into(),
        task_id: Some(task.into()),
        title: title.into(),
        why,
        detail: detail.into(),
        options: options.iter().map(|o| (*o).to_owned()).collect(),
        answer: None,
    })
}

fn supervisor_item(
    id: &str,
    at: DateTime<Utc>,
    device: &str,
    title: &str,
    detail: &str,
    options: &[&str],
) -> NeedsYouRow {
    NeedsYouRow::Item(SupervisorItem {
        id: id.into(),
        at,
        raised_by: Raiser::Supervisor,
        raiser_chat_id: format!("{}{device}", zeron_proto::supervisor::CHAT_PREFIX),
        task_id: None,
        title: title.into(),
        why: ItemWhy::Decision,
        detail: detail.into(),
        options: options.iter().map(|o| (*o).to_owned()).collect(),
        answer: None,
    })
}

const FLAKY_ITEM: &str = "item-settle";

pub(crate) fn supervisors(now: DateTime<Utc>) -> Vec<(&'static str, SupervisorSnapshot)> {
    use SupervisorEventKind::*;
    let mac = SupervisorSnapshot {
        chat_id: Some("supervisor-dev-mac".into()),
        state: SupervisorState::Asleep,
        next_check: Some(NextCheck {
            at: now + Duration::minutes(8),
            kind: NextCheckKind::MissionSilent,
            task_id: Some(PLAN_READY.into()),
        }),
        wakes: vec![
            wake(
                "wake-mac-1",
                now - Duration::minutes(2),
                &[StepFailed, NeedsYou],
                WakeDelivery::Started,
            ),
            wake(
                "wake-mac-2",
                now - Duration::minutes(48),
                &[LeadSilent],
                WakeDelivery::Steered,
            ),
        ],
        watched_roots: vec![
            "chat-lead-ios".into(),
            "chat-lead-widget".into(),
            "chat-picker".into(),
        ],
        needs_you: vec![
            item(
                "item-testflight",
                now - Duration::minutes(3),
                "chat-lead-ios",
                PLAN_READY,
                "Ship to TestFlight before the widget?",
                ItemWhy::Decision,
                "Step 5 can follow in the next build.",
                &["Ship now, widget in the next build", "Wait for the widget"],
            ),
            NeedsYouRow::Input(NeedsYouInput {
                chat_id: "chat-picker".into(),
                request_id: "req-1".into(),
                asker: InputAsker::Thread,
                task_id: None,
                step_id: None,
                at: now - Duration::minutes(2),
                questions: super::transcripts::picker_questions(),
            }),
            supervisor_item(
                "item-worktrees",
                now - Duration::minutes(12),
                MAC,
                "Remove 4 merged worktrees on this Mac?",
                "They hold 3.1 GB and every branch is merged.",
                &["Remove them", "Keep them"],
            ),
        ],
        interventions: vec![SupervisorIntervention {
            id: "iv-approve-widget".into(),
            at: now - Duration::minutes(1),
            kind: InterventionKind::Approve,
            task_id: Some(HELD.into()),
            step_ids: Vec::new(),
            target: None,
            message_id: None,
            request_id: None,
            text: "Approved the plan: three steps, each scoped to one widget size.".into(),
            trigger: None,
            chips: Vec::new(),
            item: None,
            approval: Some(PlanApproval::Held {
                starts_at: now + Duration::minutes(10),
            }),
        }],
        ..Default::default()
    };
    let mini = SupervisorSnapshot {
        chat_id: Some("supervisor-dev-mini".into()),
        state: SupervisorState::Awake,
        wakes: vec![wake(
            "wake-mini-1",
            now - Duration::minutes(6),
            &[StepFailed],
            WakeDelivery::Steered,
        )],
        watched_roots: vec!["chat-lead-shelf".into()],
        needs_you: vec![
            item(
                "item-spend",
                now - Duration::minutes(9),
                "chat-lead-shelf",
                RUNNING,
                "Raise the spend cap to 2x?",
                ItemWhy::Spend,
                "Step 3 retried twice on the swipe animation.",
                &["Raise to 2x", "Keep 1x"],
            ),
            NeedsYouRow::Input(NeedsYouInput {
                chat_id: "chat-shelf-4".into(),
                request_id: SHELF_QUESTION.into(),
                asker: InputAsker::Step,
                task_id: Some(RUNNING.into()),
                step_id: Some(format!("{RUNNING}-s4")),
                at: now - Duration::minutes(4),
                questions: shelf_questions(),
            }),
            supervisor_item(
                FLAKY_ITEM,
                now - Duration::minutes(15),
                MINI,
                "Keep 7 days before settling?",
                "Idle threads move to Settled after this many days.",
                &["Keep 7 days", "Change to 14 days"],
            ),
        ],
        ..Default::default()
    };
    vec![(MAC, mac), (MINI, mini)]
}

pub(crate) fn host_call(
    client: &Arc<ClientInner>,
    settings: &Mutex<HashMap<String, SupervisorSettings>>,
    refused: &Mutex<HashSet<String>>,
    device_id: &str,
    method: &str,
    params: &Value,
) -> Option<Result<Value>> {
    use zeron_rpc::methods as m;
    Some(match method {
        m::MUTATE if params["op"] == "setChatConfig" => return None,
        m::MUTATE => mutate(client, refused, device_id, params).map(|()| json!({})),
        m::PREVIEW_ROUTES => Ok(route_preview(params)),
        m::ENSURE_SUPERVISOR => {
            let chat_id = format!("{}{}", zeron_proto::supervisor::CHAT_PREFIX, device_id);
            client.workspace.update_supervisor(device_id, |snapshot| {
                snapshot.chat_id = Some(chat_id.clone());
                snapshot.blocked = None;
                if snapshot.state == SupervisorState::Off {
                    snapshot.state = SupervisorState::Asleep;
                }
            });
            client.recompute_workspace();
            Ok(json!({ "chatId": chat_id }))
        }
        m::QUEUE_COMMAND if is_supervisor_chat(params["chatId"].as_str().unwrap_or_default()) => {
            let now = Utc::now();
            client.workspace.update_supervisor(device_id, |snapshot| {
                snapshot.state = SupervisorState::Stopped;
                snapshot.wakes.insert(
                    0,
                    wake(
                        &format!("wake-held-{}", now.timestamp_millis()),
                        now,
                        &[SupervisorEventKind::NeedsYou],
                        WakeDelivery::HeldByStop,
                    ),
                );
                snapshot.waiting = snapshot
                    .wakes
                    .iter()
                    .filter(|w| w.delivery == WakeDelivery::HeldByStop)
                    .count();
            });
            client.recompute_workspace();
            Ok(json!({}))
        }
        m::STEER_QUEUED_MESSAGE_NOW
            if is_supervisor_chat(params["chatId"].as_str().unwrap_or_default()) =>
        {
            let id = params["id"].as_str().unwrap_or_default();
            let mut sent = false;
            client.workspace.update_supervisor(device_id, |snapshot| {
                for wake in &mut snapshot.wakes {
                    if wake.id == id && wake.delivery == WakeDelivery::HeldByStop {
                        wake.delivery = WakeDelivery::Started;
                        sent = true;
                    }
                }
                if sent {
                    snapshot.state = SupervisorState::Awake;
                    snapshot.waiting = 0;
                }
            });
            client.recompute_workspace();
            Ok(json!({ "sent": sent }))
        }
        m::GET_SUPERVISOR_SETTINGS => Ok(json!(
            lock(settings).get(device_id).copied().unwrap_or_default()
        )),
        m::SET_SUPERVISOR_SETTINGS => {
            let mut all = lock(settings);
            let current = all.get(device_id).copied().unwrap_or_default();
            let mut merged = json!(current);
            if let (Value::Object(base), Value::Object(patch)) = (&mut merged, params.clone()) {
                base.extend(patch);
            }
            serde_json::from_value::<SupervisorSettings>(merged)
                .map_err(|e| ClientError::HostError(e.to_string()))
                .map(|next| {
                    all.insert(device_id.to_owned(), next);
                    json!(next)
                })
        }
        _ => return None,
    })
}

fn parse<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T> {
    serde_json::from_value(value.clone()).map_err(|e| ClientError::HostError(e.to_string()))
}

fn mutate(
    client: &Arc<ClientInner>,
    refused: &Mutex<HashSet<String>>,
    device_id: &str,
    params: &Value,
) -> Result<()> {
    let now = Utc::now();
    let id = |key: &str| params[key].as_str().unwrap_or_default().to_owned();
    match params["op"].as_str().unwrap_or_default() {
        "approvePlan" => {
            let task_id = id("taskId");
            let patch = TaskPatch {
                plan_pending: Some(false),
                ..Default::default()
            };
            client.registry_write(|doc| doc.update_task(&task_id, &patch, now))?;
            set_approval(client, device_id, &task_id, None);
        }
        "takeBackPlanApproval" => {
            let task_id = id("taskId");
            set_approval(
                client,
                device_id,
                &task_id,
                Some(PlanApproval::TakenBack { at: now }),
            );
        }
        "retryTaskStep" => {
            let step_id = id("stepId");
            let patch: TaskStepPatch = parse(&json!({ "turn": null, "outcome": null }))?;
            client.registry_write(|doc| doc.update_task_step(&step_id, &patch, now))?;
        }
        "updateTaskStep" => {
            let patch: TaskStepPatch = parse(&params["patch"])?;
            let step_id = id("stepId");
            client.registry_write(|doc| doc.update_task_step(&step_id, &patch, now))?;
        }
        "updateTask" => {
            let patch: TaskPatch = parse(&params["patch"])?;
            let task_id = id("taskId");
            client.registry_write(|doc| doc.update_task(&task_id, &patch, now))?;
        }
        "deleteTask" => {
            let task_id = id("taskId");
            client.registry_write(|doc| doc.delete_task(&task_id, now))?;
        }
        "createTask" => {
            let task_id = id("taskId");
            let task = task(
                &task_id,
                params["title"].as_str().unwrap_or_default(),
                json!({
                    "notes": params["notes"].as_str().unwrap_or_default(),
                    "spaceId": params["spaceId"],
                    "dueOn": params["dueOn"],
                    "images": params.get("images").cloned().unwrap_or(json!([])),
                    "context": params.get("context").cloned().unwrap_or(json!([])),
                    "createdAt": now,
                }),
                now,
            );
            client.registry_write(|doc| doc.create_task(&task))?;
        }
        "startTask" => {
            let task_id = id("taskId");
            let chat_id = id("chatId");
            let (state, _) = client.workspace.state();
            let task = state
                .tasks
                .iter()
                .find(|task| task.id == task_id)
                .ok_or_else(|| ClientError::NotFound(task_id.clone()))?;
            let space_id = match params.get("spaceId") {
                Some(Value::String(id)) => Some(id.clone()),
                Some(_) => None,
                None => task.space_id.clone(),
            };
            let config = params
                .get("config")
                .filter(|c| !c.is_null())
                .map(parse)
                .transpose()?;
            let chat = lead_chat(
                &chat_id,
                device_id,
                space_id.clone(),
                &task.title,
                config,
                now,
            );
            let orchestrate = params["orchestrate"].as_bool().unwrap_or(false);
            let mut patch = TaskPatch {
                chat_id: Some(Some(chat_id.clone())),
                ..Default::default()
            };
            if task.space_id.is_none() {
                patch.space_id = Some(space_id);
            }
            if orchestrate {
                patch.mode = Some(zeron_proto::TaskMode::Orchestrated);
                patch.plan_pending = Some(params["approvePlan"].as_bool().unwrap_or(true));
            }
            client.registry_write(|doc| {
                doc.upsert_chat(&chat)?;
                doc.update_task(&task_id, &patch, now)
            })?;
        }
        "assignTask" => {
            let task_id = id("taskId");
            let mut patch = TaskPatch {
                chat_id: Some(Some(id("chatId"))),
                completed: Some(false),
                ..Default::default()
            };
            if params["orchestrate"].as_bool().unwrap_or(false) {
                patch.mode = Some(zeron_proto::TaskMode::Orchestrated);
                patch.plan_pending = Some(params["approvePlan"].as_bool().unwrap_or(true));
            }
            client.registry_write(|doc| doc.update_task(&task_id, &patch, now))?;
        }
        "editTaskPlan" => {
            let task_id = id("taskId");
            let edit: zeron_proto::PlanEdit = parse(params)?;
            let patch = TaskPatch {
                plan: edit.overview.clone(),
                ..Default::default()
            };
            let order_key = |ix: usize| format!("{:04x}1", ix + 1);
            let mut updates: Vec<(String, TaskStepPatch)> = Vec::new();
            for step in &edit.steps {
                if let Some(step_id) = &step.step_id {
                    updates.push((
                        step_id.clone(),
                        TaskStepPatch {
                            title: Some(step.title.clone()),
                            notes: Some(step.notes.clone()),
                            images: Some(step.images.clone()),
                            ..Default::default()
                        },
                    ));
                }
            }
            for (ix, step_id) in edit.order.iter().enumerate() {
                match updates.iter_mut().find(|(id, _)| id == step_id) {
                    Some((_, patch)) => patch.order_key = Some(order_key(ix)),
                    None => updates.push((
                        step_id.clone(),
                        TaskStepPatch {
                            order_key: Some(order_key(ix)),
                            ..Default::default()
                        },
                    )),
                }
            }
            let creates: Vec<TaskStep> = edit
                .steps
                .iter()
                .filter(|step| step.step_id.is_none())
                .map(|step| {
                    serde_json::from_value(json!({
                        "id": crate::new_id(),
                        "taskId": task_id,
                        "title": step.title,
                        "notes": step.notes,
                        "images": step.images,
                        "createdAt": now,
                        "updatedAt": now,
                    }))
                    .expect("demo step")
                })
                .collect();
            client.registry_write(|doc| {
                doc.edit_task_plan(
                    &task_id,
                    &patch,
                    &updates,
                    &creates,
                    &edit.remove_step_ids,
                    now,
                )
            })?;
        }
        "setMissionSupervised" => {
            let task_id = id("taskId");
            let on = params["supervised"].as_bool().unwrap_or(true);
            client.workspace.update_supervisor(device_id, |snapshot| {
                snapshot.unwatched.retain(|id| *id != task_id);
                if !on {
                    snapshot.unwatched.push(task_id.clone());
                }
            });
            client.recompute_workspace();
        }
        "setTreeWatched" => {
            let root = id("rootChatId");
            let on = params["watched"].as_bool().unwrap_or(true);
            client.workspace.update_supervisor(device_id, |snapshot| {
                snapshot.watched_roots.retain(|id| *id != root);
                if on {
                    snapshot.watched_roots.push(root.clone());
                }
            });
            client.recompute_workspace();
        }
        "setWatchEverything" => {
            let on = params["on"].as_bool().unwrap_or(false);
            client.workspace.update_supervisor(device_id, |snapshot| {
                snapshot.watch_everything = on;
            });
            client.recompute_workspace();
        }
        "resolveSupervisorItems" => {
            let answered: Vec<String> = params["answers"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|answer| answer["itemId"].as_str().map(str::to_owned))
                .collect();
            if answered.iter().any(|id| id == FLAKY_ITEM) && lock(refused).insert(FLAKY_ITEM.into())
            {
                return Err(ClientError::HostUnavailable(
                    "Mac mini did not answer in time".into(),
                ));
            }
            client.workspace.update_supervisor(device_id, |snapshot| {
                snapshot.needs_you.retain(
                    |row| !matches!(row, NeedsYouRow::Item(item) if answered.contains(&item.id)),
                );
            });
            client.recompute_workspace();
        }
        other => return Err(ClientError::Unsupported(format!("mutate {other}"))),
    }
    Ok(())
}

fn lead_chat(
    id: &str,
    device_id: &str,
    space_id: Option<String>,
    title: &str,
    config: Option<zeron_proto::ChatConfig>,
    now: DateTime<Utc>,
) -> zeron_proto::Chat {
    zeron_proto::Chat {
        id: id.to_owned(),
        device_id: device_id.to_owned(),
        title: Some(title.to_owned()),
        archived: false,
        cwd: Some("~".to_owned()),
        branch: None,
        checkout_id: None,
        source_context: None,
        config,
        last_message_preview: None,
        last_message_at: None,
        created_at: now,
        harness_session_id: None,
        harness_session_cwd: None,
        harness_session_harness: None,
        space_id,
        last_seen_at: None,
        room_gen: Some(2),
        sidechat_source_chat_id: None,
        created_by_chat_id: None,
        forked_from_chat_id: None,
        previous_harnesses: Vec::new(),
        delegation: None,
        settle: None,
        unsettled_at: None,
    }
}

fn set_approval(
    client: &Arc<ClientInner>,
    device_id: &str,
    task_id: &str,
    approval: Option<PlanApproval>,
) {
    client.workspace.update_supervisor(device_id, |snapshot| {
        for row in &mut snapshot.interventions {
            if row.kind == InterventionKind::Approve && row.task_id.as_deref() == Some(task_id) {
                row.approval = approval.clone();
            }
        }
    });
    client.recompute_workspace();
}

pub(crate) fn input_answered(client: &Arc<ClientInner>, chat_id: &str, request_id: &str) {
    let mut changed = false;
    for device in [MAC, MINI] {
        changed |= client.workspace.update_supervisor(device, |snapshot| {
            snapshot.needs_you.retain(|row| {
                !matches!(row, NeedsYouRow::Input(input) if input.chat_id == chat_id && input.request_id == request_id)
            });
        });
    }
    if changed {
        client.recompute_workspace();
    }
}

pub(crate) fn session_stats(params: &serde_json::Value) -> serde_json::Value {
    let ids = params["chatIds"].as_array().into_iter().flatten();
    let stats = ids.filter_map(serde_json::Value::as_str).map(|id| {
        let seed = id
            .bytes()
            .fold(7u64, |h, b| h.wrapping_mul(31).wrapping_add(u64::from(b)));
        let stats = zeron_proto::SessionStats {
            turns: 2 + seed % 9,
            llm_ms: 40_000 + seed % 500_000,
            tool_ms: 10_000 + seed % 200_000,
            output_tokens: 3_000 + seed % 40_000,
            input_tokens: 8_000 + seed % 30_000,
            cache_read_tokens: 90_000 + seed % 900_000,
            ..Default::default()
        };
        (
            id.to_owned(),
            serde_json::to_value(stats).unwrap_or_default(),
        )
    });
    serde_json::Value::Object(stats.collect())
}
