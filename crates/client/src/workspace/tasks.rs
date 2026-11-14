use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use zeron_doc::tree::{self, IndexRows, TreeNode, TreeSource};
use zeron_doc::{SessionMessageEntry, WorkspaceState};
use zeron_proto::mission::{self, Rollup};
use zeron_proto::{
    Chat, ChatIndicator, HarnessId, NeedsYou, Session, SessionStats, Task, TaskStatus,
};

use super::view::{DeriveContext, DeviceView, ProjectRef, SessionRow, project_color_index};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostRef {
    pub device_id: String,
    pub name: String,
    pub online: bool,
}

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TasksPage {
    pub needs_you: Vec<TaskRow>,
    pub working: Vec<TaskRow>,
    pub to_do: Vec<TaskRow>,
    pub done: Vec<TaskRow>,
    pub board: Vec<(TaskStatus, Vec<TaskRow>)>,
    pub needs_you_count: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskDetail {
    pub row: TaskRow,
    pub images: Vec<String>,
    pub context: Vec<ContextChat>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextChat {
    pub chat_id: String,
    pub title: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskDraft {
    pub title: String,
    pub notes: String,
    pub space_id: Option<String>,
    pub images: Vec<String>,
    pub context: Vec<String>,
}

struct Ctx<'a> {
    state: &'a WorkspaceState,
    rows: &'a HashMap<String, Arc<SessionRow>>,
    devices: &'a [DeviceView],
    transcripts: &'a HashMap<String, Vec<Arc<SessionMessageEntry>>>,
    now: DateTime<Utc>,
}

impl Ctx<'_> {
    fn session(&self, chat_id: &str) -> Option<&Session> {
        self.state.sessions.iter().find(|s| s.chat_id == chat_id)
    }

    fn indicator(&self, chat: &Chat) -> ChatIndicator {
        self.rows
            .get(&chat.id)
            .map_or(ChatIndicator::Idle, |row| row.indicator)
    }

    fn host(&self, device_id: &str) -> HostRef {
        let device = self.devices.iter().find(|d| d.id == device_id);
        HostRef {
            device_id: device_id.to_owned(),
            name: device.map_or_else(|| "Unknown device".to_owned(), |d| d.name.clone()),
            online: device.is_some_and(|d| d.online),
        }
    }

    fn rollup(&self, task: &Task) -> Rollup<String> {
        mission::rollup(
            task,
            &self.state.chats,
            &[],
            std::iter::empty(),
            |chat| self.indicator(chat),
            |chat| brand(chat).unwrap_or_default(),
        )
    }

    fn task_device(&self, task: &Task) -> Option<String> {
        task_device(self.state, task)
    }

    fn task_row(&self, task: &Task) -> TaskRow {
        let plan = self.rollup(task);
        let lead = mission::linked_chat(&self.state.chats, task);
        let project = task
            .space_id
            .as_deref()
            .and_then(|id| self.state.spaces.iter().find(|s| s.id == id))
            .map(|space| ProjectRef {
                id: space.id.clone(),
                name: space.display_name().to_owned(),
                color_index: project_color_index(&space.path),
            });
        TaskRow {
            id: task.id.clone(),
            title: task.title.clone(),
            notes: task.notes.clone(),
            due_on: task.due_on.clone(),
            project,
            status: plan.status,
            word: plan.needs.map_or_else(
                || mission::column_title(plan.status).to_owned(),
                NeedsYou::label,
            ),
            lead_chat_id: lead.map(|chat| chat.id.clone()),
            host: self.task_device(task).map(|device| self.host(&device)),
            brands: lead
                .and_then(brand)
                .filter(|mark| !mark.is_empty())
                .into_iter()
                .collect(),
            working_since_ms: lead
                .and_then(|chat| self.rows.get(&chat.id))
                .and_then(|row| row.working_since_ms),
            created_at_ms: task.created_at.timestamp_millis(),
            completed_at_ms: task.completed_at.map(|at| at.timestamp_millis()),
        }
    }
}

impl TreeSource for Ctx<'_> {
    type Brand = String;

    fn chats(&self) -> &[Chat] {
        &self.state.chats
    }

    fn tasks(&self) -> &[Task] {
        &self.state.tasks
    }

    fn now(&self) -> DateTime<Utc> {
        self.now
    }

    fn indicator(&self, chat: &Chat) -> ChatIndicator {
        Ctx::indicator(self, chat)
    }

    fn running_subagents(&self, chat_id: &str) -> usize {
        zeron_proto::view::running_subagents(self.session(chat_id), self.now) as usize
    }

    fn started_at(&self, chat_id: &str) -> Option<DateTime<Utc>> {
        self.session(chat_id).and_then(|session| session.started_at)
    }

    fn hidden(&self, _: &str) -> bool {
        false
    }

    fn brand(&self, harness: Option<HarnessId>, model: Option<&str>) -> String {
        harness_brand(harness, model).unwrap_or_default()
    }

    fn rollup(&self, task: &Task) -> Rollup<String> {
        Ctx::rollup(self, task)
    }

    fn transcript(&self, chat_id: &str) -> Box<dyn Iterator<Item = &SessionMessageEntry> + '_> {
        Box::new(
            self.transcripts
                .get(chat_id)
                .into_iter()
                .flatten()
                .map(|entry| entry.as_ref()),
        )
    }
}

