use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use zeron_doc::{MessagePart, SessionMessageEntry, SubagentStatus};
use zeron_proto::{Chat, ChatIndicator, DelegationStatus, LinkKind};

use super::tree::Status;
use super::{Brand, chat_brand, rollup, task_led_by};
use crate::state::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NodeKind {
    Root,
    Step(usize),
    Agent,
    Native,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Group {
    Steps(usize),
    Agents(usize),
}

#[derive(Debug, Clone)]
pub(crate) struct TreeNode {
    pub id: String,
    pub parent: Option<usize>,
    pub depth: usize,
    pub kind: NodeKind,
    pub group: Option<Group>,
    pub title: String,
    pub status: Status,
    pub word: &'static str,
    pub brand: Option<Brand>,
    pub chat_id: Option<String>,
    pub step_id: Option<String>,
    pub since: Option<DateTime<Utc>>,
}

struct Walk<'a> {
    state: &'a AppState,
    children: HashMap<&'a str, Vec<&'a Chat>>,
    native: bool,
    seen: HashSet<String>,
    now: DateTime<Utc>,
    out: Vec<TreeNode>,
}

pub(crate) fn tree_rows(
    state: &AppState,
    root_chat_id: &str,
    native: bool,
    now: DateTime<Utc>,
) -> Vec<TreeNode> {
    let Some(root) = state.chats.iter().find(|chat| chat.id == root_chat_id) else {
        return Vec::new();
    };
    let mut walk = Walk {
        state,
        children: children_by_parent(state),
        native,
        seen: HashSet::from([root.id.clone()]),
        now,
        out: Vec::new(),
    };
    let plan = task_led_by(&state.tasks, &root.id);
    let rollup = plan.map(|task| (task, rollup(state, task, now)));
    let (status, word) = match &rollup {
        Some((task, plan)) => super::tree::lead_status(task, plan),
        None => chat_status(state, root, now),
    };
    let title = match &rollup {
        Some((task, plan)) => super::TaskCenter::lead_line(task, plan),
        None => String::new(),
    };
    walk.out.push(TreeNode {
        id: root.id.clone(),
        parent: None,
        depth: 0,
        kind: NodeKind::Root,
        group: None,
        title,
        status,
        word,
        brand: Some(chat_brand(root)),
        chat_id: Some(root.id.clone()),
        step_id: None,
        since: since(state, root),
    });
    let mut bound: HashSet<&str> = HashSet::new();
    if let Some((_, plan)) = &rollup {
        let total = plan.steps.len();
        for (n, row) in plan.steps.iter().enumerate() {
            let chat = super::step_chat(&state.chats, &row.step);
            if let Some(chat) = chat {
                bound.insert(chat.id.as_str());
                walk.seen.insert(chat.id.clone());
            }
            let delegation = chat.and_then(|chat| chat.delegation.as_ref().map(|d| d.status));
            let (status, word) = Status::of_step(row, delegation);
            let ix = walk.out.len();
            walk.out.push(TreeNode {
                id: row.step.id.clone(),
                parent: Some(0),
                depth: 1,
                kind: NodeKind::Step(n + 1),
                group: (n == 0).then_some(Group::Steps(total)),
                title: crate::transcript::single_line(&row.step.title),
                status,
                word,
                brand: row.brand,
                chat_id: chat.map(|chat| chat.id.clone()),
                step_id: Some(row.step.id.clone()),
                since: chat.and_then(|chat| since(state, chat)),
            });
            if let Some(chat) = chat {
                walk.descend(chat, ix, 1);
            }
        }
    }
    let agents: Vec<&Chat> = walk
        .children
        .get(root.id.as_str())
        .into_iter()
        .flatten()
        .copied()
        .filter(|chat| !bound.contains(chat.id.as_str()))
        .collect();
    let first = walk.out.len();
    for chat in agents {
        walk.agent(chat, 0, 1);
    }
    walk.natives(root, 0, 1);
    let count = walk.out[first..]
        .iter()
        .filter(|node| node.parent == Some(0))
        .count();
    if count > 0
        && rollup
            .as_ref()
            .is_some_and(|(task, plan)| super::is_mission(task, plan))
    {
        walk.out[first].group = Some(Group::Agents(count));
    }
    if rollup.is_none() {
        walk.out[0].title = root_line(&walk.out);
    }
    walk.out
}

impl<'a> Walk<'a> {
    fn agent(&mut self, chat: &'a Chat, parent: usize, depth: usize) -> Option<usize> {
        if !self.seen.insert(chat.id.clone()) {
            return None;
        }
        let (status, word) = chat_status(self.state, chat, self.now);
        let ix = self.out.len();
        self.out.push(TreeNode {
            id: chat.id.clone(),
            parent: Some(parent),
            depth,
            kind: NodeKind::Agent,
            group: None,
            title: crate::transcript::single_line(&crate::home::chat_title(chat)),
            status,
            word,
            brand: Some(chat_brand(chat)),
            chat_id: Some(chat.id.clone()),
            step_id: None,
            since: since(self.state, chat),
        });
        self.descend(chat, ix, depth);
        Some(ix)
    }

