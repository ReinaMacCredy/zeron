use super::tasks::{
    at, check_images, check_notes, check_order_key, check_row_fits, check_step_count, checked_title,
    invalid,
};
use super::*;
use zeron_proto::{
    StepOutcome, StepTurn, TaskPatch, TaskStep, TaskStepPatch, pin_order_key_between,
    valid_pin_order_key,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTaskStep {
    id: String,
    #[serde(default)]
    task_id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    order_key: String,
    #[serde(default)]
    chat_id: Option<String>,
    #[serde(default)]
    outcome: Option<StepOutcome>,
    #[serde(default)]
    created_at: i64,
    #[serde(default)]
    updated_at: i64,
    #[serde(default)]
    deleted: bool,
    #[serde(default)]
    turn: Option<StepTurn>,
    #[serde(default)]
    images: Vec<String>,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    route: Option<serde_json::Value>,
}

impl From<RawTaskStep> for TaskStep {
    fn from(raw: RawTaskStep) -> Self {
        TaskStep {
            id: raw.id,
            task_id: raw.task_id,
            title: raw.title,
            notes: raw.notes,
            order_key: raw.order_key,
            chat_id: raw.chat_id,
            outcome: raw.outcome,
            created_at: at(raw.created_at),
            updated_at: at(raw.updated_at),
            deleted: raw.deleted,
            turn: raw.turn,
            images: raw.images,
            role: raw.role,
            route: raw
                .route
                .and_then(|route| serde_json::from_value(route).ok()),
        }
    }
}

fn step_fields(step: &TaskStep, title: String, order_key: String) -> BTreeMap<String, Value> {
    fields([
        ("id", json!(step.id)),
        ("taskId", json!(step.task_id)),
        ("title", json!(title)),
        ("notes", json!(step.notes)),
        ("orderKey", json!(order_key)),
        ("chatId", opt_str(step.chat_id.as_deref())),
        ("outcome", json!(step.outcome)),
        ("createdAt", json!(step.created_at.timestamp_millis())),
        ("updatedAt", json!(step.updated_at.timestamp_millis())),
        ("deleted", json!(false)),
        ("turn", json!(step.turn)),
        ("images", json!(step.images)),
        ("role", opt_str(step.role.as_deref())),
        ("route", json!(step.route)),
    ])
}

fn flagged_deleted(id: &str, now: DateTime<Utc>, hlc: String) -> RowOp {
    RowOp {
        kind: KIND_TASK_STEPS.into(),
        id: id.into(),
        op: OpKind::Update,
        set: Some(fields([
            ("deleted", json!(true)),
            ("updatedAt", json!(now.timestamp_millis())),
        ])),
        hlc,
        clocks: None,
    }
}

impl RegistryDoc {
    pub fn task_step(&self, step_id: &str) -> Option<TaskStep> {
        self.overlay_row(KIND_TASK_STEPS, step_id)
            .and_then(|row| row_to::<RawTaskStep>(&row))
            .map(TaskStep::from)
            .filter(|step| !step.deleted)
    }

    pub fn read_task_steps(&self) -> Result<Vec<TaskStep>, DocError> {
        let mut steps: Vec<TaskStep> = self
            .read_kind::<RawTaskStep>(KIND_TASK_STEPS)
            .into_iter()
            .map(TaskStep::from)
            .filter(|step| !step.deleted)
            .collect();
        steps.sort_by(|a, b| {
            (
                &a.task_id,
                a.order_key.is_empty(),
                &a.order_key,
                a.created_at,
                &a.id,
            )
                .cmp(&(
                    &b.task_id,
                    b.order_key.is_empty(),
                    &b.order_key,
                    b.created_at,
                    &b.id,
                ))
        });
        Ok(steps)
    }

    pub fn task_steps_of(&self, task_id: &str) -> Vec<TaskStep> {
        self.read_task_steps()
            .unwrap_or_default()
            .into_iter()
            .filter(|step| step.task_id == task_id)
            .collect()
    }

    fn last_step_key(&self, task_id: &str) -> Option<String> {
        self.overlay_rows(KIND_TASK_STEPS)
            .into_iter()
            .filter(|row| row.fields.get("taskId").and_then(Value::as_str) == Some(task_id))
            .filter_map(|row| {
                row.fields
                    .get("orderKey")
                    .and_then(Value::as_str)
                    .filter(|key| valid_pin_order_key(key))
                    .map(str::to_owned)
            })
            .max()
    }

    fn step_upsert(&mut self, step: &TaskStep, after: Option<&str>) -> Result<RowOp, DocError> {
        if !valid_row_id(&step.id) {
            return Err(invalid("Invalid step ID"));
        }
        let title = checked_title(&step.title)?;
        check_notes(&step.notes)?;
        check_images(&step.images)?;
        let hlc = self.next_hlc();
        let order_key = if step.order_key.is_empty() {
            pin_order_key_between(after, None, &hlc).map_err(invalid)?
        } else {
            check_order_key(&step.order_key)?;
            step.order_key.clone()
        };
        let op = RowOp {
            kind: KIND_TASK_STEPS.into(),
            id: step.id.clone(),
            op: OpKind::Upsert,
            set: Some(step_fields(step, title, order_key)),
            hlc,
            clocks: None,
        };
        let row = overlay(None, [&op]).ok_or_else(|| invalid("Step no longer exists"))?;
        check_row_fits(&row)?;
        Ok(op)
    }

    /// A taken id, deleted rows included, is never overwritten: retries must not revive.
    pub fn create_task_step(&mut self, step: &TaskStep) -> Result<bool, DocError> {
        if self.task(&step.task_id).is_none() {
            return Err(invalid("Task no longer exists"));
        }
        if self.overlay_row(KIND_TASK_STEPS, &step.id).is_some() {
            return Ok(false);
        }
        let after = self.last_step_key(&step.task_id);
        let op = self.step_upsert(step, after.as_deref())?;
        self.enqueue_ops(vec![op]);
        Ok(true)
    }

    fn task_patch_op(
        &mut self,
        task_id: &str,
        patch: &TaskPatch,
        now: DateTime<Utc>,
    ) -> Result<Option<RowOp>, DocError> {
        if *patch == TaskPatch::default() {
            return Ok(None);
        }
        self.task_update_op(task_id, patch, now)?
            .map(Some)
            .ok_or_else(|| invalid("Task no longer exists"))
    }

    fn new_step_ops(
        &mut self,
        task_id: &str,
        steps: &[TaskStep],
        mut after: Option<String>,
    ) -> Result<Vec<RowOp>, DocError> {
        let mut ops = Vec::new();
        for step in steps {
            if step.task_id != task_id {
                return Err(invalid("A step belongs to another task"));
            }
            let op = self.step_upsert(step, after.as_deref())?;
            after = op
                .set
                .as_ref()
                .and_then(|set| set.get("orderKey"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            ops.push(op);
        }
        Ok(ops)
    }

    /// One batch with the task patch, so a refused row leaves the old plan whole.
    pub fn plan_task_steps(
        &mut self,
        task_id: &str,
        steps: &[TaskStep],
        patch: &TaskPatch,
        now: DateTime<Utc>,
    ) -> Result<(), DocError> {
        if self.task(task_id).is_none() {
            return Err(invalid("Task no longer exists"));
        }
        let (kept, dropped): (Vec<TaskStep>, Vec<TaskStep>) = self
            .task_steps_of(task_id)
            .into_iter()
            .partition(|step| step.chat_id.is_some() || step.outcome.is_some());
        check_step_count(kept.len() + steps.len())?;
        let task_op = self.task_patch_op(task_id, patch, now)?;
        let ops = self.new_step_ops(task_id, steps, self.last_step_key(task_id))?;
        let hlc = self.next_hlc();
        let dropped = dropped
            .into_iter()
            .map(|step| flagged_deleted(&step.id, now, hlc.clone()));
        let mut batch: Vec<RowOp> = task_op.into_iter().chain(dropped).collect();
        batch.extend(ops);
        self.enqueue_ops(batch);
        Ok(())
    }

    /// Every row is checked before any is queued, so a refused edit applies nothing.
    pub fn edit_task_plan(
        &mut self,
        task_id: &str,
        patch: &TaskPatch,
        updates: &[(String, TaskStepPatch)],
        creates: &[TaskStep],
        deletes: &[String],
        now: DateTime<Utc>,
    ) -> Result<(), DocError> {
        if self.task(task_id).is_none() {
            return Err(invalid("Task no longer exists"));
        }
        check_step_count(
            (self.task_steps_of(task_id).len() + creates.len()).saturating_sub(deletes.len()),
        )?;
        let mut batch: Vec<RowOp> = self
            .task_patch_op(task_id, patch, now)?
            .into_iter()
            .collect();
        for (step_id, patch) in updates {
            let op = self
                .step_update_op(step_id, patch, now)?
                .ok_or_else(|| invalid("Step no longer exists"))?;
            batch.push(op);
        }
        let after = updates
            .iter()
            .filter_map(|(_, patch)| patch.order_key.clone())
            .chain(self.last_step_key(task_id))
            .max();
        batch.extend(self.new_step_ops(task_id, creates, after)?);
        for step_id in deletes {
            if self.task_step(step_id).is_none() {
                return Err(invalid("Step no longer exists"));
            }
            batch.push(flagged_deleted(step_id, now, self.next_hlc()));
        }
        self.enqueue_ops(batch);
        Ok(())
    }

    pub fn update_task_step(
        &mut self,
        step_id: &str,
        patch: &TaskStepPatch,
        now: DateTime<Utc>,
    ) -> Result<bool, DocError> {
        let Some(op) = self.step_update_op(step_id, patch, now)? else {
            return Ok(false);
        };
        self.enqueue_ops(vec![op]);
        Ok(true)
    }

    fn step_update_op(
        &mut self,
        step_id: &str,
        patch: &TaskStepPatch,
        now: DateTime<Utc>,
    ) -> Result<Option<RowOp>, DocError> {
        if self.task_step(step_id).is_none() {
            return Ok(None);
        }
        let mut set = BTreeMap::new();
        if let Some(title) = &patch.title {
            set.insert("title".into(), json!(checked_title(title)?));
        }
        if let Some(notes) = &patch.notes {
            check_notes(notes)?;
            set.insert("notes".into(), json!(notes));
        }
        if let Some(outcome) = &patch.outcome {
            set.insert("outcome".into(), json!(outcome));
        }
        if let Some(order_key) = &patch.order_key {
            check_order_key(order_key)?;
            set.insert("orderKey".into(), json!(order_key));
        }
        if let Some(chat_id) = &patch.chat_id {
            set.insert("chatId".into(), opt_str(chat_id.as_deref()));
        }
        if let Some(turn) = &patch.turn {
            set.insert("turn".into(), json!(turn));
        }
        if let Some(images) = &patch.images {
            check_images(images)?;
            set.insert("images".into(), json!(images));
        }
        if let Some(role) = &patch.role {
            set.insert("role".into(), opt_str(role.as_deref()));
        }
        if let Some(route) = &patch.route {
            set.insert("route".into(), json!(route));
        }
        set.insert("updatedAt".into(), json!(now.timestamp_millis()));
        let op = RowOp {
            kind: KIND_TASK_STEPS.into(),
            id: step_id.into(),
            op: OpKind::Update,
            set: Some(set),
            hlc: self.next_hlc(),
            clocks: None,
        };
        let row = overlay(self.overlay_row(KIND_TASK_STEPS, step_id).as_ref(), [&op])
            .ok_or_else(|| invalid("Step no longer exists"))?;
        check_row_fits(&row)?;
        Ok(Some(op))
    }

    /// A flagged row, not a tombstone, so a retried create cannot revive it.
    pub fn delete_task_step(
        &mut self,
        step_id: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, DocError> {
        if self.task_step(step_id).is_none() {
            return Ok(false);
        }
        let hlc = self.next_hlc();
        self.enqueue_ops(vec![flagged_deleted(step_id, now, hlc)]);
        Ok(true)
    }
}