fn brand(chat: &Chat) -> Option<String> {
    let config = chat.config.as_ref();
    harness_brand(
        config.map(|c| c.harness),
        config.and_then(|c| c.model.as_deref()),
    )
}

fn harness_brand(harness: Option<HarnessId>, model: Option<&str>) -> Option<String> {
    let harness = HarnessId::brand_of(harness, model)?;
    serde_json::to_value(harness)
        .ok()?
        .as_str()
        .map(str::to_owned)
}

pub(crate) fn task_device(state: &WorkspaceState, task: &Task) -> Option<String> {
    mission::linked_chat(&state.chats, task)
        .map(|chat| chat.device_id.clone())
        .or_else(|| {
            let space = task.space_id.as_deref()?;
            let space = state.spaces.iter().find(|s| s.id == space)?;
            Some(space.device_id.clone())
        })
}

pub(crate) fn derive(
    state: &WorkspaceState,
    rows: &HashMap<String, Arc<SessionRow>>,
    devices: &[DeviceView],
    cx: &DeriveContext<'_>,
) -> TasksPage {
    tasks_page(&Ctx {
        state,
        rows,
        devices,
        transcripts: &HashMap::new(),
        now: cx.now,
    })
}

fn tasks_page(ctx: &Ctx<'_>) -> TasksPage {
    let mut page = TasksPage::default();
    let (open, done) = mission::split_tasks(&ctx.state.tasks);
    page.board = mission::COLUMNS.map(|column| (column, Vec::new())).to_vec();
    for task in open {
        let row = ctx.task_row(task);
        if row.status == TaskStatus::NeedsYou {
            page.needs_you_count += 1;
        }
        if let Some((_, rows)) = page.board.iter_mut().find(|(c, _)| *c == row.status) {
            rows.push(row.clone());
        }
        match row.status {
            TaskStatus::NeedsYou => page.needs_you.push(row),
            TaskStatus::Working => page.working.push(row),
            TaskStatus::ToDo => page.to_do.push(row),
            TaskStatus::Done => page.done.push(row),
        }
    }
    let done: Vec<TaskRow> = done.into_iter().map(|task| ctx.task_row(task)).collect();
    if let Some((_, rows)) = page.board.last_mut() {
        rows.extend(done.iter().cloned());
    }
    page.done.extend(done);
    page
}

pub(crate) fn detail(
    state: &WorkspaceState,
    snapshot: &super::WorkspaceSnapshot,
    task_id: &str,
) -> Option<TaskDetail> {
    let task = state
        .tasks
        .iter()
        .find(|task| task.id == task_id && !task.deleted)?;
    let ctx = Ctx {
        state,
        rows: &snapshot.sessions,
        devices: &snapshot.devices,
        transcripts: &HashMap::new(),
        now: Utc::now(),
    };
    Some(TaskDetail {
        row: ctx.task_row(task),
        images: task.images.clone(),
        context: task
            .context
            .iter()
            .map(|id| ContextChat {
                chat_id: id.clone(),
                title: state
                    .chats
                    .iter()
                    .find(|chat| chat.id == *id)
                    .and_then(|chat| chat.title.clone())
                    .unwrap_or_else(|| "New session".to_owned()),
            })
            .collect(),
    })
}

#[derive(Debug, Clone)]
pub struct OutlineRow {
    pub node: TreeNode<String>,
    pub tokens: Option<String>,
    pub time: Option<String>,
}

pub(crate) fn trees(state: &WorkspaceState, snapshot: &super::WorkspaceSnapshot) -> IndexRows {
    zeron_doc::tree::tree_index(&Ctx {
        state,
        rows: &snapshot.sessions,
        devices: &snapshot.devices,
        transcripts: &HashMap::new(),
        now: Utc::now(),
    })
}

pub(crate) fn outline_docs(
    state: &WorkspaceState,
    snapshot: &super::WorkspaceSnapshot,
    root_chat_id: &str,
    held: &[String],
) -> Vec<String> {
    let ctx = Ctx {
        state,
        rows: &snapshot.sessions,
        devices: &snapshot.devices,
        transcripts: &HashMap::new(),
        now: Utc::now(),
    };
    tree::watched_docs(&ctx, root_chat_id, held, false)
}

pub(crate) fn outline(
    state: &WorkspaceState,
    snapshot: &super::WorkspaceSnapshot,
    stats: &HashMap<String, SessionStats>,
    transcripts: &HashMap<String, Vec<Arc<SessionMessageEntry>>>,
    root_chat_id: &str,
) -> Vec<OutlineRow> {
    let ctx = Ctx {
        state,
        rows: &snapshot.sessions,
        devices: &snapshot.devices,
        transcripts,
        now: Utc::now(),
    };
    zeron_doc::tree::tree_rows(&ctx, root_chat_id, true)
        .into_iter()
        .map(|node| {
            let stats = node.chat_id.as_deref().and_then(|id| stats.get(id));
            OutlineRow {
                tokens: tree::tokens_of(stats).map(tree::tokens_text),
                time: tree::thread_time(node.status, node.since, stats),
                node,
            }
        })
        .collect()
}