    fn descend(&mut self, chat: &'a Chat, ix: usize, depth: usize) {
        let kids: Vec<&Chat> = self
            .children
            .get(chat.id.as_str())
            .into_iter()
            .flatten()
            .copied()
            .collect();
        for kid in kids {
            self.agent(kid, ix, depth + 1);
        }
        self.natives(chat, ix, depth + 1);
    }

    fn natives(&mut self, chat: &Chat, parent: usize, depth: usize) {
        if !self.native {
            return;
        }
        let entries: &[SessionMessageEntry] =
            if self.state.selected_chat.as_deref() == Some(chat.id.as_str()) {
                &self.state.transcript
            } else {
                self.state.sub_transcript(&chat.id)
            };
        let harness = chat.config.as_ref().map(|config| config.harness);
        for part in entries.iter().flat_map(|entry| &entry.parts) {
            let MessagePart::Tool {
                call,
                subagent_ref: Some(doc_id),
                subagent_status,
                ..
            } = part
            else {
                continue;
            };
            if !call.is_subagent_spawn() || !self.seen.insert(doc_id.clone()) {
                continue;
            }
            let (status, word) = match subagent_status {
                Some(SubagentStatus::Done) => (Status::Done, "Done"),
                Some(SubagentStatus::Failed) => (Status::Failed, "Failed"),
                Some(SubagentStatus::Stopped) => (Status::Stopped, "Stopped"),
                _ => (Status::Running, "Working"),
            };
            let model = call.subagent_model();
            let started = self
                .state
                .sub_transcript(doc_id)
                .first()
                .and_then(|entry| DateTime::from_timestamp_millis(entry.created_at));
            self.out.push(TreeNode {
                id: doc_id.clone(),
                parent: Some(parent),
                depth,
                kind: NodeKind::Native,
                group: None,
                title: crate::transcript::subagent_tab_title(call).to_string(),
                status,
                word,
                brand: Some(crate::pickers::agent_brand_icon(harness, model)),
                chat_id: None,
                step_id: None,
                since: started,
            });
        }
    }
}

pub(super) fn children_by_parent(state: &AppState) -> HashMap<&str, Vec<&Chat>> {
    let ids: HashSet<&str> = state.chats.iter().map(|chat| chat.id.as_str()).collect();
    let mut children: HashMap<&str, Vec<&Chat>> = HashMap::new();
    for chat in state.chats.iter().filter(|chat| !chat.archived) {
        let Some(link) = zeron_proto::thread_link(chat, &state.tasks, &state.task_steps) else {
            continue;
        };
        if !matches!(link.kind, LinkKind::Delegated | LinkKind::OpenedFrom) {
            continue;
        }
        let Some(&parent) = ids.get(link.parent.as_str()) else {
            continue;
        };
        children.entry(parent).or_default().push(chat);
    }
    for kids in children.values_mut() {
        kids.sort_by_key(|chat| chat.created_at);
    }
    children
}

fn since(state: &AppState, chat: &Chat) -> Option<DateTime<Utc>> {
    state
        .session_for(&chat.id)
        .and_then(|session| session.started_at)
        .or(chat.delegation.as_ref().map(|d| d.started_at))
}

pub(crate) fn chat_status(
    state: &AppState,
    chat: &Chat,
    now: DateTime<Utc>,
) -> (Status, &'static str) {
    match state.display_status_for(chat, now) {
        ChatIndicator::Working => return (Status::Running, "Working"),
        ChatIndicator::AwaitingInput => return (Status::Waiting, "Needs you"),
        ChatIndicator::Errored => return (Status::Failed, "Failed"),
        ChatIndicator::Completed | ChatIndicator::Idle => {}
    }
    if state.running_subagents_for(&chat.id, now) > 0 {
        return (Status::Running, "Working");
    }
    match chat.delegation.as_ref().map(|d| d.status) {
        Some(DelegationStatus::Failed) => (Status::Failed, "Failed"),
        Some(DelegationStatus::Interrupted) => (Status::Failed, "Interrupted"),
        Some(DelegationStatus::Cancelled) => (Status::Failed, "Stopped"),
        _ if chat.last_message_at.is_none() => (Status::Queued, "Not started"),
        _ if chat.delegated_by().is_some() || chat.created_by_chat_id.is_some() => {
            (Status::Done, "Done")
        }
        _ => (Status::Queued, "Idle"),
    }
}

fn root_line(nodes: &[TreeNode]) -> String {
    let root = &nodes[0];
    let working = nodes[1..]
        .iter()
        .filter(|node| node.parent == Some(0) && node.status.live())
        .count();
    match root.status {
        Status::Waiting => "Needs you".into(),
        Status::Failed => "Failed".into(),
        _ if working > 0 => format!(
            "Watching {working} {}",
            if working == 1 { "agent" } else { "agents" }
        ),
        Status::Running => "Working".into(),
        _ => "Idle".into(),
    }
}
