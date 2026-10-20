use super::*;
use zeron_proto::{Task, TaskMode, TaskPatch, pin_order_key_between, valid_pin_order_key};

pub const TASK_TITLE_MAX_CHARS: usize = 500;
pub const TASK_NOTES_MAX_CHARS: usize = 10_000;
/// As long as the uuid a device mints, for clocks a row has not been written with yet.
const CLOCK_DEVICE: &str = "00000000-0000-0000-0000-000000000000";

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
    #[serde(default)]
    mode: TaskMode,
    #[serde(default)]
    plan_pending: bool,
    #[serde(default)]
    send_error: Option<String>,
    #[serde(default)]
    plan: String,
    #[serde(default)]
    plan_doc: bool,
    #[serde(default)]
    resources: Vec<String>,
    #[serde(default)]
    images: Vec<String>,
    #[serde(default)]
    context: Vec<String>,
}

pub(super) fn at(ms: i64) -> DateTime<Utc> {
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
            mode: raw.mode,
            plan_pending: raw.plan_pending,
            send_error: raw.send_error,
            plan: raw.plan,
            plan_doc: raw.plan_doc,
            resources: raw.resources,
            images: raw.images,
            context: raw.context,
        }
    }
}

pub(super) fn invalid(message: &str) -> DocError {
    DocError::Schema(message.into())
}

pub(super) fn checked_title(title: &str) -> Result<String, DocError> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > TASK_TITLE_MAX_CHARS {
        return Err(invalid("A task needs a title of up to 500 characters"));
    }
    Ok(title.to_string())
}

pub(super) fn check_notes(notes: &str) -> Result<(), DocError> {
    if notes.chars().count() > TASK_NOTES_MAX_CHARS {
        return Err(invalid("Task notes are limited to 10,000 characters"));
    }
    Ok(())
}

pub const TASK_IMAGES_MAX: usize = 12;

pub fn check_images(images: &[String]) -> Result<(), DocError> {
    if images.len() > TASK_IMAGES_MAX {
        return Err(invalid("A task or step holds up to 12 images"));
    }
    if images
        .iter()
        .any(|path| path.trim().is_empty() || path.chars().any(char::is_control))
    {
        return Err(invalid("An image is a single-line file path"));
    }
    Ok(())
}

pub const TASK_STEPS_MAX: usize = 50;

pub(super) fn check_step_count(count: usize) -> Result<(), DocError> {
    if count > TASK_STEPS_MAX {
        return Err(invalid("A plan holds up to 50 steps"));
    }
    Ok(())
}

pub const TASK_CONTEXT_MAX: usize = 5;

