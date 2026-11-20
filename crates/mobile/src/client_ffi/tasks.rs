use zeron_client as zc;

use super::{ChatConfig, CoreClient, CoreResult, ProjectRef, on_runtime};

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TaskStatus {
    ToDo,
    Working,
    NeedsYou,
    Done,
}

impl From<zeron_proto::TaskStatus> for TaskStatus {
    fn from(s: zeron_proto::TaskStatus) -> Self {
        match s {
            zeron_proto::TaskStatus::ToDo => Self::ToDo,
            zeron_proto::TaskStatus::Working => Self::Working,
            zeron_proto::TaskStatus::NeedsYou => Self::NeedsYou,
            zeron_proto::TaskStatus::Done => Self::Done,
        }
    }
}

impl From<TaskStatus> for zeron_proto::TaskStatus {
    fn from(s: TaskStatus) -> Self {
        match s {
            TaskStatus::ToDo => Self::ToDo,
            TaskStatus::Working => Self::Working,
            TaskStatus::NeedsYou => Self::NeedsYou,
            TaskStatus::Done => Self::Done,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TreeStatus {
    Queued,
    Running,
    Waiting,
    Done,
    Failed,
    Stopped,
}

impl From<zeron_doc::tree::TreeStatus> for TreeStatus {
    fn from(s: zeron_doc::tree::TreeStatus) -> Self {
        use zeron_doc::tree::TreeStatus as S;
        match s {
            S::Queued => Self::Queued,
            S::Running => Self::Running,
            S::Waiting => Self::Waiting,
            S::Done => Self::Done,
            S::Failed => Self::Failed,
            S::Stopped => Self::Stopped,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TreeNodeKind {
    Root,
    Agent,
    Native,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TreeNodeRow {
    pub id: String,
    pub parent: Option<u32>,
    pub depth: u32,
    pub kind: TreeNodeKind,
    pub title: String,
    pub status: TreeStatus,
    pub muted: bool,
    pub word: String,
    pub brand: Option<String>,
    pub chat_id: Option<String>,
    pub tokens: Option<String>,
    pub time: Option<String>,
}

impl From<&zc::OutlineRow> for TreeNodeRow {
    fn from(row: &zc::OutlineRow) -> Self {
        use zeron_doc::tree::NodeKind;
        let n = &row.node;
        Self {
            id: n.id.clone(),
            parent: n.parent.map(|ix| ix as u32),
            depth: n.depth as u32,
            kind: match n.kind {
                NodeKind::Root => TreeNodeKind::Root,
                NodeKind::Agent => TreeNodeKind::Agent,
                NodeKind::Native => TreeNodeKind::Native,
            },
            title: n.title.clone(),
            status: n.status.into(),
            muted: n.status.muted(),
            word: n.word.to_owned(),
            brand: n.brand.clone().filter(|b| !b.is_empty()),
            chat_id: n.chat_id.clone(),
            tokens: row.tokens.clone(),
            time: row.time.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TreeIndexRow {
    pub root: String,
    pub title: String,
    pub status: TreeStatus,
    pub line: String,
    pub asks: bool,
    pub agents: Option<String>,
    pub segments: Vec<TreeStatus>,
}

impl From<&zeron_doc::tree::IndexRow> for TreeIndexRow {
    fn from(r: &zeron_doc::tree::IndexRow) -> Self {
        Self {
            root: r.root.clone(),
            title: r.title.clone(),
            status: r.status.into(),
            line: r.line.clone(),
            asks: r.asks,
            agents: r.agents_line(),
            segments: r.segments.iter().copied().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TaskTrees {
    pub running: Vec<TreeIndexRow>,
    pub done_today: Vec<TreeIndexRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TreeLabels {
    pub views: Vec<String>,
    pub badge: String,
}

#[uniffi::export]
pub fn tree_labels() -> TreeLabels {
    TreeLabels {
        views: zeron_doc::tree::TREE_VIEWS.map(str::to_owned).to_vec(),
        badge: zeron_doc::tree::BADGE.to_owned(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HostRef {
    pub device_id: String,
    pub name: String,
    pub online: bool,
}

impl From<&zc::HostRef> for HostRef {
    fn from(h: &zc::HostRef) -> Self {
        Self {
            device_id: h.device_id.clone(),
            name: h.name.clone(),
            online: h.online,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TaskRow {
    pub id: String,
    pub title: String,
    pub notes: String,
    pub due_on: Option<String>,
    pub project: Option<ProjectRef>,
    pub status: TaskStatus,
    pub word: String,
    pub lead_chat_id: Option<String>,
    pub host: Option<HostRef>,
    pub brands: Vec<String>,
    pub working_since_ms: Option<i64>,
    pub created_at_ms: i64,
    pub completed_at_ms: Option<i64>,
}

impl From<&zc::TaskRow> for TaskRow {
    fn from(r: &zc::TaskRow) -> Self {
        Self {
            id: r.id.clone(),
            title: r.title.clone(),
            notes: r.notes.clone(),
            due_on: r.due_on.clone(),
            project: r.project.as_ref().map(|p| ProjectRef {
                id: p.id.clone(),
                name: p.name.clone(),
                color_index: p.color_index,
            }),
            status: r.status.into(),
            word: r.word.clone(),
            lead_chat_id: r.lead_chat_id.clone(),
            host: r.host.as_ref().map(Into::into),
            brands: r.brands.clone(),
            working_since_ms: r.working_since_ms,
            created_at_ms: r.created_at_ms,
            completed_at_ms: r.completed_at_ms,
        }
    }
}

fn task_rows(rows: &[zc::TaskRow]) -> Vec<TaskRow> {
    rows.iter().map(Into::into).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TasksPage {
    pub needs_you: Vec<TaskRow>,
    pub working: Vec<TaskRow>,
    pub to_do: Vec<TaskRow>,
    pub done: Vec<TaskRow>,
    pub board: Vec<KanbanColumn>,
    pub needs_you_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct KanbanColumn {
    pub status: TaskStatus,
    pub title: String,
    pub rows: Vec<TaskRow>,
}

#[uniffi::export]
pub fn tasks_view_labels() -> Vec<String> {
    zeron_proto::mission::TASKS_VIEWS
        .map(str::to_owned)
        .to_vec()
}

#[uniffi::export]
pub fn task_drop_completes(from: TaskStatus, to: TaskStatus) -> Option<bool> {
    zeron_proto::mission::drop_completes(from.into(), to.into())
}

impl From<&zc::TasksPage> for TasksPage {
    fn from(p: &zc::TasksPage) -> Self {
        Self {
            needs_you: task_rows(&p.needs_you),
            working: task_rows(&p.working),
            to_do: task_rows(&p.to_do),
            done: task_rows(&p.done),
            board: p
                .board
                .iter()
                .map(|(status, rows)| KanbanColumn {
                    status: (*status).into(),
                    title: zeron_proto::mission::column_title(*status).into(),
                    rows: task_rows(rows),
                })
                .collect(),
            needs_you_count: p.needs_you_count,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContextChat {
    pub chat_id: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TaskDetail {
    pub row: TaskRow,
    pub images: Vec<String>,
    pub context: Vec<ContextChat>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TaskDraft {
    pub title: String,
    pub notes: String,
    pub space_id: Option<String>,
    pub images: Vec<String>,
    pub context: Vec<String>,
}

impl From<TaskDraft> for zc::TaskDraft {
    fn from(d: TaskDraft) -> Self {
        Self {
            title: d.title,
            notes: d.notes,
            space_id: d.space_id,
            images: d.images,
            context: d.context,
        }
    }
}

impl From<&zc::TaskDetail> for TaskDetail {
    fn from(d: &zc::TaskDetail) -> Self {
        Self {
            row: (&d.row).into(),
            images: d.images.clone(),
            context: d
                .context
                .iter()
                .map(|c| ContextChat {
                    chat_id: c.chat_id.clone(),
                    title: c.title.clone(),
                })
                .collect(),
        }
    }
}

#[uniffi::export]
impl CoreClient {
    pub fn tasks_page(&self) -> TasksPage {
        (&self.client.workspace().tasks).into()
    }

    pub fn task_trees(&self) -> TaskTrees {
        let trees = self.client.task_trees();
        TaskTrees {
            running: trees.running.iter().map(Into::into).collect(),
            done_today: trees.done_today.iter().map(Into::into).collect(),
        }
    }

    pub fn task_outline(&self, root_chat_id: String) -> Vec<TreeNodeRow> {
        self.client
            .task_outline(&root_chat_id)
            .iter()
            .map(Into::into)
            .collect()
    }

    pub fn watch_outline_docs(&self, root_chat_id: String) -> CoreResult<()> {
        Ok(self.client.watch_outline_docs(&root_chat_id)?)
    }

    pub async fn refresh_outline_stats(&self, root_chat_id: String) -> CoreResult<bool> {
        let client = self.client.clone();
        on_runtime(async move { client.refresh_outline_stats(&root_chat_id).await }).await
    }

    pub fn task_detail(&self, task_id: String) -> Option<TaskDetail> {
        self.client.task_detail(&task_id).as_ref().map(Into::into)
    }

    pub async fn set_task_completed(&self, task_id: String, completed: bool) -> CoreResult<()> {
        let client = self.client.clone();
        on_runtime(async move { client.set_task_completed(&task_id, completed).await }).await
    }

    /// A Bot's memory, Inbox, board or schedules (`method`: `BotMemory`,
    /// `BotInbox`, `BotBoard`, `BotSchedule`), its params and answer as JSON.
    pub async fn bot_call_json(
        &self,
        device_id: String,
        method: String,
        params_json: String,
    ) -> CoreResult<String> {
        let client = self.client.clone();
        let params = serde_json::from_str(&params_json).unwrap_or_else(|_| serde_json::json!({}));
        on_runtime(async move {
            client
                .bot_call(&device_id, &method, params)
                .await
                .map(|v| v.to_string())
        })
        .await
    }

    pub async fn delete_task(&self, task_id: String) -> CoreResult<()> {
        let client = self.client.clone();
        on_runtime(async move { client.delete_task(&task_id).await }).await
    }

    pub async fn create_task(&self, device_id: String, draft: TaskDraft) -> CoreResult<String> {
        let client = self.client.clone();
        on_runtime(async move { client.create_task(&device_id, draft.into()).await }).await
    }

    pub async fn set_task_images(&self, task_id: String, images: Vec<String>) -> CoreResult<()> {
        let client = self.client.clone();
        let patch = zeron_proto::TaskPatch {
            images: Some(images),
            ..Default::default()
        };
        on_runtime(async move { client.update_task(&task_id, patch).await }).await
    }

    pub async fn set_task_context(&self, task_id: String, chat_ids: Vec<String>) -> CoreResult<()> {
        let client = self.client.clone();
        let patch = zeron_proto::TaskPatch {
            context: Some(chat_ids),
            ..Default::default()
        };
        on_runtime(async move { client.update_task(&task_id, patch).await }).await
    }

    pub async fn unassign_task(&self, task_id: String) -> CoreResult<()> {
        let client = self.client.clone();
        let patch = zeron_proto::TaskPatch {
            chat_id: Some(None),
            ..Default::default()
        };
        on_runtime(async move { client.update_task(&task_id, patch).await }).await
    }

    /// Starts the task in its own project on `device_id`; returns the new chat id.
    pub async fn start_task(
        &self,
        task_id: String,
        device_id: String,
        config: Option<ChatConfig>,
    ) -> CoreResult<String> {
        let client = self.client.clone();
        let config = config.map(TryInto::try_into).transpose()?;
        on_runtime(async move { client.start_task(&task_id, &device_id, None, config).await }).await
    }

    pub async fn assign_task(
        &self,
        task_id: String,
        chat_id: String,
        send: bool,
    ) -> CoreResult<()> {
        let client = self.client.clone();
        on_runtime(async move { client.assign_task(&task_id, &chat_id, send).await }).await
    }
}
