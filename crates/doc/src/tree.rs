use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Local, Utc};
use zeron_proto::mission::{PlanPhase, Rollup, StepRow, is_mission, plan_led_by, step_chat};
use zeron_proto::today::plural;
use zeron_proto::usage::format_tokens;
use zeron_proto::view::single_line;
use zeron_proto::{
    Chat, ChatIndicator, DelegationStatus, HarnessId, LinkKind, SessionStats, StepStatus, Task,
    TaskStep,
};

use crate::{MessagePart, SessionMessageEntry, SubagentStatus};

pub const TREE_VIEWS: [&str; 3] = ["Spine", "Outline", "Timeline"];
pub const BADGE: &str = "Tree";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TreeStatus {
    Queued,
    Running,
    Waiting,
    Done,
    Failed,
    Stopped,
}

impl TreeStatus {
    pub fn of_step<B>(
        row: &StepRow<B>,
        delegation: Option<DelegationStatus>,
    ) -> (Self, &'static str) {
        match row.status {
            StepStatus::Done => (Self::Done, "Done"),
            StepStatus::Skipped => (Self::Stopped, "Skipped"),
            StepStatus::Planned => (Self::Queued, "Planned"),
            StepStatus::Working => (Self::Running, "Working"),
            StepStatus::Failed => (Self::Failed, "Failed"),
            StepStatus::NeedsYou => {
                let stopped = delegation.is_none()
                    && row.step.turn.as_ref().map(|turn| turn.ended)
                        == Some(zeron_proto::StepTurnEnd::Stopped);
                match delegation {
                    _ if row.indicator == Some(ChatIndicator::AwaitingInput) => {
                        (Self::Waiting, "Needs you")
                    }
                    Some(DelegationStatus::Cancelled) => (Self::Failed, "Stopped"),
                    Some(DelegationStatus::Interrupted) => (Self::Failed, "Interrupted"),
                    Some(DelegationStatus::Failed) => (Self::Failed, "Failed"),
                    _ if stopped => (Self::Failed, "Stopped"),
                    _ if row.indicator == Some(ChatIndicator::Errored) => (Self::Failed, "Failed"),
                    _ => (Self::Waiting, "Needs you"),
                }
            }
        }
    }

    pub fn muted(self) -> bool {
        matches!(self, Self::Queued | Self::Stopped)
    }

