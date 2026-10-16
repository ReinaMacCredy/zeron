use super::*;
use zeron_proto::{Task, TaskPatch, pin_order_key_between, valid_pin_order_key};

const MAX_TITLE_CHARS: usize = 500;
const MAX_NOTES_CHARS: usize = 10_000;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTask {
    id: String,
    #[serde(default)]
    space_id: Option<String>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    due_on: Option<String>,
    #[serde(default)]
    chat_id: Option<String>,
    #[serde(default)]
    completed_at: Option<i64>,
    #[serde(default)]
    order_key: String,
    #[serde(default)]
    created_at: i64,
    #[serde(default)]
    updated_at: i64,
    #[serde(default)]
    deleted: bool,
}

fn at(ms: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms).unwrap_or(DateTime::UNIX_EPOCH)
}

impl From<RawTask> for Task {
    fn from(raw: RawTask) -> Self {
        Task {
            id: raw.id,
            space_id: raw.space_id,
            title: raw.title,
            notes: raw.notes,
            due_on: raw.due_on,
            chat_id: raw.chat_id,
            completed_at: raw.completed_at.map(at),
            order_key: raw.order_key,
            created_at: at(raw.created_at),
            updated_at: at(raw.updated_at),
            deleted: raw.deleted,
        }
    }
}

fn invalid(message: &str) -> DocError {
    DocError::Schema(message.into())
}

fn checked_title(title: &str) -> Result<String, DocError> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > MAX_TITLE_CHARS {
        return Err(invalid("A task needs a title of up to 500 characters"));
    }
    Ok(title.to_string())
}

fn check_notes(notes: &str) -> Result<(), DocError> {
    if notes.chars().count() > MAX_NOTES_CHARS {
        return Err(invalid("Task notes are limited to 10,000 characters"));
    }
    Ok(())
}

fn check_due_on(due_on: Option<&str>) -> Result<(), DocError> {
    match due_on {
        Some(day)
            if day.len() != 10 || chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").is_err() =>
        {
            Err(invalid("A due date is a YYYY-MM-DD day"))
        }
        _ => Ok(()),
    }
}

fn check_order_key(key: &str) -> Result<(), DocError> {
    if !valid_pin_order_key(key) {
        return Err(invalid("Invalid task order key"));
    }
    Ok(())
}

impl RegistryDoc {
    pub fn task(&self, task_id: &str) -> Option<Task> {
        self.overlay_row(KIND_TASKS, task_id)
            .and_then(|row| row_to::<RawTask>(&row))
            .map(Task::from)
            .filter(|task| !task.deleted)
    }

    pub fn read_tasks(&self) -> Result<Vec<Task>, DocError> {
        let mut tasks: Vec<Task> = self
            .read_kind::<RawTask>(KIND_TASKS)
            .into_iter()
            .map(Task::from)
            .filter(|task| !task.deleted)
            .collect();
        tasks.sort_by(|a, b| {
            (a.order_key.is_empty(), &a.order_key, a.created_at, &a.id).cmp(&(
                b.order_key.is_empty(),
                &b.order_key,
                b.created_at,
                &b.id,
            ))
        });
        Ok(tasks)
    }

    /// A taken id, deleted rows included, is never overwritten: retries must not revive.
    pub fn create_task(&mut self, task: &Task) -> Result<bool, DocError> {
        if task.id.is_empty() || task.id.len() > 256 {
            return Err(invalid("Invalid task ID"));
        }
        let title = checked_title(&task.title)?;
        check_notes(&task.notes)?;
        check_due_on(task.due_on.as_deref())?;
        if self.overlay_row(KIND_TASKS, &task.id).is_some() {
            return Ok(false);
        }
        let hlc = self.next_hlc();
        let order_key = if task.order_key.is_empty() {
            let last = self
                .overlay_rows(KIND_TASKS)
                .into_iter()
                .filter_map(|row| {
                    row.fields
                        .get("orderKey")
                        .and_then(Value::as_str)
                        .filter(|key| valid_pin_order_key(key))
                        .map(str::to_owned)
                })
                .max();
            pin_order_key_between(last.as_deref(), None, &hlc).map_err(invalid)?
        } else {
            check_order_key(&task.order_key)?;
            task.order_key.clone()
        };
        let set = fields([
            ("id", json!(task.id)),
            ("spaceId", opt_str(task.space_id.as_deref())),
            ("title", json!(title)),
            ("notes", json!(task.notes)),
            ("dueOn", opt_str(task.due_on.as_deref())),
            ("chatId", opt_str(task.chat_id.as_deref())),
            ("completedAt", opt_ms(task.completed_at)),
            ("orderKey", json!(order_key)),
            ("createdAt", json!(task.created_at.timestamp_millis())),
            ("updatedAt", json!(task.updated_at.timestamp_millis())),
            ("deleted", json!(false)),
        ]);
        self.enqueue_ops(vec![RowOp {
            kind: KIND_TASKS.into(),
            id: task.id.clone(),
            op: OpKind::Upsert,
            set: Some(set),
            hlc,
            clocks: None,
        }]);
        Ok(true)
    }

    pub fn update_task(
        &mut self,
        task_id: &str,
        patch: &TaskPatch,
        now: DateTime<Utc>,
    ) -> Result<bool, DocError> {
        let Some(task) = self.task(task_id) else {
            return Ok(false);
        };
        let mut set = BTreeMap::new();
        if let Some(title) = &patch.title {
            set.insert("title".into(), json!(checked_title(title)?));
        }
        if let Some(notes) = &patch.notes {
            check_notes(notes)?;
            set.insert("notes".into(), json!(notes));
        }
        if let Some(due_on) = &patch.due_on {
            check_due_on(due_on.as_deref())?;
            set.insert("dueOn".into(), opt_str(due_on.as_deref()));
        }
        if let Some(space_id) = &patch.space_id {
            set.insert("spaceId".into(), opt_str(space_id.as_deref()));
        }
        if let Some(chat_id) = &patch.chat_id {
            set.insert("chatId".into(), opt_str(chat_id.as_deref()));
        }
        if let Some(completed) = patch.completed {
            let completed_at = completed.then(|| task.completed_at.unwrap_or(now));
            set.insert("completedAt".into(), opt_ms(completed_at));
        }
        if let Some(order_key) = &patch.order_key {
            check_order_key(order_key)?;
            set.insert("orderKey".into(), json!(order_key));
        }
        set.insert("updatedAt".into(), json!(now.timestamp_millis()));
        self.write(KIND_TASKS, task_id, OpKind::Update, set);
        Ok(true)
    }

    /// A flagged row, not a tombstone, so a retried create cannot revive it.
    pub fn delete_task(&mut self, task_id: &str, now: DateTime<Utc>) -> Result<bool, DocError> {
        if self.task(task_id).is_none() {
            return Ok(false);
        }
        self.write(
            KIND_TASKS,
            task_id,
            OpKind::Update,
            fields([
                ("deleted", json!(true)),
                ("updatedAt", json!(now.timestamp_millis())),
            ]),
        );
        Ok(true)
    }
}
