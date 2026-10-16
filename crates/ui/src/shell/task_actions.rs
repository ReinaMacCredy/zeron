use super::*;
use zeron_proto::TaskPatch;

impl Shell {
    /// Detached, unlike `mutate`: a follow-up edit must not cancel the create.
    fn mutate_task_row(&mut self, params: serde_json::Value, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.show_notice(crate::toast::ToastKind::Error, "Engine not connected", cx);
            cx.notify();
            return;
        };
        Self::spawn_mutate(engine, params, cx).detach();
    }

    pub fn create_task(
        &mut self,
        title: &str,
        notes: &str,
        space_id: Option<String>,
        cx: &mut Context<Self>,
    ) -> String {
        let task_id = uuid::Uuid::new_v4().to_string();
        self.mutate_task_row(
            serde_json::json!({
                "op": "createTask", "taskId": task_id, "title": title, "notes": notes,
                "spaceId": space_id,
            }),
            cx,
        );
        task_id
    }

    pub fn update_task(&mut self, task_id: &str, patch: TaskPatch, cx: &mut Context<Self>) {
        self.mutate_task_row(
            serde_json::json!({ "op": "updateTask", "taskId": task_id, "patch": patch }),
            cx,
        );
    }

    pub fn delete_task(&mut self, task_id: &str, cx: &mut Context<Self>) {
        self.mutate_task_row(
            serde_json::json!({ "op": "deleteTask", "taskId": task_id }),
            cx,
        );
    }

    pub fn start_task(
        &mut self,
        task_id: &str,
        space_id: Option<String>,
        cx: &mut Context<Self>,
    ) -> String {
        let chat_id = uuid::Uuid::new_v4().to_string();
        self.mutate_task_row(
            serde_json::json!({
                "op": "startTask", "taskId": task_id, "chatId": chat_id, "spaceId": space_id,
            }),
            cx,
        );
        chat_id
    }
}