    pub fn live(self) -> bool {
        matches!(self, Self::Running | Self::Waiting)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Root,
    Step(usize),
    Agent,
    Native,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Steps(usize),
    Agents(usize),
}

impl Group {
    pub fn label(self) -> String {
        match self {
            Self::Steps(n) => format!("Steps \u{a0}{n}"),
            Self::Agents(n) => format!("Outside the plan \u{a0}{n}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TreeNode<B> {
    pub id: String,
    pub parent: Option<usize>,
    pub depth: usize,
    pub kind: NodeKind,
    pub group: Option<Group>,
    pub title: String,
    pub status: TreeStatus,
    pub word: &'static str,
    pub brand: Option<B>,
    pub chat_id: Option<String>,
    pub step_id: Option<String>,
    pub since: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct IndexRow {
    pub root: String,
    pub title: String,
    pub status: TreeStatus,
    pub line: String,
    pub asks: bool,
    pub segments: Vec<TreeStatus>,
    pub started: DateTime<Utc>,
    pub ended: Option<DateTime<Utc>>,
}

impl IndexRow {
    pub fn agents_line(&self) -> Option<String> {
        let finished = self
            .segments
            .iter()
            .filter(|status| **status == TreeStatus::Done)
            .count();
        (!self.segments.is_empty()).then(|| format!("{finished}/{} agents", self.segments.len()))
    }
}

#[derive(Debug, Default)]
pub struct IndexRows {
    pub running: Vec<IndexRow>,
    pub done_today: Vec<IndexRow>,
}

pub trait TreeSource {
    type Brand: Clone;
    fn chats(&self) -> &[Chat];
    fn tasks(&self) -> &[Task];
    fn steps(&self) -> &[TaskStep];
    fn now(&self) -> DateTime<Utc>;
    fn indicator(&self, chat: &Chat) -> ChatIndicator;
    fn running_subagents(&self, chat_id: &str) -> usize;
    fn started_at(&self, chat_id: &str) -> Option<DateTime<Utc>>;
    fn hidden(&self, chat_id: &str) -> bool;
    fn brand(&self, harness: Option<HarnessId>, model: Option<&str>) -> Self::Brand;
    fn rollup(&self, task: &Task) -> Rollup<Self::Brand>;
    fn transcript(&self, chat_id: &str) -> Box<dyn Iterator<Item = &SessionMessageEntry> + '_>;

    fn chat_brand(&self, chat: &Chat) -> Self::Brand {
        let config = chat.config.as_ref();
        self.brand(
            config.map(|config| config.harness),
            config.and_then(|config| config.model.as_deref()),
        )
    }
}

pub fn fmt_time(ms: u64) -> String {
    let secs = ms / 1000;
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m {:02}s", secs / 60, secs % 60),
        _ => format!("{}h {:02}m", secs / 3600, secs % 3600 / 60),
    }
}

pub fn thread_time(
    status: TreeStatus,
    since: Option<DateTime<Utc>>,
    stats: Option<&SessionStats>,
) -> Option<String> {
    if status.live()
        && let Some(since) = since
    {
        let ms = (Utc::now() - since).num_milliseconds().max(0) as u64;
        return Some(fmt_time(ms));
    }
    let stats = stats?;
    let ms = stats.llm_ms + stats.tool_ms;
    (ms > 0).then(|| fmt_time(ms))
}

pub fn tokens_of(stats: Option<&SessionStats>) -> Option<u64> {
    stats
        .map(SessionStats::total_tokens)
        .filter(|tokens| *tokens > 0)
}

pub fn tokens_text(tokens: u64) -> String {
    format!("{} tokens", format_tokens(tokens))
}

pub fn linked_label(indicator: ChatIndicator) -> &'static str {
    match indicator {
        ChatIndicator::Working => "Working",
        ChatIndicator::AwaitingInput => "Asked you a question",
        ChatIndicator::Errored => "Failed",
        ChatIndicator::Completed | ChatIndicator::Idle => "Ready for you",
    }
}

/// A plan first, else the task handed to this chat, open before done.
pub fn task_led_by<'a>(tasks: &'a [Task], chat_id: &str) -> Option<&'a Task> {
    plan_led_by(tasks, chat_id).or_else(|| {
        tasks
            .iter()
            .filter(|task| !task.deleted && task.chat_id.as_deref() == Some(chat_id))
            .max_by_key(|task| (task.completed_at.is_none(), task.completed_at))
    })
}

pub fn lead_waits<B>(task: &Task, plan: &Rollup<B>) -> Option<String> {
    let mut waits = Vec::new();
    if plan.phase(task) == PlanPhase::PlanReady {
        waits.push("Plan review".to_owned());
    }
    let questions = plan
        .questions
        .max(usize::from(plan.lead == Some(ChatIndicator::AwaitingInput)));
    if questions > 0 {
        waits.push(format!(
            "{questions} {}",
            plural(questions, "question", "questions")
        ));
    }
    (!waits.is_empty()).then(|| waits.join(", "))
}

pub fn lead_line<B>(task: &Task, plan: &Rollup<B>) -> String {
    let running = plan.count(StepStatus::Working);
    let working = plan.lead == Some(ChatIndicator::Working);
    if task.completed_at.is_some() {
        "Done".into()
    } else if plan.lead.is_none() {
        "Not started".into()
    } else if let Some(lead) = plan.lead.filter(|_| !is_mission(task, plan)) {
        linked_label(lead).into()
    } else if plan.phase(task) == PlanPhase::Planning {
        "Planning".into()
    } else if plan.lead == Some(ChatIndicator::Errored) {
        "Failed".into()
    } else if let Some(waits) = lead_waits(task, plan).filter(|_| !working) {
        waits
    } else if running > 0 {
        format!("Watching {running} {}", plural(running, "step", "steps"))
    } else if working {
        "Working".into()
    } else {
        "Idle".into()
    }
}

pub fn lead_status<B>(task: &Task, plan: &Rollup<B>) -> (TreeStatus, &'static str) {
    if task.completed_at.is_some() {
        return (TreeStatus::Done, "Done");
    }
    if let Some(indicator) = plan.lead.filter(|_| !is_mission(task, plan)) {
        let status = match indicator {
            ChatIndicator::Working => TreeStatus::Running,
            ChatIndicator::Errored => TreeStatus::Failed,
            ChatIndicator::AwaitingInput | ChatIndicator::Completed | ChatIndicator::Idle => {
                TreeStatus::Waiting
            }
        };
        return (status, linked_label(indicator));
    }
    match plan.lead {
        None => (TreeStatus::Queued, "Not started"),
        _ if plan.phase(task) == PlanPhase::Planning => (TreeStatus::Running, "Planning"),
        Some(ChatIndicator::Working) => (TreeStatus::Running, "Working"),
        Some(ChatIndicator::Errored) => (TreeStatus::Failed, "Failed"),
        _ if lead_waits(task, plan).is_some() => (TreeStatus::Waiting, "Needs you"),
        _ if plan.count(StepStatus::Working) > 0 => (TreeStatus::Running, "Working"),
        _ => (TreeStatus::Queued, "Idle"),
    }
}

pub fn chat_status<S: TreeSource>(source: &S, chat: &Chat) -> (TreeStatus, &'static str) {
    match source.indicator(chat) {
        ChatIndicator::Working => return (TreeStatus::Running, "Working"),
        ChatIndicator::AwaitingInput => return (TreeStatus::Waiting, "Needs you"),
        ChatIndicator::Errored => return (TreeStatus::Failed, "Failed"),
        ChatIndicator::Completed | ChatIndicator::Idle => {}
    }
    if source.running_subagents(&chat.id) > 0 {
        return (TreeStatus::Running, "Working");
    }
    match chat.delegation.as_ref().map(|d| d.status) {
        Some(DelegationStatus::Failed) => (TreeStatus::Failed, "Failed"),
        Some(DelegationStatus::Interrupted) => (TreeStatus::Failed, "Interrupted"),
        Some(DelegationStatus::Cancelled) => (TreeStatus::Failed, "Stopped"),
        _ if chat.last_message_at.is_none() => (TreeStatus::Queued, "Not started"),
        _ if chat.delegated_by().is_some() || chat.created_by_chat_id.is_some() => {
            (TreeStatus::Done, "Done")
        }
        _ => (TreeStatus::Queued, "Idle"),
    }
}

pub fn children_by_parent<'a>(
    chats: &'a [Chat],
    tasks: &[Task],
    steps: &[TaskStep],
) -> HashMap<&'a str, Vec<&'a Chat>> {
    let ids: HashSet<&str> = chats.iter().map(|chat| chat.id.as_str()).collect();
    let mut children: HashMap<&str, Vec<&Chat>> = HashMap::new();
    for chat in chats.iter().filter(|chat| !chat.archived) {
        let Some(link) = zeron_proto::thread_link(chat, tasks, steps) else {
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

struct Walk<'a, S: TreeSource> {
    source: &'a S,
    children: HashMap<&'a str, Vec<&'a Chat>>,
    native: bool,
    seen: HashSet<String>,
    out: Vec<TreeNode<S::Brand>>,
}

pub fn tree_rows<S: TreeSource>(
    source: &S,
    root_chat_id: &str,
    native: bool,
) -> Vec<TreeNode<S::Brand>> {
    let Some(root) = source.chats().iter().find(|chat| chat.id == root_chat_id) else {
        return Vec::new();
    };
    let mut walk = Walk {
        source,
        children: children_by_parent(source.chats(), source.tasks(), source.steps()),
        native,
        seen: HashSet::from([root.id.clone()]),
        out: Vec::new(),
    };
    let plan = task_led_by(source.tasks(), &root.id);
    let rollup = plan.map(|task| (task, source.rollup(task)));
    let (status, word) = match &rollup {
        Some((task, plan)) => lead_status(task, plan),
        None => chat_status(source, root),
    };
    let title = match &rollup {
        Some((task, plan)) => lead_line(task, plan),
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
        brand: Some(source.chat_brand(root)),
        chat_id: Some(root.id.clone()),
        step_id: None,
        since: since(source, root),
    });
    let mut bound: HashSet<&str> = HashSet::new();
    if let Some((_, plan)) = &rollup {
        let total = plan.steps.len();
        for (n, row) in plan.steps.iter().enumerate() {
            let chat = step_chat(source.chats(), &row.step);
            if let Some(chat) = chat {
                bound.insert(chat.id.as_str());
                walk.seen.insert(chat.id.clone());
            }
            let delegation = chat.and_then(|chat| chat.delegation.as_ref().map(|d| d.status));
            let (status, word) = TreeStatus::of_step(row, delegation);
            let ix = walk.out.len();
            walk.out.push(TreeNode {
                id: row.step.id.clone(),
                parent: Some(0),
                depth: 1,
                kind: NodeKind::Step(n + 1),
                group: (n == 0).then_some(Group::Steps(total)),
                title: single_line(&row.step.title),
                status,
                word,
                brand: row.brand.clone(),
                chat_id: chat.map(|chat| chat.id.clone()),
                step_id: Some(row.step.id.clone()),
                since: chat.and_then(|chat| since(source, chat)),
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
            .is_some_and(|(task, plan)| is_mission(task, plan))
    {
        walk.out[first].group = Some(Group::Agents(count));
    }
    if rollup.is_none() {
        walk.out[0].title = root_line(&walk.out);
    }
    walk.out
}

impl<'a, S: TreeSource> Walk<'a, S> {
    fn agent(&mut self, chat: &'a Chat, parent: usize, depth: usize) -> Option<usize> {
        if !self.seen.insert(chat.id.clone()) {
            return None;
        }
        let (status, word) = chat_status(self.source, chat);
        let ix = self.out.len();
        self.out.push(TreeNode {
            id: chat.id.clone(),
            parent: Some(parent),
            depth,
            kind: NodeKind::Agent,
            group: None,
            title: single_line(&chat.display_title()),
            status,
            word,
            brand: Some(self.source.chat_brand(chat)),
            chat_id: Some(chat.id.clone()),
            step_id: None,
            since: since(self.source, chat),
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
        let harness = chat.config.as_ref().map(|config| config.harness);
        for part in self
            .source
            .transcript(&chat.id)
            .flat_map(|entry| &entry.parts)
        {
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
                Some(SubagentStatus::Done) => (TreeStatus::Done, "Done"),
                Some(SubagentStatus::Failed) => (TreeStatus::Failed, "Failed"),
                Some(SubagentStatus::Stopped) => (TreeStatus::Stopped, "Stopped"),
                _ => (TreeStatus::Running, "Working"),
            };
            let started = self
                .source
                .transcript(doc_id)
                .next()
                .and_then(|entry| DateTime::from_timestamp_millis(entry.created_at));
            self.out.push(TreeNode {
                id: doc_id.clone(),
                parent: Some(parent),
                depth,
                kind: NodeKind::Native,
                group: None,
                title: call.subagent_title(),
                status,
                word,
                brand: Some(self.source.brand(harness, call.subagent_model())),
                chat_id: None,
                step_id: None,
                since: started,
            });
        }
    }
}

fn since<S: TreeSource>(source: &S, chat: &Chat) -> Option<DateTime<Utc>> {
    source
        .started_at(&chat.id)
        .or(chat.delegation.as_ref().map(|d| d.started_at))
}

fn root_line<B>(nodes: &[TreeNode<B>]) -> String {
    let root = &nodes[0];
    let working = nodes[1..]
        .iter()
        .filter(|node| node.parent == Some(0) && node.status.live())
        .count();
    match root.status {
        TreeStatus::Waiting => "Needs you".into(),
        TreeStatus::Failed => "Failed".into(),
        _ if working > 0 => format!("Watching {working} {}", plural(working, "agent", "agents")),
        TreeStatus::Running => "Working".into(),
        _ => "Idle".into(),
    }
}

fn is_today(at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    at.with_timezone(&Local).date_naive() == now.with_timezone(&Local).date_naive()
}

fn roots_a_tree<S: TreeSource>(
    source: &S,
    chat: &Chat,
    children: &HashMap<&str, Vec<&Chat>>,
    nested: &HashSet<&str>,
) -> bool {
    if chat.archived || chat.is_system() || source.hidden(&chat.id) {
        return false;
    }
    !nested.contains(chat.id.as_str())
        && children.contains_key(chat.id.as_str())
        && task_led_by(source.tasks(), &chat.id).is_none()
}

pub fn tree_index<S: TreeSource>(source: &S) -> IndexRows {
    let now = source.now();
    let children = children_by_parent(source.chats(), source.tasks(), source.steps());
    let nested: HashSet<&str> = children
        .iter()
        .flat_map(|(parent, kids)| {
            kids.iter()
                .filter(move |kid| kid.id != *parent)
                .map(|kid| kid.id.as_str())
        })
        .collect();
    let mut rows = IndexRows::default();
    for chat in source.chats() {
        if !roots_a_tree(source, chat, &children, &nested) {
            continue;
        }
        let nodes = tree_rows(source, &chat.id, false);
        if nodes.is_empty() {
            continue;
        }
        let live = nodes.iter().any(|node| node.status.live());
        let last = nodes
            .iter()
            .filter_map(|node| node.chat_id.as_deref())
            .filter_map(|id| source.chats().iter().find(|chat| chat.id == id))
            .filter_map(|chat| chat.last_message_at)
            .max();
        let settled_today = chat
            .settle
            .filter(|settle| {
                settle.settled && chat.last_message_at.is_none_or(|at| at <= settle.at)
            })
            .map(|settle| settle.at);
        let done = match settled_today {
            Some(at) if is_today(at, now) => Some(at),
            Some(_) => continue,
            None if live => None,
            None => match last {
                Some(at) if is_today(at, now) => Some(at),
                _ => continue,
            },
        };
        let row = index_row(source, chat, &nodes, done);
        if done.is_some() {
            rows.done_today.push(row);
        } else {
            rows.running.push(row);
        }
    }
    let latest = |row: &IndexRow| std::cmp::Reverse(row.ended.unwrap_or(row.started));
    rows.running.sort_by_key(latest);
    rows.done_today.sort_by_key(latest);
    rows
}

fn index_row<S: TreeSource>(
    source: &S,
    chat: &Chat,
    nodes: &[TreeNode<S::Brand>],
    done: Option<DateTime<Utc>>,
) -> IndexRow {
    let asker = nodes
        .iter()
        .find(|node| node.status == TreeStatus::Waiting && node.kind != NodeKind::Root);
    let root_waits = nodes[0].status == TreeStatus::Waiting;
    let agents_running = nodes
        .iter()
        .filter(|node| {
            matches!(node.kind, NodeKind::Agent | NodeKind::Native)
                && node.status == TreeStatus::Running
        })
        .count();
    let line = if let Some(asker) = asker {
        format!("Needs you \u{b7} {} asks", asker.title)
    } else if root_waits {
        format!("Needs you \u{b7} {}", nodes[0].title)
    } else if done.is_some() {
        "Done".into()
    } else if agents_running > 0 {
        format!(
            "Watching {agents_running} {}",
            plural(agents_running, "agent", "agents")
        )
    } else if nodes[0].status == TreeStatus::Running {
        "Working".into()
    } else {
        "Idle".into()
    };
    let status = if done.is_some() {
        TreeStatus::Done
    } else if asker.is_some() || root_waits {
        TreeStatus::Waiting
    } else if nodes[0].status == TreeStatus::Done && nodes.iter().any(|node| node.status.live()) {
        TreeStatus::Running
    } else {
        nodes[0].status
    };
    let started = nodes[1..]
        .iter()
        .filter_map(|node| node.chat_id.as_deref())
        .filter_map(|id| source.chats().iter().find(|chat| chat.id == id))
        .map(|chat| chat.created_at)
        .min()
        .unwrap_or(chat.created_at);
    IndexRow {
        root: chat.id.clone(),
        title: single_line(&chat.display_title()),
        status,
        line,
        asks: asker.is_some() || root_waits,
        segments: nodes[1..].iter().map(|node| node.status).collect(),
        started,
        ended: done,
    }
}
