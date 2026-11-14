use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use zeron_doc::RegistryDoc;
use zeron_proto::{Task, TaskPatch};

use crate::client::ClientInner;
use crate::error::{ClientError, Result};

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

pub(crate) fn seed(doc: &mut RegistryDoc, now: DateTime<Utc>) -> Result<(), zeron_doc::DocError> {
    let local = now.with_timezone(&chrono::Local).date_naive();
    let day = |offset: i64| {
        (local + Duration::days(offset))
            .format("%Y-%m-%d")
            .to_string()
    };
    let tasks = [
        task(
            "task-ios-install",
            "Build iOS App Install",
            json!({ "spaceId": "space-zeron", "chatId": "chat-lead-ios" }),
            now,
        ),
        task(
            "task-settle-shelf",
            "Inbox settle shelf",
            json!({ "spaceId": "space-fork", "chatId": "chat-lead-shelf" }),
            now,
        ),
        task(
            "task-widget",
            "Needs you widget",
            json!({ "spaceId": "space-zeron", "chatId": "chat-lead-widget" }),
            now,
        ),
        task(
            "task-notes-115",
            "Release notes v0.2.115",
            json!({
                "spaceId": "space-blog",
                "chatId": "chat-blog",
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
    Ok(())
}

pub(crate) fn host_call(
    client: &Arc<ClientInner>,
    device_id: &str,
    method: &str,
    params: &Value,
) -> Option<Result<Value>> {
    (method == zeron_rpc::methods::MUTATE && params["op"] != "setChatConfig")
        .then(|| mutate(client, device_id, params).map(|()| json!({})))
}

fn parse<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T> {
    serde_json::from_value(value.clone()).map_err(|e| ClientError::HostError(e.to_string()))
}

fn mutate(client: &Arc<ClientInner>, device_id: &str, params: &Value) -> Result<()> {
    let now = Utc::now();
    let id = |key: &str| params[key].as_str().unwrap_or_default().to_owned();
    match params["op"].as_str().unwrap_or_default() {
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
                    "dependsOn": params.get("dependsOn").cloned().unwrap_or(json!([])),
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
            let mut patch = TaskPatch {
                chat_id: Some(Some(chat_id.clone())),
                ..Default::default()
            };
            if task.space_id.is_none() {
                patch.space_id = Some(space_id);
            }
            client.registry_write(|doc| {
                doc.upsert_chat(&chat)?;
                doc.update_task(&task_id, &patch, now)
            })?;
        }
        "assignTask" => {
            let task_id = id("taskId");
            let patch = TaskPatch {
                chat_id: Some(Some(id("chatId"))),
                completed: Some(false),
                ..Default::default()
            };
            client.registry_write(|doc| doc.update_task(&task_id, &patch, now))?;
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
        child_kind: None,
        forked_from_chat_id: None,
        previous_harnesses: Vec::new(),
        delegation: None,
        settle: None,
        work_tree: None,
        unsettled_at: None,
        sleep: None,
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
