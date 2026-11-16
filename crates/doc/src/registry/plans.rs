use super::task_steps::flagged_deleted;
use super::tasks::{check_images, check_notes, check_step_count, invalid};
use super::*;
use zeron_proto::{ChatPlan, TaskStep, TaskStepPatch};

fn check_plan(plan: &ChatPlan) -> Result<(), DocError> {
    check_notes(&plan.overview)?;
    check_images(&plan.images)
}

fn plan_value(plan: Option<&ChatPlan>) -> Result<Value, DocError> {
    Ok(plan
        .map(serde_json::to_value)
        .transpose()?
        .unwrap_or(Value::Null))
}

impl RegistryDoc {
    fn chat_op(&mut self, chat_id: &str, set: BTreeMap<String, Value>) -> Result<RowOp, DocError> {
        let op = RowOp {
            kind: KIND_CHATS.into(),
            id: chat_id.into(),
            op: OpKind::Update,
            set: Some(set),
            hlc: self.next_hlc(),
            clocks: None,
        };
        self.chat_row_after(chat_id, &op)?;
        Ok(op)
    }

    fn chat_row_after(&self, chat_id: &str, op: &RowOp) -> Result<RegistryRow, DocError> {
        let row = overlay(self.overlay_row(KIND_CHATS, chat_id).as_ref(), [op])
            .ok_or_else(|| invalid("Chat no longer exists"))?;
        if !fits_op_budget(&row_to_seed_op(&row)) {
            return Err(invalid(
                "This plan is too large to sync with its chat; shorten the overview",
            ));
        }
        Ok(row)
    }

    /// Closed plans' overviews give way, oldest first, so a long history never refuses a write.
    fn plan_op(&mut self, chat_id: &str, plan: Option<&ChatPlan>) -> Result<RowOp, DocError> {
        let Some(plan) = plan else {
            return self.chat_op(chat_id, fields([("plan", Value::Null)]));
        };
        check_plan(plan)?;
        let mut plan = plan.clone();
        loop {
            let op = self.chat_op(chat_id, fields([("plan", plan_value(Some(&plan))?)]));
            match plan.closed.iter_mut().find(|old| !old.overview.is_empty()) {
                Some(old) if op.is_err() => old.overview.clear(),
                _ => return op,
            }
        }
    }

    fn current_steps(&self, chat_id: &str, plan: Option<&ChatPlan>) -> Vec<TaskStep> {
        self.steps_of(chat_id)
            .into_iter()
            .filter(|step| !plan.is_some_and(|plan| plan.is_closed(step.plan)))
            .collect()
    }

    fn chat_plan(&self, chat_id: &str) -> Result<Option<ChatPlan>, DocError> {
        Ok(self.chat(chat_id)?.and_then(|chat| chat.plan))
    }

    pub fn set_chat_plan(
        &mut self,
        chat_id: &str,
        plan: Option<&ChatPlan>,
        pending: bool,
    ) -> Result<(), DocError> {
        if let Some(plan) = plan {
            check_plan(plan)?;
        }
        let op = self.chat_op(
            chat_id,
            fields([("plan", plan_value(plan)?), ("planPending", json!(pending))]),
        )?;
        self.enqueue_ops(vec![op]);
        Ok(())
    }

    pub fn set_plan_pending(&mut self, chat_id: &str, pending: bool) -> Result<(), DocError> {
        let op = self.chat_op(chat_id, fields([("planPending", json!(pending))]))?;
        self.enqueue_ops(vec![op]);
        Ok(())
    }

    /// One batch with the chat's plan, so a refused row leaves the old plan whole.
    pub fn write_plan(
        &mut self,
        chat_id: &str,
        plan: &ChatPlan,
        steps: &[TaskStep],
        pending: bool,
        now: DateTime<Utc>,
    ) -> Result<(), DocError> {
        let replaced = self.chat_plan(chat_id)?;
        let dropped: Vec<TaskStep> = self
            .current_steps(chat_id, replaced.as_ref())
            .into_iter()
            .filter(|step| !self.step_started(step))
            .collect();
        let kept = self
            .current_steps(chat_id, Some(plan))
            .into_iter()
            .filter(|step| self.step_started(step))
            .count();
        check_step_count(kept + steps.len())?;
        let mut plan_op = self.plan_op(chat_id, Some(plan))?;
        if pending && let Some(set) = plan_op.set.as_mut() {
            set.insert("planPending".into(), json!(true));
        }
        let ops = self.new_step_ops(chat_id, steps, self.last_step_key(chat_id))?;
        let hlc = self.next_hlc();
        let mut batch: Vec<RowOp> = std::iter::once(plan_op)
            .chain(
                dropped
                    .into_iter()
                    .map(|step| flagged_deleted(&step.id, now, hlc.clone())),
            )
            .collect();
        batch.extend(ops);
        self.enqueue_ops(batch);
        Ok(())
    }