pub fn check_context(context: &[String]) -> Result<(), DocError> {
    if context.len() > TASK_CONTEXT_MAX {
        return Err(invalid("A mission reads up to 5 chats"));
    }
    if context
        .iter()
        .any(|id| !valid_row_id(id) || context.iter().filter(|other| *other == id).count() > 1)
    {
        return Err(invalid("Context chats are distinct chat ids"));
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

pub(super) fn check_order_key(key: &str) -> Result<(), DocError> {
    if !valid_pin_order_key(key) {
        return Err(invalid("Invalid task order key"));
    }
    Ok(())
}

fn task_fields(task: &Task, title: String, order_key: String) -> BTreeMap<String, Value> {
    fields([
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
        ("mode", json!(task.mode)),
        ("planPending", json!(task.plan_pending)),
        ("sendError", opt_str(task.send_error.as_deref())),
        ("plan", json!(task.plan)),
        ("planDoc", json!(task.plan_doc)),
        ("resources", json!(task.resources)),
        ("images", json!(task.images)),
        ("context", json!(task.context)),
    ])
}

/// A re-seed sends the whole row and its clocks as one op; the room rejects an oversized op's batch.
pub(super) fn check_row_fits(row: &RegistryRow) -> Result<(), DocError> {
    if fits_op_budget(&row_to_seed_op(row)) {
        return Ok(());
    }
    Err(invalid(if row.kind == KIND_TASK_STEPS {
        "This step is too large to sync; shorten its spec"
    } else {
        "This task is too large to sync: its notes and plan overview share one limit, so shorten one of them"
    }))
}

pub fn check_task_syncs(task: &Task) -> Result<(), DocError> {
    let title = checked_title(&task.title)?;
    check_notes(&task.notes)?;
    let fields = task_fields(task, title, task.order_key.clone());
    let clock = encode_hlc(9_999_999_999_999, 999_999, CLOCK_DEVICE);
    check_row_fits(&RegistryRow {
        kind: KIND_TASKS.into(),
        id: task.id.clone(),
        seq: 0,
        deleted: false,
        del_hlc: None,
        clocks: fields
            .keys()
            .map(|key| (key.clone(), clock.clone()))
            .collect(),
        fields,
    })
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
        if !valid_row_id(&task.id) {
            return Err(invalid("Invalid task ID"));
        }
        let title = checked_title(&task.title)?;
        check_notes(&task.notes)?;
        check_images(&task.images)?;
        check_context(&task.context)?;
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
        let set = task_fields(task, title, order_key);
        let op = self.task_op(&task.id, OpKind::Upsert, set, hlc)?;
        self.enqueue_ops(vec![op]);
        Ok(true)
    }

    fn task_op(
        &self,
        id: &str,
        op: OpKind,
        set: BTreeMap<String, Value>,
        hlc: String,
    ) -> Result<RowOp, DocError> {
        let op = RowOp {
            kind: KIND_TASKS.into(),
            id: id.into(),
            op,
            set: Some(set),
            hlc,
            clocks: None,
        };
        let row = overlay(self.overlay_row(KIND_TASKS, id).as_ref(), [&op])
            .ok_or_else(|| invalid("Task no longer exists"))?;
        check_row_fits(&row)?;
        Ok(op)
    }

    pub fn update_task(
        &mut self,
        task_id: &str,
        patch: &TaskPatch,
        now: DateTime<Utc>,
    ) -> Result<bool, DocError> {
        let Some(op) = self.task_update_op(task_id, patch, now)? else {
            return Ok(false);
        };
        self.enqueue_ops(vec![op]);
        Ok(true)
    }

    pub(super) fn task_update_op(
        &mut self,
        task_id: &str,
        patch: &TaskPatch,
        now: DateTime<Utc>,
    ) -> Result<Option<RowOp>, DocError> {
        let Some(task) = self.task(task_id) else {
            return Ok(None);
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
        if let Some(mode) = patch.mode {
            set.insert("mode".into(), json!(mode));
        }
        if let Some(plan_pending) = patch.plan_pending {
            set.insert("planPending".into(), json!(plan_pending));
        }
        if let Some(send_error) = &patch.send_error {
            set.insert("sendError".into(), opt_str(send_error.as_deref()));
        }
        if let Some(plan) = &patch.plan {
            check_notes(plan)?;
            set.insert("plan".into(), json!(plan));
        }
        if let Some(plan_doc) = patch.plan_doc {
            set.insert("planDoc".into(), json!(plan_doc));
        }
        if let Some(resources) = &patch.resources {
            set.insert("resources".into(), json!(resources));
        }
        if let Some(images) = &patch.images {
            check_images(images)?;
            set.insert("images".into(), json!(images));
        }
        if let Some(context) = &patch.context {
            check_context(context)?;
            set.insert("context".into(), json!(context));
        }
        set.insert("updatedAt".into(), json!(now.timestamp_millis()));
        let hlc = self.next_hlc();
        self.task_op(task_id, OpKind::Update, set, hlc).map(Some)
    }

    /// A flagged row, not a tombstone, so a retried create cannot revive it.
    pub fn delete_task(&mut self, task_id: &str, now: DateTime<Utc>) -> Result<bool, DocError> {
        if self.task(task_id).is_none() {
            return Ok(false);
        }
        let hlc = self.next_hlc();
        let flag = |kind: &str, id: &str| RowOp {
            kind: kind.into(),
            id: id.into(),
            op: OpKind::Update,
            set: Some(fields([
                ("deleted", json!(true)),
                ("updatedAt", json!(now.timestamp_millis())),
            ])),
            hlc: hlc.clone(),
            clocks: None,
        };
        let mut ops = vec![flag(KIND_TASKS, task_id)];
        ops.extend(
            self.task_steps_of(task_id)
                .iter()
                .map(|step| flag(KIND_TASK_STEPS, &step.id)),
        );
        self.enqueue_ops(ops);
        Ok(true)
    }
}