    /// Every row is checked before any is queued, so a refused edit applies nothing.
    pub fn edit_plan(
        &mut self,
        chat_id: &str,
        plan: Option<&ChatPlan>,
        updates: &[(String, TaskStepPatch)],
        creates: &[TaskStep],
        deletes: &[String],
        now: DateTime<Utc>,
    ) -> Result<(), DocError> {
        let current = self.current_steps(chat_id, self.chat_plan(chat_id)?.as_ref());
        check_step_count((current.len() + creates.len()).saturating_sub(deletes.len()))?;
        let mut batch: Vec<RowOp> = match plan {
            Some(plan) => vec![self.plan_op(chat_id, Some(plan))?],
            None => Vec::new(),
        };
        for (step_id, patch) in updates {
            let op = self
                .step_update_op(step_id, patch, now)?
                .ok_or_else(|| invalid("Step no longer exists"))?;
            batch.push(op);
        }
        let after = updates
            .iter()
            .filter_map(|(_, patch)| patch.order_key.clone())
            .chain(self.last_step_key(chat_id))
            .max();
        batch.extend(self.new_step_ops(chat_id, creates, after)?);
        for step_id in deletes {
            if self.task_step(step_id).is_none() {
                return Err(invalid("Step no longer exists"));
            }
            batch.push(flagged_deleted(step_id, now, self.next_hlc()));
        }
        self.enqueue_ops(batch);
        Ok(())
    }

    pub fn move_plans_to_chats(&mut self) -> usize {
        let text = |row: &RegistryRow, key: &str| {
            row.fields
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        let flag = |row: &RegistryRow, key: &str| {
            row.fields
                .get(key)
                .and_then(Value::as_bool)
                .unwrap_or(false)
        };
        let list = |row: &RegistryRow, key: &str| -> Vec<String> {
            row.fields
                .get(key)
                .and_then(|value| serde_json::from_value(value.clone()).ok())
                .unwrap_or_default()
        };
        let mut tasks: Vec<RegistryRow> = self
            .overlay_rows(KIND_TASKS)
            .into_iter()
            .filter(|row| text(row, "mode").as_deref() == Some("orchestrated"))
            .filter(|row| !flag(row, "deleted"))
            .collect();
        tasks.sort_by(|a, b| a.id.cmp(&b.id));
        let mut steps: Vec<RegistryRow> = self
            .overlay_rows(KIND_TASK_STEPS)
            .into_iter()
            .filter(|row| !flag(row, "deleted"))
            .collect();
        steps.sort_by(|a, b| a.id.cmp(&b.id));
        let hlc = encode_hlc(1, 0, "migration");
        let mut written = 0;
        for task in tasks {
            let Some(chat_id) = text(&task, "chatId") else {
                continue;
            };
            let Some(chat) = self.overlay_row(KIND_CHATS, &chat_id) else {
                continue;
            };
            let plan = ChatPlan {
                overview: text(&task, "plan").unwrap_or_default(),
                plan_doc: flag(&task, "planDoc"),
                same_agents: flag(&task, "sameAgents"),
                images: list(&task, "images"),
                resources: list(&task, "resources"),
                closed: Vec::new(),
            };
            let Ok(plan) = serde_json::to_value(&plan) else {
                continue;
            };
            let mut set = fields([("plan", plan)]);
            if flag(&task, "planPending") {
                set.insert("planPending".into(), json!(true));
            }
            let update = |kind: &str, id: &str, set: BTreeMap<String, Value>| RowOp {
                kind: kind.into(),
                id: id.into(),
                op: OpKind::Update,
                set: Some(set),
                hlc: hlc.clone(),
                clocks: None,
            };
            let chat_op = update(KIND_CHATS, &chat_id, set);
            let (moved, changed) = apply_op(Some(&chat), &chat_op);
            if moved.is_none_or(|row| !fits_op_budget(&row_to_seed_op(&row))) {
                tracing::warn!(task = %task.id, chat = %chat_id, "plan too large for its lead chat; left on the task");
                continue;
            }
            let mut ops: Vec<RowOp> = changed.then_some(chat_op).into_iter().collect();
            for step in steps
                .iter()
                .filter(|step| text(step, "taskId").as_deref() == Some(task.id.as_str()))
            {
                let op = update(
                    KIND_TASK_STEPS,
                    &step.id,
                    fields([("ownerChatId", json!(chat_id))]),
                );
                let current = self.overlay_row(KIND_TASK_STEPS, &step.id);
                if apply_op(current.as_ref(), &op).1 {
                    ops.push(op);
                }
            }
            written += ops.len();
            self.enqueue_ops(ops);
        }
        written
    }
}
