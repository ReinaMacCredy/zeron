use std::collections::HashSet;
use std::time::Duration;

use gpui::{
    Action, Animation, AnimationExt, AnyElement, App, Context, Entity, EntityInputHandler,
    FocusHandle, Focusable, KeyDownEvent, ListAlignment, ListState, SharedString, Subscription,
    Task, Window, div, list, percentage, prelude::*, px,
};
use zeron_proto::{
    CheckoutDiff, CommitSummary, DiffFileSummary, GenerateCommitMessageRequest,
    GeneratedCommitMessage, GetWorkingTreeFileDiffRequest, GetWorkingTreeRequest,
    GitAgentBusyPolicy, GitFileState, GitHistoryCommit, GitHistoryRefKind,
    RunWorkingTreeActionRequest, SessionStatus, SyncBranchReply, SyncBranchRequest, SyncDirection,
    WorkingTree, WorkingTreeAction, WorkingTreeActionReply, WorkingTreeFile, WorkingTreeFileDiff,
    WorkspaceTarget,
};
use zeron_rpc::{RpcError, methods};

use crate::changes::{self, FileDiff};
use crate::composer::{ComposerInput, ComposerInputEvent};
use crate::history::{GitHistory, GitHistoryEvent, GitHistoryTarget};
use crate::icons::{self, icon};
use crate::pickers::{CommitModelPicked, Pickers};
use crate::popover::{self, Loadable};
use crate::settings::CommitModelChoice;
use crate::settings::widgets::text_tooltip;
use crate::state::AppState;
use crate::theme::Theme;

#[derive(Clone, PartialEq, Action)]
#[action(namespace = pull_requests, no_json)]
pub struct OpenSourceControlSettings;

const POLL_INTERVAL: Duration = Duration::from_secs(4);
const WATCH_RETRY: Duration = Duration::from_secs(2);
const COMMIT_FILE_ROW_HEIGHT: f32 = 26.0;
const HISTORY_KEEP_WIDTH: f32 = 480.0;
const SUBJECT_LIMIT: usize = 72;
const MESSAGE_MIN_HEIGHT: f32 = 44.0;
const TRAY_MAX_HEIGHT: f32 = 220.0;
const DIFF_LINE_LIMIT: usize = 400;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GitSpace {
    pub space_id: String,
    pub path: String,
    pub device: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CheckoutOption {
    pub path: Option<String>,
    pub label: SharedString,
    pub detail: SharedString,
    pub live: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SyncKind {
    Push,
    Pull,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SyncOutcome {
    pub kind: SyncKind,
    pub text: SharedString,
    pub failed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingTarget {
    File { staged: bool, path: String },
    All { staged: bool },
    Commit,
    Stash,
    Detail,
    AbortPick,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Confirm {
    Discard { path: String, revision: String },
    DropStash { oid: String },
    Checkout { sha: String },
    CherryPick { sha: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommitKind {
    Staged,
    Amend,
    All,
}

impl CommitKind {
    fn action(self, message: String) -> WorkingTreeAction {
        WorkingTreeAction::Commit {
            message,
            amend: self == Self::Amend,
            all: self == Self::All,
        }
    }
}

struct Pending {
    target: PendingTarget,
    replied: bool,
}

struct Expanded {
    staged: bool,
    path: String,
    view: Loadable<(WorkingTreeFileDiff, Option<FileDiff>)>,
    rows: Vec<DiffRow>,
    list: ListState,
}

impl Expanded {
    fn new(staged: bool, path: String) -> Self {
        Self {
            staged,
            path,
            view: Loadable::Loading,
            rows: vec![DiffRow::Loading],
            list: ListState::new(1, ListAlignment::Top, px(1024.0)),
        }
    }

    fn set_view(&mut self, view: Loadable<(WorkingTreeFileDiff, Option<FileDiff>)>) {
        self.view = view;
        let rows = match &self.view {
            Loadable::Idle | Loadable::Loading => vec![DiffRow::Loading],
            Loadable::Error(_) => vec![DiffRow::Failed],
            Loadable::Ready((diff, parsed)) => {
                diff_rows(parsed.as_ref(), diff.binary, diff.truncated)
            }
        };
        splice_rows(&self.list, &mut self.rows, rows);
    }
}

struct BranchForm {
    name: Entity<ComposerInput>,
    checkout: bool,
    _edits: Subscription,
}

struct CommitDetail {
    commit: GitHistoryCommit,
    diff: Loadable<(CheckoutDiff, Vec<FileDiff>)>,
    open_files: HashSet<String>,
    branch_form: Option<BranchForm>,
    outcome: Option<(SharedString, bool)>,
    rows: Vec<DetailRow>,
    list: ListState,
}

impl CommitDetail {
    fn new(commit: GitHistoryCommit) -> Self {
        Self {
            commit,
            diff: Loadable::Loading,
            open_files: HashSet::new(),
            branch_form: None,
            outcome: None,
            rows: vec![DetailRow::Header, DetailRow::Status],
            list: ListState::new(2, ListAlignment::Top, px(1024.0)),
        }
    }

    fn sync_rows(&mut self) {
        let mut rows = vec![DetailRow::Header];
        match &self.diff {
            Loadable::Ready((diff, parsed)) if !diff.files.is_empty() => {
                for (index, file) in diff.files.iter().enumerate() {
                    rows.push(DetailRow::File(index));
                    if !self.open_files.contains(&file.path) {
                        continue;
                    }
                    let position = parsed.iter().position(|parsed| parsed.path == file.path);
                    let last = position.is_some_and(|at| at + 1 == parsed.len());
                    rows.extend(
                        diff_rows(
                            position.map(|at| &parsed[at]),
                            file.binary,
                            diff.truncated && (position.is_none() || last),
                        )
                        .into_iter()
                        .map(|row| DetailRow::Diff(index, row)),
                    );
                }
            }
            _ => rows.push(DetailRow::Status),
        }
        splice_rows(&self.list, &mut self.rows, rows);
    }
}

#[derive(Clone, Copy, PartialEq)]
enum DetailRow {
    Header,
    Status,
    File(usize),
    Diff(usize, DiffRow),
}

#[derive(Clone, Copy, PartialEq)]
enum DiffRow {
    Loading,
    Failed,
    Hunk(usize),
    Line(usize, usize),
    Hidden(usize),
    Binary,
    Truncated,
    NoText,
}

struct CommitPanelResize;

struct CommitPanelResizeGhost;

impl Render for CommitPanelResizeGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

pub struct GitTab {
    state: Entity<AppState>,
    history: Entity<GitHistory>,
    space: Option<GitSpace>,
    checkout_path: Option<String>,
    tree: Option<WorkingTree>,
    error: Option<SharedString>,
    reload_wanted: bool,
    load_task: Option<Task<()>>,
    watch_task: Option<Task<()>>,
    _poll_task: Task<()>,
    visible: bool,
    window_active: bool,
    activation: Option<Subscription>,
    pending: Option<Pending>,
    action_error: Option<SharedString>,
    action_task: Option<Task<()>>,
    expanded: Option<Expanded>,
    diff_task: Option<Task<()>>,
    message: Entity<ComposerInput>,
    commit_menu: bool,
    confirm: Option<Confirm>,
    confirm_focus: FocusHandle,
    stash_menu: bool,
    stash_note: Option<SharedString>,
    last_commit: Option<CommitSummary>,
    write_task: Option<Task<()>>,
    write_focus: FocusHandle,
    written: Option<GeneratedCommitMessage>,
    write_error: Option<SharedString>,
    model_override: Option<CommitModelChoice>,
    shown_default: Option<CommitModelChoice>,
    model_picker: Entity<Pickers>,
    syncing: Option<SyncKind>,
    sync_outcome: Option<SyncOutcome>,
    sync_task: Option<Task<()>>,
    checkout_menu: bool,
    sync_menu: Option<SyncKind>,
    detail: Option<CommitDetail>,
    detail_task: Option<Task<()>>,
    copy_task: Option<Task<()>>,
    _model_picks: Subscription,
    _settings_changes: Subscription,
    _message_edits: Subscription,
    _history_events: Subscription,
    compact: bool,
    toolbar_lead: Vec<AnyElement>,
    composer_open: bool,
    tray_open: bool,
    focus_message: bool,
    focus: FocusHandle,
    float: crate::dock_orb::Float,
}

fn workspace_target(space: &GitSpace, checkout_path: Option<&str>) -> WorkspaceTarget {
    WorkspaceTarget {
        chat_id: None,
        space_id: Some(space.space_id.clone()),
        checkout_path: checkout_path.map(str::to_owned),
    }
}

impl GitSpace {
    fn route(&self, request: impl serde::Serialize) -> serde_json::Value {
        let mut params = serde_json::to_value(request).unwrap_or_default();
        if let (Some(device), Some(map)) = (&self.device, params.as_object_mut()) {
            map.insert("targetDeviceId".into(), device.as_str().into());
        }
        params
    }
}

fn target_params(space: &GitSpace, checkout_path: Option<&str>) -> serde_json::Value {
    space.route(GetWorkingTreeRequest {
        target: workspace_target(space, checkout_path),
    })
}

fn decode<T: serde::de::DeserializeOwned>(
    reply: Result<serde_json::Value, RpcError>,
) -> Result<T, String> {
    reply
        .map_err(|error| error.to_string())
        .and_then(|value| serde_json::from_value(value).map_err(|error| error.to_string()))
}

fn push_direction(tree: &WorkingTree) -> SyncDirection {
    SyncDirection::Push {
        set_upstream: push_sets_upstream(tree),
    }
}

fn busy_policy(cx: &App) -> GitAgentBusyPolicy {
    crate::settings::git_agent_busy_policy(cx).into()
}

fn in_section(tree: &WorkingTree, staged: bool, path: &str) -> bool {
    let rows = if staged {
        staged_files(tree)
    } else {
        changed_files(tree)
    };
    rows.iter().any(|file| file.path == path)
}

fn agent_banner(policy: GitAgentBusyPolicy) -> &'static str {
    match policy {
        GitAgentBusyPolicy::SafeOnly => {
            "An agent is running in this worktree. Stage, unstage and commit of staged files still work; other Git actions wait until it stops."
        }
        GitAgentBusyPolicy::BlockAll => {
            "An agent is running in this worktree. Git is read-only here until it stops."
        }
        GitAgentBusyPolicy::AllowAll => {
            "An agent is running in this worktree. Git actions here can collide with its edits."
        }
    }
}

fn conflict_note(conflicts: &[String]) -> String {
    let shown: Vec<&str> = conflicts.iter().take(3).map(String::as_str).collect();
    let more = conflicts.len().saturating_sub(shown.len());
    let files = if more > 0 {
        format!("{} and {more} more", shown.join(", "))
    } else {
        shown.join(", ")
    };
    let count = if conflicts.len() == 1 {
        "1 file".to_string()
    } else {
        format!("{} files", conflicts.len())
    };
    format!(
        "The pop stopped on conflicts in {count} ({files}). The stash is kept; resolve the conflicts, then delete it."
    )
}

fn commit_count(count: u32) -> String {
    if count == 1 {
        "1 commit".to_string()
    } else {
        format!("{count} commits")
    }
}

fn publish_destination(tree: &WorkingTree) -> Option<String> {
    let (upstream, remote, branch) = (
        tree.upstream.as_deref()?,
        tree.push_remote.as_deref()?,
        tree.branch.as_deref()?,
    );
    let destination = format!("{remote}/{branch}");
    (upstream != destination).then_some(destination)
}

fn push_sets_upstream(tree: &WorkingTree) -> bool {
    tree.upstream.is_none() || publish_destination(tree).is_some()
}

fn sync_note(
    kind: SyncKind,
    branch: &str,
    destination: Option<&str>,
    reply: &SyncBranchReply,
) -> String {
    let upstream = destination
        .or(reply.upstream.as_deref())
        .unwrap_or("its upstream");
    match kind {
        SyncKind::Push if reply.commits == 0 => format!("Everything was already on {upstream}."),
        SyncKind::Push => format!("Pushed {} to {upstream}.", commit_count(reply.commits)),
        SyncKind::Pull if reply.commits == 0 => {
            format!("{branch} is already up to date with {upstream}.")
        }
        SyncKind::Pull => format!("Pulled {} into {branch}.", commit_count(reply.commits)),
    }
}

fn short_sha(sha: &str) -> String {
    sha.chars().take(7).collect()
}

fn checkout_target(commit: &GitHistoryCommit, tree: &WorkingTree) -> Option<(String, bool)> {
    let on_commit = tree.head_sha.as_deref() == Some(commit.sha.as_str());
    let branch = commit
        .refs
        .iter()
        .filter(|reference| reference.kind == GitHistoryRefKind::Branch)
        .map(|reference| reference.label.as_str())
        .find(|label| !on_commit || tree.branch.as_deref() != Some(*label));
    match branch {
        Some(branch) => Some((branch.to_owned(), false)),
        None if on_commit => None,
        None => Some((commit.sha.clone(), true)),
    }
}

fn clean_status(tree: &WorkingTree) -> String {
    let mut parts = vec![
        "Working tree clean".to_string(),
        tree.branch
            .clone()
            .unwrap_or_else(|| "detached HEAD".into()),
    ];
    if tree.ahead > 0 {
        parts.push(format!("{} ahead", tree.ahead));
    }
    if tree.behind > 0 {
        parts.push(format!("{} behind", tree.behind));
    }
    parts.join(" · ")
}

fn subject_chars(message: &str) -> usize {
    message.lines().next().unwrap_or_default().chars().count()
}

fn same_path(left: &str, right: &str) -> bool {
    left.trim_end_matches('/') == right.trim_end_matches('/')
}

fn status_letter(state: GitFileState) -> &'static str {
    match state {
        GitFileState::Added => "A",
        GitFileState::Modified => "M",
        GitFileState::Deleted => "D",
        GitFileState::Renamed => "R",
        GitFileState::Copied => "C",
        GitFileState::Unmerged => "!",
        GitFileState::Untracked => "U",
        GitFileState::TypeChanged => "T",
        GitFileState::Unchanged => " ",
    }
}

fn state_color(state: GitFileState, theme: &Theme) -> gpui::Hsla {
    match state {
        GitFileState::Added | GitFileState::Untracked => theme.success,
        GitFileState::Deleted | GitFileState::Unmerged => theme.danger,
        GitFileState::Renamed | GitFileState::Copied => theme.accent,
        _ => theme.warning,
    }
}

pub(super) fn staged_files(tree: &WorkingTree) -> Vec<&WorkingTreeFile> {
    tree.files.iter().filter(|file| file.is_staged()).collect()
}

pub(super) fn changed_files(tree: &WorkingTree) -> Vec<&WorkingTreeFile> {
    tree.files
        .iter()
        .filter(|file| file.worktree != GitFileState::Unchanged)
        .collect()
}

pub(super) fn checkout_options(state: &AppState, space: &GitSpace) -> Vec<CheckoutOption> {
    let space_device = state
        .spaces
        .iter()
        .find(|candidate| candidate.id == space.space_id)
        .map(|candidate| candidate.device_id.clone());
    let mut options = vec![CheckoutOption {
        path: None,
        label: "Main checkout".into(),
        detail: space.path.clone().into(),
        live: false,
    }];
    let mut worktrees: Vec<(String, Option<String>, Option<String>, bool)> = Vec::new();
    for chat in &state.chats {
        if chat.archived
            || chat.space_id.as_deref() != Some(space.space_id.as_str())
            || Some(&chat.device_id) != space_device.as_ref()
        {
            continue;
        }
        let Some(cwd) = chat.cwd.as_deref() else {
            continue;
        };
        if same_path(cwd, &space.path) {
            continue;
        }
        let live = state.session_for(&chat.id).is_some_and(|session| {
            matches!(
                session.status,
                SessionStatus::Working | SessionStatus::AwaitingInput
            )
        });
        match worktrees.iter_mut().find(|known| same_path(&known.0, cwd)) {
            Some(known) => {
                known.3 |= live;
                if known.2.is_none() {
                    known.2 = chat.title.clone();
                }
            }
            None => worktrees.push((
                cwd.to_owned(),
                chat.branch.clone(),
                chat.title.clone(),
                live,
            )),
        }
    }
    worktrees.sort_by(|a, b| b.3.cmp(&a.3).then_with(|| a.0.cmp(&b.0)));
    options.extend(worktrees.into_iter().map(|(path, branch, title, live)| {
        let label = branch.unwrap_or_else(|| {
            std::path::Path::new(&path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone())
        });
        let thread = title
            .map(|title| format!("Thread \"{title}\""))
            .unwrap_or_else(|| "Agent thread".into());
        let detail = if live {
            format!("{thread} · running")
        } else {
            thread
        };
        CheckoutOption {
            path: Some(path),
            label: label.into(),
            detail: detail.into(),
            live,
        }
    }));
    options
}

impl GitTab {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let history = cx.new(|cx| {
            let mut history = GitHistory::new(state.clone(), cx);
            history.set_target(GitHistoryTarget::Nothing, cx);
            history
        });
        let history_events = cx.subscribe(&history, |tab: &mut Self, _, event, cx| match event {
            GitHistoryEvent::OpenCommit(commit) => {
                let again = tab
                    .detail
                    .as_ref()
                    .is_some_and(|detail| detail.commit.sha == commit.sha);
                tab.select_commit((!again).then(|| commit.clone()), cx);
            }
            GitHistoryEvent::OpenRefBadge(_) => {}
            GitHistoryEvent::SelectWorkingTree => {
                tab.select_commit(None, cx);
                tab.tray_open = true;
                tab.open_composer(cx);
            }
            GitHistoryEvent::FetchSucceeded => tab.load(cx),
        });
        let poll_task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;
                let Ok(()) = this.update(cx, |tab, cx| {
                    if tab.visible && tab.window_active && !tab.loading() {
                        tab.load(cx);
                    }
                    if tab.visible && tab.watch_task.is_none() {
                        tab.watch(cx);
                    }
                }) else {
                    break;
                };
            }
        });
        let message = cx.new(|cx| ComposerInput::new("Commit message (⌘↵ to commit)", cx));
        let message_edits = cx.subscribe(&message, |_, _, event: &ComposerInputEvent, cx| {
            if matches!(event, ComposerInputEvent::Edited) {
                cx.notify();
            }
        });
        let shown_default = crate::settings::current(cx).commit_message_model;
        let model_picker = cx
            .new(|cx| Pickers::new_for_commit_messages(state.clone(), shown_default.as_ref(), cx));
        let model_picks = cx.subscribe(
            &model_picker,
            |tab: &mut Self, _, event: &CommitModelPicked, cx| {
                let default = crate::settings::commit_message_model(cx);
                tab.model_override = (default.as_ref() != Some(&event.0)).then(|| event.0.clone());
                cx.notify();
            },
        );
        let settings_changes = cx.observe_global::<crate::settings::SettingsStore>(|tab, cx| {
            let default = crate::settings::commit_message_model(cx);
            if default != tab.shown_default {
                tab.shown_default = default;
                if tab.model_override.is_none() {
                    tab.reset_model_override(cx);
                }
            }
        });
        Self {
            state,
            history,
            space: None,
            checkout_path: None,
            tree: None,
            error: None,
            reload_wanted: false,
            load_task: None,
            watch_task: None,
            _poll_task: poll_task,
            visible: true,
            window_active: true,
            activation: None,
            pending: None,
            action_error: None,
            action_task: None,
            expanded: None,
            diff_task: None,
            message,
            commit_menu: false,
            confirm: None,
            confirm_focus: cx.focus_handle(),
            stash_menu: false,
            stash_note: None,
            last_commit: None,
            write_task: None,
            write_focus: cx.focus_handle(),
            written: None,
            write_error: None,
            model_override: None,
            shown_default,
            model_picker,
            syncing: None,
            sync_outcome: None,
            sync_task: None,
            checkout_menu: false,
            sync_menu: None,
            detail: None,
            detail_task: None,
            copy_task: None,
            _model_picks: model_picks,
            _settings_changes: settings_changes,
            _message_edits: message_edits,
            _history_events: history_events,
            compact: false,
            toolbar_lead: Vec::new(),
            composer_open: false,
            tray_open: false,
            focus_message: false,
            focus: cx.focus_handle(),
            float: Default::default(),
        }
    }

    pub(super) fn checkout_path(&self) -> Option<&str> {
        self.checkout_path.as_deref()
    }

    pub(super) fn set_space(&mut self, space: Option<GitSpace>, cx: &mut Context<Self>) {
        if self.space == space {
            return;
        }
        self.space = space;
        self.checkout_path = None;
        self.retarget(cx);
    }

    pub(super) fn set_checkout(&mut self, path: Option<String>, cx: &mut Context<Self>) {
        if self.checkout_path == path {
            return;
        }
        self.checkout_path = path;
        self.retarget(cx);
    }

    pub(super) fn set_compact(&mut self, compact: bool, cx: &mut Context<Self>) {
        if self.compact != compact {
            self.compact = compact;
            cx.notify();
        }
    }

    pub(super) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        let resumed = visible && !self.visible;
        self.visible = visible;
        if !visible {
            self.watch_task = None;
        }
        if resumed {
            if !self.loading() {
                self.load(cx);
            }
            if self.watch_task.is_none() {
                self.watch(cx);
            }
        }
    }

    pub(super) fn drop_missing_checkout(&mut self, cx: &mut Context<Self>) {
        let (Some(space), Some(path)) = (&self.space, &self.checkout_path) else {
            return;
        };
        let listed = checkout_options(self.state.read(cx), space)
            .iter()
            .any(|option| {
                option
                    .path
                    .as_deref()
                    .is_some_and(|known| same_path(known, path))
            });
        if !listed {
            self.set_checkout(None, cx);
        }
    }

    fn retarget(&mut self, cx: &mut Context<Self>) {
        self.tree = None;
        self.error = None;
        self.reload_wanted = false;
        self.load_task = None;
        self.watch_task = None;
        self.pending = None;
        self.action_error = None;
        self.action_task = None;
        self.expanded = None;
        self.diff_task = None;
        self.commit_menu = false;
        self.confirm = None;
        self.stash_menu = false;
        self.stash_note = None;
        self.last_commit = None;
        self.syncing = None;
        self.sync_outcome = None;
        self.sync_task = None;
        self.checkout_menu = false;
        self.sync_menu = None;
        self.detail = None;
        self.detail_task = None;
        self.cancel_writing();
        if self.written.take().is_some() {
            self.message.update(cx, |input, cx| input.set_text("", cx));
        }
        self.write_error = None;
        self.reset_model_override(cx);
        let target = self.space.as_ref().map(|space| GitHistoryTarget::Pinned {
            cwd: self
                .checkout_path
                .clone()
                .unwrap_or_else(|| space.path.clone()),
            device: space.device.clone(),
        });
        self.history.update(cx, |history, cx| {
            history.set_target(target.unwrap_or(GitHistoryTarget::Nothing), cx)
        });
        if self.visible {
            self.load(cx);
            self.watch(cx);
        }
        cx.notify();
    }

    fn loading(&self) -> bool {
        self.load_task.is_some()
    }

    fn writing(&self) -> bool {
        self.write_task.is_some()
    }

    fn load_if_idle(&mut self, cx: &mut Context<Self>) {
        if self.loading() {
            self.reload_wanted = true;
        } else {
            self.load(cx);
        }
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(space) = self.space.clone() else {
            return;
        };
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let params = target_params(&space, self.checkout_path.as_deref());
        self.reload_wanted = false;
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let result = decode::<WorkingTree>(
                engine
                    .client()
                    .call(methods::GET_WORKING_TREE, params)
                    .await,
            );
            this.update(cx, |tab, cx| {
                tab.load_task = None;
                if tab.pending.as_ref().is_some_and(|pending| pending.replied) {
                    tab.pending = None;
                    cx.notify();
                }
                match result {
                    Ok(tree) => {
                        let moved = tab
                            .tree
                            .as_ref()
                            .is_some_and(|previous| previous.head_sha != tree.head_sha);
                        let revised = tab
                            .tree
                            .as_ref()
                            .is_some_and(|previous| previous.revision != tree.revision);
                        tab.error = None;
                        if tab.tree.as_ref() != Some(&tree) {
                            tab.tree = Some(tree);
                            cx.notify();
                        }
                        if moved {
                            tab.history.update(cx, |history, cx| history.refresh(cx));
                        }
                        if tab.confirm_is_stale() {
                            tab.confirm = None;
                            cx.notify();
                        }
                        if let (Some(expanded), Some(tree)) = (&tab.expanded, &tab.tree) {
                            if !in_section(tree, expanded.staged, &expanded.path) {
                                tab.expanded = None;
                                tab.diff_task = None;
                                cx.notify();
                            } else if revised {
                                tab.fetch_diff(cx);
                            }
                        }
                    }
                    Err(error) => {
                        tab.error = Some(error.into());
                        cx.notify();
                    }
                }
                if std::mem::take(&mut tab.reload_wanted) {
                    tab.load(cx);
                }
            })
            .ok();
        }));
    }

    fn watch(&mut self, cx: &mut Context<Self>) {
        let Some(space) = self.space.clone() else {
            return;
        };
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let params = target_params(&space, self.checkout_path.as_deref());
        self.watch_task = Some(cx.spawn(async move |this, cx| {
            let mut seen: Option<String> = None;
            loop {
                match engine
                    .client()
                    .subscribe_checked(methods::WATCH_WORKSPACE_GIT_STATUS, params.clone())
                    .await
                {
                    Ok(mut receiver) => {
                        while let Some(value) = receiver.recv().await {
                            let revision = serde_json::from_value::<
                                zeron_proto::WorkspaceGitStatusFrame,
                            >(value)
                            .ok()
                            .and_then(|frame| frame.status)
                            .map(|status| status.revision);
                            if revision.is_none() || revision == seen {
                                continue;
                            }
                            let first = seen.is_none();
                            seen = revision;
                            if first {
                                continue;
                            }
                            if this.update(cx, |tab, cx| tab.load_if_idle(cx)).is_err() {
                                return;
                            }
                        }
                    }
                    Err(error)
                        if matches!(error, RpcError::UnknownMethod(_))
                            || error.to_string().starts_with("unknown method:") =>
                    {
                        return;
                    }
                    Err(_) => {}
                }
                cx.background_executor().timer(WATCH_RETRY).await;
            }
        }));
    }

    fn supports(&self, capability: &str, cx: &App) -> bool {
        let Some(space) = &self.space else {
            return false;
        };
        let state = self.state.read(cx);
        space
            .device
            .as_deref()
            .or(state.local_device_id.as_deref())
            .is_some_and(|device| state.device_supports(device, capability))
    }

    fn source_control(&self, cx: &App) -> bool {
        self.supports(zeron_proto::capabilities::SOURCE_CONTROL_V1, cx)
    }

    pub(super) fn syncs_branches(&self, cx: &App) -> bool {
        self.source_control(cx)
            && self.supports(zeron_proto::capabilities::SOURCE_CONTROL_SYNC_V1, cx)
    }

    fn allows(&self, action: &WorkingTreeAction, policy: GitAgentBusyPolicy, cx: &App) -> bool {
        !self.busy_for_sync()
            && self.source_control(cx)
            && self.supports(action.required_capability(), cx)
            && self
                .tree
                .as_ref()
                .is_some_and(|tree| !tree.agent_active || policy.allows(action))
    }

    fn run_action(
        &mut self,
        action: WorkingTreeAction,
        target: PendingTarget,
        cx: &mut Context<Self>,
    ) {
        let policy = busy_policy(cx);
        if !self.allows(&action, policy, cx) {
            return;
        }
        let (Some(space), Some(tree)) = (self.space.clone(), self.tree.as_ref()) else {
            return;
        };
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let op = action.clone();
        let request = RunWorkingTreeActionRequest {
            target: workspace_target(&space, self.checkout_path.as_deref()),
            expected_revision: tree.revision.clone(),
            busy_policy: policy,
            action,
        };
        let params = space.route(request);
        let committing = target == PendingTarget::Commit;
        let stashing = target == PendingTarget::Stash;
        let detailed = target == PendingTarget::Detail;
        let detail_sha = detailed
            .then(|| self.detail.as_ref().map(|detail| detail.commit.sha.clone()))
            .flatten();
        self.pending = Some(Pending {
            target,
            replied: false,
        });
        self.action_error = None;
        self.expanded = None;
        self.diff_task = None;
        self.commit_menu = false;
        self.confirm = None;
        self.stash_menu = false;
        self.stash_note = None;
        cx.notify();
        self.action_task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::RUN_WORKING_TREE_ACTION, params)
                .await;
            this.update(cx, |tab, cx| {
                match result {
                    Ok(value) => {
                        if let Some(pending) = tab.pending.as_mut() {
                            pending.replied = true;
                        }
                        let reply: WorkingTreeActionReply =
                            serde_json::from_value(value).unwrap_or_default();
                        if stashing && !reply.conflicts.is_empty() {
                            tab.stash_note = Some(conflict_note(&reply.conflicts).into());
                        }
                        if detailed {
                            tab.finish_detail(&op, &reply, detail_sha.as_deref(), cx);
                        }
                        if committing {
                            tab.last_commit = reply.commit;
                            if let WorkingTreeAction::Commit { message, .. } = &op
                                && tab.message.read(cx).text() == message.as_str()
                            {
                                tab.message.update(cx, |input, cx| input.set_text("", cx));
                            }
                            tab.written = None;
                            tab.write_error = None;
                            tab.reset_model_override(cx);
                        }
                    }
                    Err(error) => {
                        tab.pending = None;
                        match tab.detail.as_mut().filter(|detail| {
                            detailed && detail_sha.as_deref() == Some(detail.commit.sha.as_str())
                        }) {
                            Some(detail) => detail.outcome = Some((error.to_string().into(), true)),
                            None => tab.action_error = Some(error.to_string().into()),
                        }
                    }
                }
                tab.load(cx);
                cx.notify();
            })
            .ok();
        }));
    }

    fn can_commit(&self, kind: CommitKind, policy: GitAgentBusyPolicy, cx: &App) -> bool {
        let Some(tree) = self.tree.as_ref() else {
            return false;
        };
        let ready = match kind {
            CommitKind::Staged => !staged_files(tree).is_empty(),
            CommitKind::Amend => tree.head_sha.is_some(),
            CommitKind::All => !tree.files.is_empty(),
        };
        ready
            && !self.writing()
            && !self.message.read(cx).text().trim().is_empty()
            && self.allows(&kind.action(String::new()), policy, cx)
    }

    fn commit(&mut self, kind: CommitKind, cx: &mut Context<Self>) {
        self.commit_menu = false;
        if !self.can_commit(kind, busy_policy(cx), cx) {
            cx.notify();
            return;
        }
        let message = self.message.read(cx).text().to_string();
        self.run_action(kind.action(message), PendingTarget::Commit, cx);
    }

    fn confirm_is_stale(&self) -> bool {
        let Some(tree) = &self.tree else {
            return false;
        };
        match &self.confirm {
            Some(Confirm::Discard { revision, .. }) => *revision != tree.revision,
            Some(Confirm::DropStash { oid }) => !tree.stashes.iter().any(|stash| stash.oid == *oid),
            Some(Confirm::Checkout { .. } | Confirm::CherryPick { .. }) | None => false,
        }
    }

    fn open_confirm(&mut self, confirm: Confirm, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm = Some(confirm);
        window.focus(&self.confirm_focus, cx);
        cx.notify();
    }

    fn cancel_confirm(&mut self, cx: &mut Context<Self>) {
        if self.confirm.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn dismiss_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.model_picker.read(cx).is_open() {
            self.model_picker.update(cx, |picker, cx| picker.close(cx));
            return true;
        }
        if self.confirm.is_some() {
            self.cancel_confirm(cx);
            return true;
        }
        if self.writing() {
            self.stop_writing(window, cx);
            return true;
        }
        if self.commit_menu || self.stash_menu || self.checkout_menu || self.sync_menu.is_some() {
            self.commit_menu = false;
            self.stash_menu = false;
            self.checkout_menu = false;
            self.sync_menu = None;
            cx.notify();
            return true;
        }
        if self
            .detail
            .as_ref()
            .is_some_and(|detail| detail.branch_form.is_some())
        {
            self.close_branch_form(cx);
            return true;
        }
        self.fold_composer(window, cx)
    }

    pub(super) fn open_composer(&mut self, cx: &mut Context<Self>) {
        self.composer_open = true;
        self.focus_message = true;
        cx.notify();
    }

    pub(super) fn fold_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.composer_open {
            return false;
        }
        self.composer_open = false;
        self.commit_menu = false;
        self.stash_menu = false;
        if self.message.focus_handle(cx).contains_focused(window, cx)
            || self.write_focus.contains_focused(window, cx)
        {
            window.focus(&self.focus, cx);
        }
        cx.notify();
        true
    }

    pub(super) fn has_draft(&self, cx: &App) -> bool {
        !self.message.read(cx).text().trim().is_empty()
    }

    fn bottom_clearance(&self) -> f32 {
        crate::pull_request_detail::NAV_CLEARANCE
            + if self.composer_open {
                self.float.height() + 8.0
            } else {
                0.0
            }
    }

    fn composer_idle(&self, cx: &App) -> bool {
        !self.has_draft(cx)
            && !self.writing()
            && !self.commit_menu
            && !self.stash_menu
            && self.confirm.is_none()
            && !self.model_picker.read(cx).is_open()
    }

    fn confirm_discard(&mut self, cx: &mut Context<Self>) {
        let stale = self.confirm_is_stale();
        let Some(Confirm::Discard { path, .. }) = self.confirm.take() else {
            return;
        };
        cx.notify();
        if stale {
            return;
        }
        self.run_action(
            WorkingTreeAction::Discard {
                paths: vec![path.clone()],
            },
            PendingTarget::File {
                staged: false,
                path,
            },
            cx,
        );
    }

    fn confirm_drop_stash(&mut self, cx: &mut Context<Self>) {
        let Some(Confirm::DropStash { oid }) = self.confirm.take() else {
            return;
        };
        cx.notify();
        self.run_action(
            WorkingTreeAction::StashDrop { oid },
            PendingTarget::Stash,
            cx,
        );
    }

    fn commits_supported(&self, cx: &App) -> bool {
        self.source_control(cx)
            && self.supports(zeron_proto::capabilities::SOURCE_CONTROL_COMMITS_V1, cx)
    }

    fn select_commit(&mut self, commit: Option<GitHistoryCommit>, cx: &mut Context<Self>) {
        let sha = commit.as_ref().map(|commit| commit.sha.clone());
        self.history
            .update(cx, |history, cx| history.set_selected_sha(sha, cx));
        if matches!(
            self.confirm,
            Some(Confirm::Checkout { .. } | Confirm::CherryPick { .. })
        ) {
            self.confirm = None;
        }
        self.detail_task = None;
        self.detail = commit.map(CommitDetail::new);
        self.fetch_commit_diff(cx);
        cx.notify();
    }

    fn fetch_commit_diff(&mut self, cx: &mut Context<Self>) {
        let (Some(space), Some(detail)) = (self.space.clone(), self.detail.as_ref()) else {
            return;
        };
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let sha = detail.commit.sha.clone();
        let cwd = self
            .checkout_path
            .clone()
            .unwrap_or_else(|| space.path.clone());
        let params =
            space.route(serde_json::json!({"cwd": cwd, "mode": "commit", "commitSha": sha}));
        self.detail_task = Some(cx.spawn(async move |this, cx| {
            let result = decode::<CheckoutDiff>(
                engine
                    .client()
                    .call(methods::GET_CHECKOUT_DIFF, params)
                    .await,
            );
            this.update(cx, |tab, cx| {
                let Some(detail) = tab
                    .detail
                    .as_mut()
                    .filter(|detail| detail.commit.sha == sha)
                else {
                    return;
                };
                detail.diff = match result {
                    Ok(diff) => {
                        let parsed = changes::parse_patch(&diff.patch);
                        Loadable::Ready((diff, parsed))
                    }
                    Err(error) => Loadable::Error(error),
                };
                detail.sync_rows();
                cx.notify();
            })
            .ok();
        }));
    }

    fn move_refusal(&self, action: &WorkingTreeAction, cx: &App) -> Option<&'static str> {
        let Some(tree) = self.tree.as_ref() else {
            return Some("Reading the working tree…");
        };
        if !self.commits_supported(cx) {
            return Some("The Zeron on this device is too old to check out or cherry-pick here.");
        }
        if tree.cherry_pick_head.is_some() {
            return Some("A cherry-pick is in progress. Finish or abort it first.");
        }
        if tree.agent_active && !busy_policy(cx).allows(action) {
            return Some(
                "An agent is running in this checkout, so checkout and cherry-pick wait until it stops.",
            );
        }
        if tree.has_tracked_changes() {
            return Some("Commit or stash your changes to check out or cherry-pick.");
        }
        None
    }

    fn checkout_action(&self) -> Option<WorkingTreeAction> {
        let (detail, tree) = (self.detail.as_ref()?, self.tree.as_ref()?);
        checkout_target(&detail.commit, tree)
            .map(|(target_ref, detach)| WorkingTreeAction::Checkout { target_ref, detach })
    }

    fn can_checkout(&self, cx: &App) -> bool {
        self.checkout_action().is_some_and(|action| {
            self.move_refusal(&action, cx).is_none() && self.allows(&action, busy_policy(cx), cx)
        })
    }

    fn can_cherry_pick(&self, cx: &App) -> bool {
        let (Some(detail), Some(tree)) = (self.detail.as_ref(), self.tree.as_ref()) else {
            return false;
        };
        let action = WorkingTreeAction::CherryPick {
            sha: detail.commit.sha.clone(),
        };
        detail.commit.parent_shas.len() <= 1
            && tree.head_sha.is_some()
            && tree.head_sha.as_deref() != Some(detail.commit.sha.as_str())
            && self.move_refusal(&action, cx).is_none()
            && self.allows(&action, busy_policy(cx), cx)
    }

    fn branch_action(&self, checkout: bool, cx: &App) -> Option<WorkingTreeAction> {
        let detail = self.detail.as_ref()?;
        let name = detail
            .branch_form
            .as_ref()
            .map(|form| form.name.read(cx).text().trim().to_owned())
            .unwrap_or_default();
        Some(WorkingTreeAction::CreateBranch {
            name,
            start_point: Some(detail.commit.sha.clone()),
            checkout,
        })
    }

    fn can_branch(&self, checkout: bool, cx: &App) -> bool {
        let (Some(action), Some(tree), Some(detail)) = (
            self.branch_action(checkout, cx),
            self.tree.as_ref(),
            self.detail.as_ref(),
        ) else {
            return false;
        };
        let moves = checkout && tree.head_sha.as_deref() != Some(detail.commit.sha.as_str());
        tree.head_sha.is_some()
            && self.allows(&action, busy_policy(cx), cx)
            && (!checkout || tree.cherry_pick_head.is_none())
            && (!moves || self.move_refusal(&action, cx).is_none())
    }

    fn start_checkout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_checkout(cx) {
            return;
        }
        match self.checkout_action() {
            Some(WorkingTreeAction::Checkout {
                target_ref,
                detach: true,
            }) => self.open_confirm(Confirm::Checkout { sha: target_ref }, window, cx),
            Some(action) => self.run_action(action, PendingTarget::Detail, cx),
            None => {}
        }
    }

    fn start_cherry_pick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_cherry_pick(cx) {
            return;
        }
        if let Some(sha) = self.detail.as_ref().map(|detail| detail.commit.sha.clone()) {
            self.open_confirm(Confirm::CherryPick { sha }, window, cx);
        }
    }

    fn confirm_detail(&mut self, cx: &mut Context<Self>) {
        let action = match self.confirm.take() {
            Some(Confirm::Checkout { sha }) if self.can_checkout(cx) => {
                WorkingTreeAction::Checkout {
                    target_ref: sha,
                    detach: true,
                }
            }
            Some(Confirm::CherryPick { sha }) if self.can_cherry_pick(cx) => {
                WorkingTreeAction::CherryPick { sha }
            }
            _ => {
                cx.notify();
                return;
            }
        };
        self.run_action(action, PendingTarget::Detail, cx);
    }

    fn open_branch_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.detail.is_none() || !self.can_branch(false, cx) {
            return;
        }
        let name = cx.new(|cx| ComposerInput::new("Branch name", cx));
        let edits = cx.subscribe(&name, |_, _, event: &ComposerInputEvent, cx| {
            if matches!(event, ComposerInputEvent::Edited) {
                cx.notify();
            }
        });
        window.focus(&name.focus_handle(cx), cx);
        if let Some(detail) = self.detail.as_mut() {
            detail.outcome = None;
            detail.branch_form = Some(BranchForm {
                name,
                checkout: false,
                _edits: edits,
            });
        }
        cx.notify();
    }

    fn close_branch_form(&mut self, cx: &mut Context<Self>) {
        if let Some(detail) = self.detail.as_mut()
            && detail.branch_form.take().is_some()
        {
            cx.notify();
        }
    }

    fn create_branch(&mut self, cx: &mut Context<Self>) {
        let checkout = self
            .detail
            .as_ref()
            .and_then(|detail| detail.branch_form.as_ref())
            .is_some_and(|form| form.checkout);
        let Some(action) = self.branch_action(checkout, cx) else {
            return;
        };
        if matches!(&action, WorkingTreeAction::CreateBranch { name, .. } if name.is_empty())
            || !self.can_branch(checkout, cx)
        {
            return;
        }
        self.run_action(action, PendingTarget::Detail, cx);
    }

    fn copy_commit_sha(&mut self, cx: &mut Context<Self>) {
        let Some(sha) = self.detail.as_ref().map(|detail| detail.commit.sha.clone()) else {
            return;
        };
        self.history
            .update(cx, |history, cx| history.copy_sha(sha, cx));
        cx.notify();
        self.copy_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1_300))
                .await;
            this.update(cx, |_, cx| cx.notify()).ok();
        }));
    }

    fn toggle_commit_file(&mut self, path: Option<String>, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.as_mut() else {
            return;
        };
        match path {
            Some(path) => {
                if !detail.open_files.remove(&path) {
                    detail.open_files.insert(path);
                }
            }
            None => {
                let Some((diff, _)) = detail.diff.ready() else {
                    return;
                };
                if detail.open_files.len() == diff.files.len() {
                    detail.open_files.clear();
                } else {
                    detail.open_files = diff.files.iter().map(|file| file.path.clone()).collect();
                }
            }
        }
        detail.sync_rows();
        cx.notify();
    }

    fn finish_detail(
        &mut self,
        op: &WorkingTreeAction,
        reply: &WorkingTreeActionReply,
        detail_sha: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        self.history.update(cx, |history, cx| history.refresh(cx));
        let sha = match op {
            WorkingTreeAction::Checkout { target_ref, .. } => target_ref.as_str(),
            WorkingTreeAction::CherryPick { sha } => sha.as_str(),
            WorkingTreeAction::CreateBranch { start_point, .. } => {
                start_point.as_deref().unwrap_or_default()
            }
            _ => return,
        };
        let short = short_sha(sha);
        let text = match op {
            WorkingTreeAction::Checkout {
                target_ref,
                detach: false,
            } => format!("Checked out {target_ref}."),
            WorkingTreeAction::Checkout { .. } => format!(
                "Checked out {short}. HEAD is detached; create a branch here to keep new commits."
            ),
            WorkingTreeAction::CreateBranch {
                name,
                checkout: true,
                ..
            } => format!("Created {name} at {short} and checked it out."),
            WorkingTreeAction::CreateBranch { name, .. } => format!("Created {name} at {short}."),
            WorkingTreeAction::CherryPick { .. } if !reply.conflicts.is_empty() => {
                let subject = self
                    .detail
                    .as_ref()
                    .filter(|detail| detail.commit.sha == sha)
                    .map(|detail| detail.commit.subject.clone());
                self.select_commit(None, cx);
                self.tray_open = true;
                self.open_composer(cx);
                if let Some(subject) = subject
                    && self.message.read(cx).text().trim().is_empty()
                {
                    self.message
                        .update(cx, |input, cx| input.set_text(subject, cx));
                }
                return;
            }
            WorkingTreeAction::CherryPick { .. } => match &reply.commit {
                Some(commit) => format!(
                    "Cherry-picked {short} onto {} as {}.",
                    commit.branch.as_deref().unwrap_or("the detached HEAD"),
                    short_sha(&commit.sha)
                ),
                None => format!("Cherry-picked {short}."),
            },
            _ => return,
        };
        let Some(detail) = self
            .detail
            .as_mut()
            .filter(|detail| detail_sha == Some(detail.commit.sha.as_str()))
        else {
            return;
        };
        detail.branch_form = None;
        detail.outcome = Some((text.into(), false));
        cx.notify();
    }

    pub(super) fn sync_outcome(&self, kind: SyncKind) -> Option<&SyncOutcome> {
        self.sync_outcome
            .as_ref()
            .filter(|outcome| outcome.kind == kind)
    }

    fn busy_for_sync(&self) -> bool {
        self.syncing.is_some() || self.pending.is_some()
    }

    fn push_refusal(
        &self,
        tree: &WorkingTree,
        policy: GitAgentBusyPolicy,
        cx: &App,
    ) -> Option<&'static str> {
        if !self.syncs_branches(cx) {
            return Some("Zeron on this device is too old to push. Update it first.");
        }
        if self.busy_for_sync() {
            return Some("Another Git action is still running.");
        }
        if tree.agent_active
            && !policy.allows_sync(SyncDirection::Push {
                set_upstream: false,
            })
        {
            return Some(agent_banner(policy));
        }
        if tree.branch.is_none() {
            return Some("HEAD is detached. Check out a branch to push.");
        }
        if tree.head_sha.is_none() {
            return Some("This branch has no commits yet.");
        }
        if tree.push_remote.is_none() {
            return Some(
                "There is no remote to push to. Add one named origin, or set remote.pushDefault.",
            );
        }
        None
    }

    pub(super) fn sync_refusal(&self, kind: SyncKind, cx: &App) -> Option<SharedString> {
        let Some(tree) = self.tree.as_ref() else {
            return Some("Reading the working tree…".into());
        };
        let policy = busy_policy(cx);
        match kind {
            SyncKind::Push => {
                if let Some(reason) = self.push_refusal(tree, policy, cx) {
                    return Some(reason.into());
                }
                match (&tree.upstream, &tree.branch) {
                    (Some(upstream), Some(branch)) if tree.ahead == 0 => {
                        Some(format!("Nothing to push. {branch} matches {upstream}.").into())
                    }
                    _ => None,
                }
            }
            SyncKind::Pull => {
                if !self.syncs_branches(cx) {
                    return Some(
                        "Zeron on this device is too old to pull. Update it first.".into(),
                    );
                }
                if self.busy_for_sync() {
                    return Some("Another Git action is still running.".into());
                }
                if tree.agent_active && !policy.allows_sync(SyncDirection::Pull) {
                    return Some(match policy {
                        GitAgentBusyPolicy::SafeOnly => {
                            "An agent is running in this worktree. Pull waits until it stops, so it cannot rewrite files under the agent.".into()
                        }
                        _ => agent_banner(policy).into(),
                    });
                }
                if tree.branch.is_none() {
                    return Some("HEAD is detached. Check out a branch to pull.".into());
                }
                if tree.upstream.is_none() {
                    return Some("This branch has no upstream to pull from. Push it first.".into());
                }
                None
            }
        }
    }

    pub(super) fn push_prompt(&self) -> Option<SharedString> {
        let tree = self.tree.as_ref()?;
        let branch = tree.branch.as_deref()?;
        let commits = commit_count(tree.ahead);
        if let Some(destination) = publish_destination(tree) {
            let remote = tree.push_remote.as_deref().unwrap_or_default();
            let upstream = tree.upstream.as_deref().unwrap_or_default();
            let mut prompt = format!("Push {commits} from {branch} to {destination}?");
            if upstream.starts_with(&format!("{remote}/")) {
                prompt.push_str(&format!(
                    " It will track {destination} instead of {upstream}."
                ));
            }
            return Some(prompt.into());
        }
        Some(
            match (&tree.upstream, &tree.push_remote) {
                (Some(upstream), _) => format!("Push {commits} from {branch} to {upstream}?"),
                (None, Some(remote)) => format!(
                    "Push {commits} to {remote} as the new branch {branch}? It will track {remote}/{branch}."
                ),
                (None, None) => return None,
            }
            .into(),
        )
    }

    pub(super) fn open_sync(&mut self, kind: SyncKind, cx: &mut Context<Self>) {
        if self
            .sync_outcome
            .as_ref()
            .is_some_and(|outcome| outcome.kind == kind)
        {
            self.sync_outcome = None;
        }
        if kind == SyncKind::Pull {
            self.start_sync(SyncKind::Pull, cx);
        }
        cx.notify();
    }

    pub(super) fn start_sync(&mut self, kind: SyncKind, cx: &mut Context<Self>) {
        if self.sync_refusal(kind, cx).is_some() {
            return;
        }
        let (Some(space), Some(tree)) = (self.space.clone(), self.tree.clone()) else {
            return;
        };
        let (Some(head), Some(branch)) = (tree.head_sha.clone(), tree.branch.clone()) else {
            return;
        };
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let (direction, destination) = match kind {
            SyncKind::Push => (push_direction(&tree), publish_destination(&tree)),
            SyncKind::Pull => (SyncDirection::Pull, None),
        };
        let params = space.route(self.sync_request(&space, head, direction, cx));
        self.syncing = Some(kind);
        self.sync_outcome = None;
        cx.notify();
        self.sync_task = Some(cx.spawn(async move |this, cx| {
            let result =
                decode::<SyncBranchReply>(engine.client().call(methods::SYNC_BRANCH, params).await);
            this.update(cx, |tab, cx| {
                tab.syncing = None;
                tab.sync_outcome = Some(match result {
                    Ok(reply) => SyncOutcome {
                        kind,
                        text: sync_note(kind, &branch, destination.as_deref(), &reply).into(),
                        failed: false,
                    },
                    Err(error) => SyncOutcome {
                        kind,
                        text: error.into(),
                        failed: true,
                    },
                });
                tab.load(cx);
                cx.notify();
            })
            .ok();
        }));
    }

    fn sync_request(
        &self,
        space: &GitSpace,
        head: String,
        direction: SyncDirection,
        cx: &App,
    ) -> SyncBranchRequest {
        SyncBranchRequest {
            target: workspace_target(space, self.checkout_path.as_deref()),
            expected_head_sha: head,
            busy_policy: busy_policy(cx),
            direction,
        }
    }

    fn reset_model_override(&mut self, cx: &mut Context<Self>) {
        self.model_override = None;
        let default = self.shown_default.clone();
        self.model_picker.update(cx, |picker, cx| {
            picker.set_commit_model(default.as_ref(), cx)
        });
        cx.notify();
    }

    fn can_write(&self, cx: &App) -> bool {
        self.tree
            .as_ref()
            .is_some_and(|tree| !staged_files(tree).is_empty())
            && !self
                .pending
                .as_ref()
                .is_some_and(|pending| pending.target == PendingTarget::Commit)
            && self.commits_supported(cx)
    }

    fn write_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.writing() {
            self.stop_writing(window, cx);
            return;
        }
        if !self.can_write(cx) {
            return;
        }
        let Some(space) = self.space.clone() else {
            return;
        };
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let settings = crate::settings::current(cx);
        let choice = self
            .model_override
            .clone()
            .or(settings.commit_message_model.clone())
            .filter(|_| space.device.is_none());
        let request = GenerateCommitMessageRequest {
            target: workspace_target(&space, self.checkout_path.as_deref()),
            instruction: Some(settings.effective_commit_instruction().to_owned()),
            harness: choice.as_ref().map(|choice| choice.harness),
            model: choice.map(|choice| choice.model),
        };
        let params = space.route(request);
        self.write_error = None;
        self.commit_menu = false;
        window.focus(&self.write_focus, cx);
        cx.notify();
        self.write_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = decode::<GeneratedCommitMessage>(
                engine
                    .client()
                    .call(methods::GENERATE_COMMIT_MESSAGE, params)
                    .await,
            );
            this.update_in(cx, |tab, window, cx| {
                tab.write_task = None;
                match result {
                    Ok(generated) => {
                        tab.message.update(cx, |input, cx| {
                            input.set_text(generated.message.clone(), cx)
                        });
                        tab.written = Some(generated);
                    }
                    Err(error) => tab.write_error = Some(error.into()),
                }
                window.focus(&tab.message.focus_handle(cx), cx);
                cx.notify();
            })
            .ok();
        }));
    }

    fn cancel_writing(&mut self) {
        self.write_task = None;
    }

    fn stop_writing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.writing() {
            return;
        }
        self.cancel_writing();
        window.focus(&self.message.focus_handle(cx), cx);
        cx.notify();
    }

    fn toggle_diff(&mut self, staged: bool, path: String, cx: &mut Context<Self>) {
        if self
            .expanded
            .as_ref()
            .is_some_and(|open| open.staged == staged && open.path == path)
        {
            self.expanded = None;
            self.diff_task = None;
            cx.notify();
            return;
        }
        if !self.source_control(cx) {
            return;
        }
        self.expanded = Some(Expanded::new(staged, path));
        self.fetch_diff(cx);
        cx.notify();
    }

    fn fetch_diff(&mut self, cx: &mut Context<Self>) {
        let (Some(space), Some(open)) = (self.space.clone(), self.expanded.as_ref()) else {
            return;
        };
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let (staged, path) = (open.staged, open.path.clone());
        let request = GetWorkingTreeFileDiffRequest {
            target: workspace_target(&space, self.checkout_path.as_deref()),
            path: path.clone(),
            staged,
        };
        let params = space.route(request);
        self.diff_task = Some(cx.spawn(async move |this, cx| {
            let result = decode::<WorkingTreeFileDiff>(
                engine
                    .client()
                    .call(methods::GET_WORKING_TREE_FILE_DIFF, params)
                    .await,
            );
            this.update(cx, |tab, cx| {
                let Some(open) = tab
                    .expanded
                    .as_mut()
                    .filter(|open| open.staged == staged && open.path == path)
                else {
                    return;
                };
                open.set_view(match result {
                    Ok(diff) => {
                        let parsed = changes::parse_patch(&diff.patch).into_iter().next();
                        Loadable::Ready((diff, parsed))
                    }
                    Err(error) => Loadable::Error(error),
                });
                cx.notify();
            })
            .ok();
        }));
    }

    fn render_empty(&self, theme: &Theme) -> AnyElement {
        div()
            .id("pull-requests-git-empty")
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(10.0))
            .px(px(16.0))
            .child(
                div()
                    .size(px(36.0))
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(theme.hairline(0.09))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        icon(icons::LUCIDE_GIT_BRANCH)
                            .size(px(16.0))
                            .text_color(theme.text),
                    ),
            )
            .child(
                div()
                    .text_size(px(15.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child("Pick a project"),
            )
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(theme.text_muted)
                    .child(
                        "Choose a project in the repository menu to see its commit graph and working tree.",
                    ),
            )
            .into_any_element()
    }

    fn toggle_checkout_menu(&mut self, cx: &mut Context<Self>) {
        self.checkout_menu = !self.checkout_menu;
        self.sync_menu = None;
        cx.notify();
    }

    fn toggle_sync_menu(&mut self, kind: SyncKind, cx: &mut Context<Self>) {
        self.checkout_menu = false;
        if self.sync_menu == Some(kind) {
            self.sync_menu = None;
            cx.notify();
            return;
        }
        self.sync_menu = Some(kind);
        self.open_sync(kind, cx);
    }

    fn render_toolbar(
        &self,
        lead: Vec<AnyElement>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id("pull-requests-git-toolbar")
            .debug_selector(|| "pull-requests-git-toolbar".into())
            .flex_none()
            .flex()
            .items_center()
            .gap(px(if self.compact { 6.0 } else { 8.0 }))
            .px(px(12.0))
            .py(px(8.0))
            .children(lead)
            .children(self.render_checkout_control(theme, cx))
            .child(div().flex_1())
            .children(self.render_sync_controls(theme, cx))
            .into_any_element()
    }

    fn render_checkout_control(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let space = self.space.as_ref()?;
        let options = checkout_options(self.state.read(cx), space);
        let option = options
            .iter()
            .find(|option| option.path.as_deref() == self.checkout_path())?;
        let tree = self.tree.as_ref();
        let label: SharedString = match tree {
            Some(tree) if tree.branch.is_some() => tree.branch.clone().unwrap_or_default().into(),
            Some(WorkingTree {
                head_sha: Some(head),
                ..
            }) => format!("detached at {}", short_sha(head)).into(),
            _ => option.label.clone(),
        };
        let ahead = tree.map(|tree| (tree.ahead, tree.behind));
        let agent = option.path.is_some();
        let menu = self
            .checkout_menu
            .then(|| self.render_checkout_menu(options.clone(), theme, cx));
        Some(
            top_chip("pull-requests-git-checkout", "Checkout", theme)
                .debug_selector(|| "pull-requests-git-checkout".into())
                .flex_shrink(1.0)
                .min_w_0()
                .max_w(px(280.0))
                .child(
                    icon(icons::GIT_BRANCH)
                        .size(px(14.0))
                        .flex_none()
                        .text_color(theme.text_muted),
                )
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(label),
                )
                .when(agent, |el| {
                    el.child(
                        div()
                            .px(px(5.0))
                            .rounded(px(4.0))
                            .bg(theme.ink(0.08))
                            .text_size(px(11.0))
                            .text_color(theme.text_muted)
                            .child("agent"),
                    )
                })
                .when_some(ahead.filter(|(a, b)| *a + *b > 0), |el, (a, b)| {
                    let count = |glyph: &'static str, value: u32| {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(1.0))
                            .text_size(px(12.0))
                            .text_color(theme.text_muted)
                            .child(
                                icon(glyph)
                                    .size(px(11.0))
                                    .flex_none()
                                    .text_color(theme.text_muted),
                            )
                            .child(SharedString::from(value.to_string()))
                    };
                    el.when(a > 0, |el| el.child(count(icons::ARROW_UP, a)))
                        .when(b > 0, |el| el.child(count(icons::ARROW_DOWN, b)))
                })
                .child(
                    icon(icons::ALT_ARROW_DOWN)
                        .size(px(12.0))
                        .flex_none()
                        .text_color(theme.text_muted.opacity(0.7)),
                )
                .on_click(cx.listener(|tab, _, _, cx| tab.toggle_checkout_menu(cx)))
                .children(menu)
                .into_any_element(),
        )
    }

    fn render_checkout_menu(
        &self,
        options: Vec<CheckoutOption>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let popup = theme.for_popup();
        let current = self.checkout_path.clone();
        let group = |label: &'static str| {
            div()
                .px(px(8.0))
                .pt(px(6.0))
                .pb(px(2.0))
                .text_size(px(11.0))
                .text_color(popup.text_muted)
                .child(label)
        };
        let worktrees = options.len().saturating_sub(1);
        let mut rows = options
            .into_iter()
            .enumerate()
            .map(|(index, option)| {
                let selected = option.path == current;
                let path = option.path.clone();
                menu_option(
                    format!("pull-requests-git-checkout-{index}"),
                    option.label,
                    option.detail,
                    selected,
                    option.live,
                    &popup,
                )
                .on_click(cx.listener(move |tab, _, _, cx| {
                    tab.checkout_menu = false;
                    tab.set_checkout(path.clone(), cx);
                    cx.notify();
                }))
                .into_any_element()
            })
            .collect::<Vec<_>>()
            .into_iter();
        popover::anchored_menu_below(
            "git-checkout-menu",
            popover::popover_card(&popup)
                .id("git-checkout-card")
                .min_w(px(280.0))
                .max_w(px(420.0))
                .p(px(4.0))
                .flex()
                .flex_col()
                .gap(px(popover::MENU_GAP))
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(group("Main checkout"))
                .children(rows.next())
                .child(group("Agent worktrees"))
                .when(worktrees == 0, |el| {
                    el.child(
                        div()
                            .px(px(8.0))
                            .py(px(6.0))
                            .text_color(popup.text_muted)
                            .child("No agent worktrees in this project"),
                    )
                })
                .children(rows)
                .on_mouse_down_out(cx.listener(|tab, _, _, cx| {
                    tab.checkout_menu = false;
                    cx.notify();
                }))
                .into_any_element(),
            None,
        )
    }

    fn render_sync_controls(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.tree.as_ref()?;
        if !self.syncs_branches(cx) {
            return None;
        }
        Some(
            div()
                .flex_none()
                .flex()
                .gap(px(6.0))
                .child(self.render_sync_chip(SyncKind::Pull, theme, cx))
                .child(self.render_sync_chip(SyncKind::Push, theme, cx))
                .into_any_element(),
        )
    }

    fn render_sync_chip(
        &self,
        kind: SyncKind,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ahead = self.tree.as_ref().map_or(0, |tree| tree.ahead);
        let (id, label, glyph) = match kind {
            SyncKind::Pull => ("pull-requests-git-pull", "Pull", icons::ARROW_DOWN),
            SyncKind::Push => ("pull-requests-git-push", "Push", icons::ARROW_UP),
        };
        let text: Option<String> = match (kind, self.syncing, ahead) {
            (SyncKind::Pull, Some(SyncKind::Pull), _) => Some("Pulling…".into()),
            (SyncKind::Push, Some(SyncKind::Push), _) => Some("Pushing…".into()),
            _ if self.compact => None,
            (SyncKind::Pull, ..) => Some("Pull".into()),
            (SyncKind::Push, _, 0) => Some("Push".into()),
            (SyncKind::Push, _, ahead) => Some(format!("Push {ahead}")),
        };
        let quiet = self.sync_refusal(kind, cx).is_some() && self.syncing != Some(kind);
        let menu = (self.sync_menu == Some(kind)).then(|| self.render_sync_menu(kind, theme, cx));
        top_chip(id, label, theme)
            .debug_selector(move || id.into())
            .when(quiet, |el| el.text_color(theme.text_muted))
            .when(text.is_none(), |el| el.w(px(28.0)).px_0().justify_center())
            .child(icon(glyph).size(px(14.0)).flex_none().text_color(if quiet {
                theme.text_muted
            } else {
                theme.text
            }))
            .children(text.map(SharedString::from))
            .on_click(cx.listener(move |tab, _, _, cx| tab.toggle_sync_menu(kind, cx)))
            .children(menu)
            .into_any_element()
    }

    fn render_sync_menu(
        &self,
        kind: SyncKind,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let popup = theme.for_popup();
        let branch = self
            .tree
            .as_ref()
            .and_then(|tree| tree.branch.clone())
            .unwrap_or_default();
        let upstream = self
            .tree
            .as_ref()
            .and_then(|tree| tree.upstream.clone())
            .unwrap_or_default();
        let (text, failed, confirm) = if self.syncing == Some(kind) {
            let text = match kind {
                SyncKind::Push => format!("Pushing {branch}…"),
                SyncKind::Pull => format!("Pulling {upstream} into {branch}…"),
            };
            (SharedString::from(text), false, false)
        } else if let Some(outcome) = self.sync_outcome(kind) {
            (outcome.text.clone(), outcome.failed, false)
        } else if let Some(reason) = self.sync_refusal(kind, cx) {
            (reason, false, false)
        } else if kind == SyncKind::Push {
            (self.push_prompt().unwrap_or_default(), false, true)
        } else {
            (SharedString::default(), false, false)
        };
        let id = match kind {
            SyncKind::Push => "git-push-popover",
            SyncKind::Pull => "git-pull-popover",
        };
        popover::anchored_menu_below_end(
            SharedString::from(format!("{id}-menu")),
            popover::popover_card(&popup)
                .id(id)
                .debug_selector(move || id.into())
                .w(px(300.0))
                .p(px(8.0))
                .flex()
                .flex_col()
                .gap(px(10.0))
                .text_size(px(12.0))
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .id(SharedString::from(format!("{id}-text")))
                        .debug_selector(move || format!("{id}-text"))
                        .text_color(if failed { popup.danger } else { popup.text })
                        .child(text),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(6.0))
                        .when(confirm, |el| {
                            el.child(
                                accent_button("git-push-confirm", "Push", true, &popup).on_click(
                                    cx.listener(|tab, _, _, cx| tab.start_sync(SyncKind::Push, cx)),
                                ),
                            )
                        })
                        .child(
                            small_button("git-sync-close", "Close", false, &popup).on_click(
                                cx.listener(|tab, _, _, cx| {
                                    tab.sync_menu = None;
                                    cx.notify();
                                }),
                            ),
                        ),
                )
                .on_mouse_down_out(cx.listener(|tab, _, _, cx| {
                    tab.sync_menu = None;
                    cx.notify();
                }))
                .into_any_element(),
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_section(
        &self,
        id: &'static str,
        title: &str,
        files: &[&WorkingTreeFile],
        staged: bool,
        empty: &'static str,
        writes: Option<GitAgentBusyPolicy>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let all = if staged {
            WorkingTreeAction::UnstageAll
        } else {
            WorkingTreeAction::StageAll
        };
        let section_pending = self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.target == PendingTarget::All { staged });
        let header_action = writes.filter(|_| !files.is_empty()).map(|policy| {
            let enabled = self.allows(&all, policy, cx);
            let selector = format!("{id}-all");
            div()
                .id(SharedString::from(selector.clone()))
                .debug_selector(move || selector.clone())
                .role(gpui::Role::Button)
                .aria_label(if staged { "Unstage all" } else { "Stage all" })
                .ml_auto()
                .px(px(6.0))
                .rounded(px(5.0))
                .text_size(px(11.0))
                .text_color(theme.text_muted)
                .child(if staged { "Unstage all" } else { "Stage all" })
                .map(|el| {
                    if enabled {
                        el.cursor_pointer()
                            .hover(|style| style.text_color(theme.text).bg(theme.ink(0.06)))
                            .on_click(cx.listener(move |tab, _, _, cx| {
                                tab.run_action(all.clone(), PendingTarget::All { staged }, cx);
                            }))
                    } else {
                        el.opacity(0.45)
                    }
                })
        });
        let mut rows: Vec<AnyElement> = Vec::with_capacity(files.len());
        for (index, file) in files.iter().enumerate() {
            let confirming = !staged
                && matches!(&self.confirm, Some(Confirm::Discard { path, .. }) if *path == file.path);
            rows.push(if confirming {
                self.render_discard_confirm(file, theme, cx)
            } else {
                self.render_file_row(id, index, file, staged, writes, section_pending, theme, cx)
            });
        }
        div()
            .id(id)
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(30.0))
                    .px(px(12.0))
                    .flex()
                    .items_center()
                    .text_size(px(12.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text_muted)
                    .child(SharedString::from(format!("{title} ({})", files.len())))
                    .children(header_action),
            )
            .when(files.is_empty(), |el| {
                el.child(
                    div()
                        .h(px(26.0))
                        .px(px(12.0))
                        .flex()
                        .items_center()
                        .text_size(px(12.0))
                        .text_color(theme.text_muted.opacity(0.6))
                        .child(empty),
                )
            })
            .children(rows)
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_file_row(
        &self,
        section: &'static str,
        index: usize,
        file: &WorkingTreeFile,
        staged: bool,
        writes: Option<GitAgentBusyPolicy>,
        section_pending: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = if staged { file.index } else { file.worktree };
        let delta = if staged { file.staged } else { file.unstaged };
        let color = state_color(state, theme);
        let (dir, name) = match file.path.rsplit_once('/') {
            Some((dir, name)) => (format!("{dir}/"), name.to_owned()),
            None => (String::new(), file.path.clone()),
        };
        let open = self
            .expanded
            .as_ref()
            .is_some_and(|open| open.staged == staged && open.path == file.path);
        let pending = section_pending
            || self.pending.as_ref().is_some_and(|pending| {
                pending.target
                    == PendingTarget::File {
                        staged,
                        path: file.path.clone(),
                    }
            });
        let selector = format!("{section}-row-{index}");
        let count_selector = format!("{section}-count-{index}");
        let group = SharedString::from(selector.clone());
        let toggle = writes.map(|policy| {
            let path = file.path.clone();
            let action = if staged {
                WorkingTreeAction::Unstage {
                    paths: vec![path.clone()],
                }
            } else {
                WorkingTreeAction::Stage {
                    paths: vec![path.clone()],
                }
            };
            let enabled = self.allows(&action, policy, cx);
            let label = if staged { "Unstage" } else { "Stage" };
            let selector = format!("{section}-toggle-{index}");
            div()
                .id(SharedString::from(selector.clone()))
                .debug_selector(move || selector.clone())
                .role(gpui::Role::CheckBox)
                .aria_label(SharedString::from(format!("{label} {}", file.path)))
                .tooltip(text_tooltip(label))
                .size(px(14.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.0))
                .border_1()
                .border_color(if staged {
                    theme.accent_strong
                } else {
                    theme.hairline(0.2)
                })
                .when(staged, |el| {
                    el.bg(theme.accent_strong).child(
                        icon(icons::LUCIDE_CHECK)
                            .size(px(10.0))
                            .text_color(theme.on_accent),
                    )
                })
                .map(|el| {
                    if enabled {
                        el.cursor_pointer()
                            .hover(|style| style.border_color(theme.accent_strong))
                            .on_click(cx.listener(move |tab, _, _, cx| {
                                cx.stop_propagation();
                                tab.run_action(
                                    action.clone(),
                                    PendingTarget::File {
                                        staged,
                                        path: path.clone(),
                                    },
                                    cx,
                                );
                            }))
                    } else {
                        el.opacity(0.45).on_click(|_, _, cx| cx.stop_propagation())
                    }
                })
        });
        let commits = self.commits_supported(cx);
        let discard = writes.filter(|_| !staged && commits).map(|policy| {
            let path = file.path.clone();
            let enabled = file.worktree != GitFileState::Unmerged
                && !path.ends_with('/')
                && self.allows(
                    &WorkingTreeAction::Discard {
                        paths: vec![path.clone()],
                    },
                    policy,
                    cx,
                );
            let selector = format!("{section}-discard-{index}");
            div()
                .id(SharedString::from(selector.clone()))
                .debug_selector(move || selector.clone())
                .role(gpui::Role::Button)
                .aria_label(SharedString::from(format!(
                    "Discard changes to {}",
                    file.path
                )))
                .tooltip(text_tooltip("Discard changes"))
                .size(px(20.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .child(
                    icon(icons::LUCIDE_ROTATE_CCW)
                        .size(px(12.0))
                        .text_color(theme.text_muted),
                )
                .map(|el| {
                    if enabled {
                        el.opacity(0.0)
                            .group_hover(group.clone(), |style| style.opacity(1.0))
                            .cursor_pointer()
                            .hover(|style| style.bg(theme.danger.opacity(0.12)))
                            .on_click(cx.listener(move |tab, _, window, cx| {
                                cx.stop_propagation();
                                let Some(revision) =
                                    tab.tree.as_ref().map(|tree| tree.revision.clone())
                                else {
                                    return;
                                };
                                tab.open_confirm(
                                    Confirm::Discard {
                                        path: path.clone(),
                                        revision,
                                    },
                                    window,
                                    cx,
                                );
                            }))
                    } else {
                        el.opacity(0.0)
                            .group_hover(group.clone(), |style| style.opacity(0.3))
                            .on_click(|_, _, cx| cx.stop_propagation())
                    }
                })
        });
        let path = file.path.clone();
        div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector.clone())
            .role(gpui::Role::Button)
            .aria_label(SharedString::from(format!(
                "{} {}",
                if staged { "Staged" } else { "Changed" },
                file.path
            )))
            .aria_expanded(open)
            .group(group.clone())
            .h(px(26.0))
            .px(px(12.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .text_size(px(12.0))
            .when(open, |el| el.bg(theme.ink(0.04)))
            .when(pending, |el| el.opacity(0.5))
            .hover(|style| style.bg(theme.ink(0.04)))
            .when(writes.is_some(), |el| {
                el.cursor_pointer()
                    .on_click(cx.listener(move |tab, _, _, cx| {
                        tab.toggle_diff(staged, path.clone(), cx);
                    }))
            })
            .children(toggle)
            .child(
                div()
                    .w(px(12.0))
                    .flex_none()
                    .font_family(theme.font_mono.clone())
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(color)
                    .child(status_letter(state)),
            )
            .child(div().flex_1().min_w_0().truncate().child(
                gpui::StyledText::new(SharedString::from(format!("{dir}{name}"))).with_highlights(
                    [(
                        0..dir.len(),
                        gpui::HighlightStyle {
                            color: Some(theme.text_muted),
                            ..Default::default()
                        },
                    )],
                ),
            ))
            .child(
                div()
                    .id(SharedString::from(count_selector.clone()))
                    .debug_selector(move || count_selector.clone())
                    .flex_none()
                    .flex()
                    .gap(px(4.0))
                    .font_family(theme.font_mono.clone())
                    .text_size(px(11.0))
                    .map(|el| match delta {
                        Some(delta) if delta.binary => {
                            el.text_color(theme.text_muted).child("binary")
                        }
                        Some(delta) => el
                            .child(
                                div()
                                    .text_color(theme.success)
                                    .child(SharedString::from(format!("+{}", delta.additions))),
                            )
                            .child(
                                div()
                                    .text_color(theme.danger)
                                    .child(SharedString::from(format!("-{}", delta.deletions))),
                            ),
                        None => el,
                    }),
            )
            .children(discard)
            .when(staged && writes.is_some(), |el| {
                el.child(div().size(px(20.0)).flex_none())
            })
            .into_any_element()
    }

    fn render_discard_confirm(
        &self,
        file: &WorkingTreeFile,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let untracked = file.worktree == GitFileState::Untracked;
        let (lead, tail) = if untracked {
            ("Delete ", "? It is untracked, so this cannot be undone.")
        } else if file.index != GitFileState::Unchanged {
            (
                "Discard the unstaged changes to ",
                "? Staged changes stay. This cannot be undone.",
            )
        } else {
            ("Discard changes to ", "? This cannot be undone.")
        };
        let name = file.path.clone();
        let text = format!("{lead}{name}{tail}");
        div()
            .id("git-discard-confirm")
            .debug_selector(|| "git-discard-confirm".into())
            .track_focus(&self.confirm_focus)
            .mx(px(12.0))
            .my(px(2.0))
            .px(px(10.0))
            .py(px(8.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(theme.danger.opacity(0.3))
            .flex()
            .items_center()
            .gap(px(8.0))
            .text_size(px(12.0))
            .text_color(theme.text)
            .child(div().flex_1().min_w_0().child(
                gpui::StyledText::new(SharedString::from(text)).with_highlights([(
                    lead.len()..lead.len() + name.len(),
                    gpui::HighlightStyle {
                        font_weight: Some(gpui::FontWeight::SEMIBOLD),
                        ..Default::default()
                    },
                )]),
            ))
            .child(
                small_button(
                    "git-discard-yes",
                    if untracked { "Delete" } else { "Discard" },
                    true,
                    theme,
                )
                .on_click(cx.listener(|tab, _, _, cx| tab.confirm_discard(cx))),
            )
            .child(
                small_button("git-discard-no", "Cancel", false, theme)
                    .on_click(cx.listener(|tab, _, _, cx| tab.cancel_confirm(cx))),
            )
            .into_any_element()
    }

    fn render_composer(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let tree = self.tree.as_ref();
        let policy = busy_policy(cx);
        let writes = self.source_control(cx).then_some(policy);
        let commits = writes.filter(|_| self.commits_supported(cx));
        let staged = tree.map(staged_files).unwrap_or_default();
        let changes = tree.map(changed_files).unwrap_or_default();
        let dirty = tree.is_some_and(|tree| !tree.files.is_empty());
        let mut notes: Vec<AnyElement> = Vec::new();
        if tree.is_some_and(|tree| tree.agent_active) {
            notes.push(
                render_agent_banner(policy, theme)
                    .mx(px(12.0))
                    .mt(px(8.0))
                    .into_any_element(),
            );
        }
        if let Some(sha) = tree.and_then(|tree| tree.cherry_pick_head.as_deref()) {
            notes.push(self.render_pick_banner(sha, policy, theme, cx));
        }
        if let Some(note) = self.stash_note.clone() {
            notes.push(
                div()
                    .id("git-stash-note")
                    .debug_selector(|| "git-stash-note".into())
                    .px(px(2.0))
                    .text_size(px(12.0))
                    .text_color(theme.warning)
                    .child(note)
                    .into_any_element(),
            );
        }
        let errors = [self.error.clone(), self.action_error.clone()];
        notes.extend(
            errors
                .into_iter()
                .flatten()
                .enumerate()
                .map(|(index, error)| {
                    let selector = format!("git-error-{index}");
                    div()
                        .id(SharedString::from(selector.clone()))
                        .debug_selector(move || selector.clone())
                        .px(px(2.0))
                        .text_size(px(12.0))
                        .text_color(theme.danger)
                        .child(error)
                        .into_any_element()
                }),
        );
        let chip = |id: &'static str, label: String, color: gpui::Hsla| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .role(gpui::Role::Button)
                .aria_expanded(self.tray_open)
                .flex_none()
                .h(px(crate::composer::PR_CHIP_HEIGHT))
                .px(px(8.0))
                .rounded(px(6.0))
                .flex()
                .items_center()
                .bg(theme.ink(if self.tray_open { 0.1 } else { 0.06 }))
                .text_size(px(11.0))
                .text_color(color)
                .cursor_pointer()
                .hover(|style| style.bg(theme.ink(0.1)))
                .child(SharedString::from(label))
                .on_click(cx.listener(|tab, _, _, cx| {
                    tab.tray_open = !tab.tray_open;
                    cx.notify();
                }))
        };
        let status: AnyElement = match tree {
            None => div()
                .px(px(2.0))
                .text_size(px(12.0))
                .text_color(theme.text_muted)
                .child(if self.error.is_some() {
                    "Could not read the working tree."
                } else {
                    "Reading the working tree…"
                })
                .into_any_element(),
            Some(tree) if !dirty => div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .id("git-clean-status")
                        .debug_selector(|| "git-clean-status".into())
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .px(px(2.0))
                        .text_size(px(12.0))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(clean_status(tree))),
                )
                .children(commits.map(|policy| self.render_stash_button(tree, policy, theme, cx)))
                .into_any_element(),
            Some(_) => div()
                .flex()
                .flex_wrap()
                .gap(px(5.0))
                .child(chip(
                    "git-chip-staged",
                    format!("{} staged", staged.len()),
                    theme.success,
                ))
                .child(chip(
                    "git-chip-changed",
                    format!("{} changed", changes.len()),
                    theme.warning,
                ))
                .child(chip(
                    "git-chip-files",
                    if self.tray_open {
                        "▾ hide files".into()
                    } else {
                        "▸ files".into()
                    },
                    theme.text.opacity(0.8),
                ))
                .into_any_element(),
        };
        let tray = tree.filter(|_| dirty && self.tray_open).map(|tree| {
            let checkout = self.space.as_ref().and_then(|space| {
                checkout_options(self.state.read(cx), space)
                    .into_iter()
                    .find(|option| option.path.as_deref() == self.checkout_path.as_deref())
            });
            let mut detail = vec![
                tree.branch
                    .clone()
                    .unwrap_or_else(|| "detached HEAD".into()),
            ];
            if let Some(option) = &checkout {
                detail.push(if option.path.is_none() {
                    "main checkout".into()
                } else {
                    "agent worktree".into()
                });
            }
            let stash = commits.map(|policy| self.render_stash_button(tree, policy, theme, cx));
            div()
                .id("git-tray")
                .debug_selector(|| "git-tray".into())
                .flex()
                .flex_col()
                .mx(px(-6.0))
                .pb(px(6.0))
                .border_b_1()
                .border_color(theme.hairline(0.07))
                .child(
                    div()
                        .h(px(30.0))
                        .px(px(8.0))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(theme.text)
                                .child("Working tree"),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.0))
                                .text_color(theme.text_muted)
                                .child(SharedString::from(detail.join(" · "))),
                        )
                        .children(stash),
                )
                .when(tree.truncated, |el| {
                    el.child(
                        div()
                            .px(px(8.0))
                            .text_size(px(12.0))
                            .text_color(theme.warning)
                            .child("Partial snapshot: only the first files are listed."),
                    )
                })
                .child(
                    div()
                        .id("git-working-tree-files")
                        .max_h(px(TRAY_MAX_HEIGHT))
                        .overflow_y_scroll()
                        .child(self.render_section(
                            "git-staged",
                            "Staged",
                            &staged,
                            true,
                            "No staged files",
                            writes,
                            theme,
                            cx,
                        ))
                        .child(self.render_section(
                            "git-changes",
                            "Changes",
                            &changes,
                            false,
                            "No changes",
                            writes,
                            theme,
                            cx,
                        )),
                )
        });
        let surface = div()
            .id("git-composer")
            .debug_selector(|| "git-composer".into())
            .rounded(px(crate::composer::COMPOSER_RADIUS))
            .border_1()
            .border_color(theme.composer_surface_border())
            .bg(theme.composer_surface_bg())
            .when(!theme.is_frost(), |el| el.shadow_lg())
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|tab, _, window, cx| {
                    if !tab.writing() && !tab.model_picker.read(cx).is_open() {
                        window.focus(&tab.message.focus_handle(cx), cx);
                        window.prevent_default();
                    }
                }),
            )
            .px(px(14.0))
            .pt(px(10.0))
            .pb(px(8.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .children(tray)
            .children(notes)
            .child(status)
            .child(self.render_commit_box(staged.len(), commits, theme, cx));
        div()
            .id("git-composer-dock")
            .absolute()
            .left_0()
            .right_0()
            .bottom(px(crate::pull_request_detail::NAV_CLEARANCE))
            .child(crate::settings::widgets::page_column().pt_0().pb_0().child(
                crate::dock_orb::float(
                    "git-composer-rise",
                    &self.float,
                    cx.entity_id(),
                    cx.listener(|tab, _, window, cx| {
                        if tab.composer_idle(cx) {
                            tab.fold_composer(window, cx);
                        }
                    }),
                    crate::frost::frosted(
                        crate::composer::COMPOSER_RADIUS,
                        crate::frost::MENU_BLUR,
                        surface,
                    ),
                ),
            ))
            .into_any_element()
    }

    fn render_file_diff(
        &self,
        open: &Expanded,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id("git-diff-view")
            .debug_selector(|| "git-diff-view".into())
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(40.0))
                    .px(px(12.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .border_b_1()
                    .border_color(theme.hairline(0.07))
                    .child(
                        back_to_history("git-diff-back", theme).on_click(cx.listener(
                            |tab, _, _, cx| {
                                tab.expanded = None;
                                tab.diff_task = None;
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.0))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child(SharedString::from(open.path.clone())),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(12.0))
                            .text_color(theme.text_muted)
                            .child(if open.staged { "Staged" } else { "Unstaged" }),
                    ),
            )
            .child(
                div()
                    .id("git-diff-scroll")
                    .debug_selector(|| "git-diff-scroll".into())
                    .flex_1()
                    .min_h_0()
                    .child(
                        list(open.list.clone(), cx.processor(Self::render_file_diff_row))
                            .size_full()
                            .pt(px(8.0))
                            .pb(px(self.bottom_clearance())),
                    ),
            )
            .into_any_element()
    }

    fn render_file_diff_row(
        &mut self,
        ix: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some((open, row)) = self
            .expanded
            .as_ref()
            .and_then(|open| Some((open, *open.rows.get(ix)?)))
        else {
            return div().into_any_element();
        };
        let (file, error) = match &open.view {
            Loadable::Ready((_, parsed)) => (parsed.as_ref(), None),
            Loadable::Error(error) => (None, Some(SharedString::from(error.clone()))),
            Loadable::Idle | Loadable::Loading => (None, None),
        };
        render_diff_row(
            row,
            file,
            error,
            (ix == 0).then(|| "git-file-diff".to_string()),
            ix + 1 == open.rows.len(),
            Theme::of(cx),
        )
    }

    fn render_stash_button(
        &self,
        tree: &WorkingTree,
        policy: GitAgentBusyPolicy,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let menu = self
            .stash_menu
            .then(|| self.render_stash_menu(tree, policy, theme, cx));
        div()
            .id("git-stash-button")
            .debug_selector(|| "git-stash-button".into())
            .role(gpui::Role::Button)
            .aria_label("Stash")
            .ml_auto()
            .relative()
            .flex_none()
            .h(px(24.0))
            .px(px(8.0))
            .rounded(px(6.0))
            .flex()
            .items_center()
            .gap(px(4.0))
            .text_size(px(12.0))
            .text_color(theme.text_muted)
            .cursor_pointer()
            .hover(|style| style.text_color(theme.text).bg(theme.ink(0.06)))
            .when(self.stash_menu, |el| el.bg(theme.ink(0.06)))
            .child(
                icon(icons::ARCHIVE_UP_MINIMALISTIC)
                    .size(px(13.0))
                    .text_color(theme.text_muted),
            )
            .child("Stash")
            .child(
                icon(icons::ALT_ARROW_DOWN)
                    .size(px(12.0))
                    .text_color(theme.text_muted),
            )
            .on_click(cx.listener(|tab, _, _, cx| {
                tab.stash_menu = !tab.stash_menu;
                if matches!(tab.confirm, Some(Confirm::DropStash { .. })) {
                    tab.confirm = None;
                }
                cx.notify();
            }))
            .children(menu)
            .into_any_element()
    }

    fn render_stash_menu(
        &self,
        tree: &WorkingTree,
        policy: GitAgentBusyPolicy,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let popup = theme.for_popup();
        let files = tree.files.len();
        let push = WorkingTreeAction::StashPush { message: None };
        let push_enabled = files > 0 && self.allows(&push, policy, cx);
        let push_label = if files == 1 {
            "Stash all (1 file)".to_string()
        } else {
            format!("Stash all ({files} files)")
        };
        let now = chrono::Utc::now();
        let mut rows: Vec<AnyElement> = Vec::with_capacity(tree.stashes.len());
        for (index, stash) in tree.stashes.iter().enumerate() {
            let name = format!("stash@{{{index}}}");
            let confirming =
                matches!(&self.confirm, Some(Confirm::DropStash { oid }) if *oid == stash.oid);
            if confirming {
                rows.push(
                    div()
                        .id("git-stash-drop-confirm")
                        .debug_selector(|| "git-stash-drop-confirm".into())
                        .track_focus(&self.confirm_focus)
                        .my(px(2.0))
                        .px(px(8.0))
                        .py(px(6.0))
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(popup.danger.opacity(0.3))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .text_size(px(12.0))
                        .child(div().flex_1().min_w_0().child(SharedString::from(format!(
                            "Delete {name}? This cannot be undone."
                        ))))
                        .child(
                            small_button("git-stash-drop-yes", "Delete", true, &popup)
                                .on_click(cx.listener(|tab, _, _, cx| tab.confirm_drop_stash(cx))),
                        )
                        .child(
                            small_button("git-stash-drop-no", "Cancel", false, &popup)
                                .on_click(cx.listener(|tab, _, _, cx| tab.cancel_confirm(cx))),
                        )
                        .into_any_element(),
                );
                continue;
            }
            let pop = WorkingTreeAction::StashPop {
                oid: stash.oid.clone(),
            };
            let drop = WorkingTreeAction::StashDrop {
                oid: stash.oid.clone(),
            };
            let pop_enabled = self.allows(&pop, policy, cx);
            let drop_enabled = self.allows(&drop, policy, cx);
            let when = stash
                .created_at
                .map(|at| super::relative_time(at, now))
                .unwrap_or_default();
            let row_button = |selector: String, label: &'static str, enabled: bool| {
                button_shell(selector, label.into(), 22.0, 8.0, 6.0)
                    .border_1()
                    .border_color(popup.hairline(0.12))
                    .map(|el| {
                        if enabled {
                            el.cursor_pointer().hover(|style| style.bg(popup.ink(0.06)))
                        } else {
                            el.opacity(0.45)
                        }
                    })
            };
            let oid = stash.oid.clone();
            rows.push(
                div()
                    .id(SharedString::from(format!("git-stash-row-{index}")))
                    .py(px(4.0))
                    .px(px(4.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_size(px(12.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .truncate()
                                    .text_color(popup.text)
                                    .child(SharedString::from(format!(
                                        "{name} · {}",
                                        stash.message
                                    ))),
                            )
                            .when(!when.is_empty(), |el| {
                                el.child(
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(popup.text_muted)
                                        .child(SharedString::from(when)),
                                )
                            }),
                    )
                    .child(
                        row_button(format!("git-stash-pop-{index}"), "Pop", pop_enabled).when(
                            pop_enabled,
                            |el| {
                                el.on_click(cx.listener(move |tab, _, _, cx| {
                                    tab.run_action(pop.clone(), PendingTarget::Stash, cx);
                                }))
                            },
                        ),
                    )
                    .child(
                        row_button(format!("git-stash-drop-{index}"), "Delete", drop_enabled).when(
                            drop_enabled,
                            |el| {
                                el.on_click(cx.listener(move |tab, _, window, cx| {
                                    tab.open_confirm(
                                        Confirm::DropStash { oid: oid.clone() },
                                        window,
                                        cx,
                                    );
                                }))
                            },
                        ),
                    )
                    .into_any_element(),
            );
        }
        popover::anchored_menu_below_end(
            "git-stash-menu",
            popover::popover_card(&popup)
                .id("git-stash-card")
                .debug_selector(|| "git-stash-card".into())
                .w(px(300.0))
                .p(px(8.0))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .id("git-stash-push")
                        .debug_selector(|| "git-stash-push".into())
                        .role(gpui::Role::Button)
                        .aria_label(SharedString::from(push_label.clone()))
                        .h(px(30.0))
                        .rounded(px(7.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(12.0))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .bg(popup.accent_strong)
                        .text_color(popup.on_accent)
                        .child(SharedString::from(push_label))
                        .map(|el| {
                            if push_enabled {
                                el.cursor_pointer()
                                    .hover(|style| style.opacity(0.9))
                                    .on_click(cx.listener(move |tab, _, _, cx| {
                                        tab.run_action(push.clone(), PendingTarget::Stash, cx);
                                    }))
                            } else {
                                el.opacity(0.45)
                            }
                        }),
                )
                .child(popover::menu_heading(&popup, "Stashed"))
                .when(rows.is_empty(), |el| {
                    el.child(
                        div()
                            .px(px(8.0))
                            .pb(px(4.0))
                            .text_size(px(12.0))
                            .text_color(popup.text_muted)
                            .child("No stashes"),
                    )
                })
                .children(rows)
                .on_mouse_down_out(cx.listener(|tab, _, _, cx| {
                    tab.stash_menu = false;
                    if matches!(tab.confirm, Some(Confirm::DropStash { .. })) {
                        tab.confirm = None;
                    }
                    cx.notify();
                }))
                .into_any_element(),
            None,
        )
    }

    fn render_commit_box(
        &self,
        staged: usize,
        writes: Option<GitAgentBusyPolicy>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let committing = self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.target == PendingTarget::Commit);
        let enabled =
            |kind: CommitKind| writes.is_some_and(|policy| self.can_commit(kind, policy, cx));
        let subject = subject_chars(self.message.read(cx).text());
        let label = if committing {
            "Committing…".to_string()
        } else if staged == 1 {
            "Commit 1 file".to_string()
        } else {
            format!("Commit {staged} files")
        };
        let main_enabled = enabled(CommitKind::Staged);
        let writing = self.writing();
        let write_enabled = writing || self.can_write(cx);
        let write_label = if writing {
            "Writing… Esc to stop"
        } else if self.written.is_some() {
            "Rewrite"
        } else {
            "Write with AI"
        };
        let write_icon: AnyElement = if writing {
            icon(icons::LUCIDE_REFRESH_CW)
                .size(px(12.0))
                .text_color(theme.text_muted)
                .with_animation(
                    "git-write-spin",
                    Animation::new(Duration::from_millis(900)).repeat(),
                    |svg, delta| {
                        svg.with_transformation(gpui::Transformation::rotate(percentage(delta)))
                    },
                )
                .into_any_element()
        } else {
            icon(icons::MAGIC_STICK_3)
                .size(px(12.0))
                .text_color(theme.text)
                .into_any_element()
        };
        let write_button = div()
            .id("git-write-ai")
            .debug_selector(|| "git-write-ai".into())
            .role(gpui::Role::Button)
            .aria_label(write_label)
            .flex_none()
            .h(px(24.0))
            .px(px(8.0))
            .rounded(px(7.0))
            .border_1()
            .flex()
            .items_center()
            .gap(px(6.0))
            .text_size(px(12.0))
            .map(|el| {
                if writing {
                    el.border_color(theme.hairline(0.12))
                        .text_color(theme.text_muted)
                } else {
                    el.border_color(theme.accent.opacity(0.25))
                        .bg(theme.accent.opacity(0.08))
                        .text_color(theme.accent)
                }
            })
            .child(write_icon)
            .child(write_label)
            .map(|el| {
                if write_enabled {
                    el.cursor_pointer()
                        .hover(|style| style.bg(theme.accent.opacity(0.15)))
                        .on_click(cx.listener(|tab, _, window, cx| tab.write_message(window, cx)))
                } else {
                    el.opacity(0.45).tooltip(text_tooltip("Stage files first"))
                }
            });
        let model_chip = div()
            .id("git-write-model")
            .min_w_0()
            .flex()
            .items_center()
            .when(
                self.space
                    .as_ref()
                    .is_some_and(|space| space.device.is_none()),
                |el| el.child(self.model_picker.clone()),
            );
        let settings_button = div()
            .id("git-write-settings")
            .debug_selector(|| "git-write-settings".into())
            .role(gpui::Role::Button)
            .aria_label("Instruction and default model…")
            .flex_none()
            .size(px(24.0))
            .rounded(px(7.0))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|style| style.bg(theme.ink(0.06)))
            .tooltip(text_tooltip("Instruction and default model…"))
            .child(
                icon(icons::SETTINGS)
                    .size(px(14.0))
                    .text_color(theme.text_muted),
            )
            .on_click(|_, window, cx| {
                window.dispatch_action(Box::new(OpenSourceControlSettings), cx)
            });
        let current_text = self.message.read(cx).text().to_string();
        let message_body: AnyElement = if writing {
            div()
                .id("git-write-preview")
                .debug_selector(|| "git-write-preview".into())
                .min_h(px(MESSAGE_MIN_HEIGHT))
                .max_h(px(160.0))
                .px(px(2.0))
                .text_size(px(13.0))
                .text_color(theme.text_muted)
                .flex()
                .flex_row()
                .flex_wrap()
                .content_start()
                .items_end()
                .when(!current_text.is_empty(), |el| {
                    el.child(SharedString::from(current_text))
                })
                .child(
                    div()
                        .ml(px(1.0))
                        .w(px(7.0))
                        .h(px(14.0))
                        .bg(theme.accent)
                        .with_animation(
                            "git-write-caret",
                            Animation::new(Duration::from_millis(900)).repeat(),
                            |caret, delta| caret.opacity(if delta < 0.5 { 1.0 } else { 0.0 }),
                        ),
                )
                .into_any_element()
        } else {
            div()
                .min_h(px(MESSAGE_MIN_HEIGHT))
                .max_h(px(160.0))
                .px(px(2.0))
                .text_size(px(13.0))
                .child(self.message.clone())
                .into_any_element()
        };
        let menu = self.commit_menu.then(|| {
            let popup = theme.for_popup();
            let row = |selector: &'static str,
                       kind: CommitKind,
                       title: &'static str,
                       detail: &'static str| {
                let on = enabled(kind);
                popover::menu_row(&popup, false, selector)
                    .id(selector)
                    .debug_selector(move || selector.to_string())
                    .role(gpui::Role::MenuItem)
                    .aria_label(title)
                    .child(
                        div().flex().flex_col().child(div().child(title)).child(
                            div()
                                .text_size(px(11.0))
                                .text_color(popup.text_muted)
                                .child(detail),
                        ),
                    )
                    .map(|el| {
                        if on {
                            el.cursor_pointer()
                                .on_click(cx.listener(move |tab, _, _, cx| tab.commit(kind, cx)))
                        } else {
                            el.opacity(0.45)
                        }
                    })
            };
            popover::anchored_menu_above_end(
                "git-commit-menu",
                popover::popover_card(&popup)
                    .w(px(240.0))
                    .flex()
                    .flex_col()
                    .child(row(
                        "git-commit-menu-commit",
                        CommitKind::Staged,
                        "Commit",
                        "Commit the staged files",
                    ))
                    .child(row(
                        "git-commit-menu-amend",
                        CommitKind::Amend,
                        "Amend",
                        "Fold into the previous commit (not pushed)",
                    ))
                    .child(row(
                        "git-commit-menu-all",
                        CommitKind::All,
                        "Commit all",
                        "Stage everything, then commit",
                    ))
                    .on_mouse_down_out(cx.listener(|tab, _, _, cx| {
                        tab.commit_menu = false;
                        cx.notify();
                    }))
                    .into_any_element(),
                None,
            )
        });
        let commit_tip = format!("{label}  ⌘↵");
        let commit_button = div()
            .id("git-commit-button")
            .debug_selector(|| "git-commit-button".into())
            .role(gpui::Role::Button)
            .aria_label(SharedString::from(label))
            .tooltip(text_tooltip(commit_tip))
            .size(px(28.0))
            .flex_none()
            .rounded_full()
            .bg(theme.text)
            .flex()
            .items_center()
            .justify_center()
            .child(
                icon(icons::LUCIDE_GIT_COMMIT_HORIZONTAL)
                    .size(px(16.0))
                    .text_color(theme.bg),
            )
            .map(|el| {
                if main_enabled {
                    el.cursor_pointer()
                        .hover(|style| style.opacity(0.85))
                        .on_click(cx.listener(|tab, _, _, cx| tab.commit(CommitKind::Staged, cx)))
                } else {
                    el.opacity(0.35)
                }
            });
        let more_button = div()
            .id("git-commit-more")
            .debug_selector(|| "git-commit-more".into())
            .role(gpui::Role::Button)
            .aria_label("More commit options")
            .relative()
            .size(px(24.0))
            .flex_none()
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .when(self.commit_menu, |el| el.bg(theme.ink(0.06)))
            .child(
                icon(icons::LUCIDE_CHEVRON_DOWN)
                    .size(px(14.0))
                    .text_color(theme.text_muted),
            )
            .when(writes.is_some() && !committing, |el| {
                el.cursor_pointer()
                    .hover(|style| style.bg(theme.ink(0.06)))
                    .on_click(cx.listener(|tab, _, _, cx| {
                        tab.commit_menu = !tab.commit_menu;
                        cx.notify();
                    }))
            })
            .children(menu);
        let note = |id: &'static str, color: gpui::Hsla, text: SharedString| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .px(px(2.0))
                .text_size(px(11.0))
                .text_color(color)
                .child(text)
        };
        div()
            .id("git-commit-box")
            .flex()
            .flex_col()
            .gap(px(6.0))
            .when(committing, |el| el.opacity(0.6))
            .child(
                div()
                    .id("git-commit-message")
                    .debug_selector(|| "git-commit-message".into())
                    .role(gpui::Role::TextInput)
                    .aria_label("Commit message")
                    .when(writing, |el| el.track_focus(&self.write_focus))
                    .capture_action(cx.listener(|tab, _: &crate::composer::Submit, window, cx| {
                        cx.stop_propagation();
                        tab.message.update(cx, |input, cx| {
                            input.replace_text_in_range(None, "\n", window, cx)
                        });
                    }))
                    .on_key_down(cx.listener(|tab, event: &KeyDownEvent, _, cx| {
                        let modifiers = &event.keystroke.modifiers;
                        if event.keystroke.key == "enter"
                            && (modifiers.platform || modifiers.control)
                        {
                            cx.stop_propagation();
                            tab.commit(CommitKind::Staged, cx);
                        }
                    }))
                    .on_click(cx.listener(|tab, _, window, cx| {
                        if !tab.writing() {
                            window.focus(&tab.message.focus_handle(cx), cx);
                        }
                    }))
                    .child(message_body),
            )
            .child(
                div()
                    .h(px(30.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_size(px(12.0))
                    .text_color(theme.text_muted)
                    .child(write_button)
                    .child(model_chip)
                    .child(settings_button)
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("git-commit-count")
                            .debug_selector(|| "git-commit-count".into())
                            .font_family(theme.font_mono.clone())
                            .text_size(px(11.0))
                            .text_color(if subject > SUBJECT_LIMIT {
                                theme.warning
                            } else {
                                theme.text_muted
                            })
                            .child(SharedString::from(format!("{subject}/{SUBJECT_LIMIT}"))),
                    )
                    .child(more_button)
                    .child(commit_button),
            )
            .when(self.model_override.is_some(), |el| {
                el.child(note(
                    "git-write-model-override",
                    theme.text_muted,
                    "The model picked above is for this commit only.".into(),
                ))
            })
            .when_some(self.written.as_ref().filter(|_| !writing), |el, written| {
                el.child(note(
                    "git-write-note",
                    theme.text_muted,
                    format!(
                        "Written by {} · {} from your instruction and the staged diff. Edit it before you commit.",
                        crate::composer::harness_display_name(written.harness),
                        written.model
                    )
                    .into(),
                ))
            })
            .when_some(self.write_error.clone(), |el, error| {
                el.child(note(
                    "git-write-error",
                    theme.danger,
                    format!(
                        "Could not write a message: {}. Pick another model or write it yourself.",
                        error.trim_end_matches('.')
                    )
                    .into(),
                ))
            })
            .when_some(self.last_commit.as_ref(), |el, commit| {
                let short = short_sha(&commit.sha);
                let text = match &commit.branch {
                    Some(branch) => format!("Committed {short} to {branch}"),
                    None => format!("Committed {short}"),
                };
                el.child(note("git-last-commit", theme.text_muted, text.into()).truncate())
            })
            .into_any_element()
    }
}

impl GitTab {
    fn render_pick_banner(
        &self,
        sha: &str,
        policy: GitAgentBusyPolicy,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let enabled = self.allows(&WorkingTreeAction::AbortCherryPick, policy, cx);
        div()
            .id("git-cherry-pick-banner")
            .debug_selector(|| "git-cherry-pick-banner".into())
            .mx(px(12.0))
            .mt(px(8.0))
            .px(px(8.0))
            .py(px(6.0))
            .rounded(px(6.0))
            .bg(theme.warning.opacity(0.1))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .text_size(px(12.0))
            .text_color(theme.warning)
            .child(SharedString::from(format!(
                "A cherry-pick of {} is in progress. Resolve the conflicts, stage the files and commit to finish it, or abort it.",
                short_sha(sha)
            )))
            .child(
                div().flex().child(
                    detail_button("git-cherry-pick-abort", "Abort cherry-pick", enabled, theme)
                        .when(enabled, |el| {
                            el.on_click(cx.listener(|tab, _, _, cx| {
                                tab.run_action(
                                    WorkingTreeAction::AbortCherryPick,
                                    PendingTarget::AbortPick,
                                    cx,
                                );
                            }))
                        }),
                ),
            )
            .into_any_element()
    }

    fn render_detail_panel(
        &self,
        detail: &CommitDetail,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let commit = &detail.commit;
        let sticky = self.render_sticky_commit_file(detail, theme, cx);
        div()
            .id("git-commit-detail")
            .debug_selector(|| "git-commit-detail".into())
            .map(|el| {
                if self.compact {
                    el.size_full()
                } else {
                    el.relative()
                        .w(px(crate::settings::git_commit_panel_width(cx)))
                        .min_w(px(crate::settings::GIT_COMMIT_PANEL_MIN))
                        .h_full()
                        .border_l_1()
                }
            })
            .border_color(theme.hairline(0.07))
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(40.0))
                    .px(px(12.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .border_b_1()
                    .border_color(theme.hairline(0.07))
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child("Commit"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(theme.font_mono.clone())
                            .text_size(px(12.0))
                            .text_color(theme.text_muted)
                            .child(SharedString::from(short_sha(&commit.sha))),
                    )
                    .child(
                        div()
                            .id("git-commit-close")
                            .debug_selector(|| "git-commit-close".into())
                            .role(gpui::Role::Button)
                            .aria_label("Close the commit")
                            .tooltip(text_tooltip("Close the commit"))
                            .size(px(22.0))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.0))
                            .cursor_pointer()
                            .hover(|style| style.bg(theme.ink(0.06)))
                            .child(
                                icon(icons::CLOSE)
                                    .size(px(12.0))
                                    .text_color(theme.text_muted),
                            )
                            .on_click(cx.listener(|tab, _, _, cx| tab.select_commit(None, cx))),
                    ),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(
                        list(detail.list.clone(), cx.processor(Self::render_detail_row))
                            .size_full()
                            .pb(px(12.0 + self.bottom_clearance())),
                    )
                    .children(sticky),
            )
            .when(!self.compact, |el| {
                el.child(self.render_panel_resize_handle(theme, cx))
            })
            .into_any_element()
    }

    fn render_panel_resize_handle(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let hover = theme.border_strong;
        div()
            .id("git-commit-resize")
            .debug_selector(|| "git-commit-resize".into())
            .absolute()
            .left(px(-3.0))
            .top_0()
            .bottom_0()
            .w(px(6.0))
            .cursor_col_resize()
            .hover(move |style| style.bg(hover.opacity(0.7)))
            .on_mouse_down(gpui::MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_drag(
                CommitPanelResize,
                |_, _point: gpui::Point<gpui::Pixels>, _, cx| {
                    cx.stop_propagation();
                    cx.new(|_| CommitPanelResizeGhost)
                },
            )
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|tab, event: &gpui::MouseUpEvent, _, cx| {
                    if event.click_count == 2 {
                        tab.set_panel_width(
                            crate::settings::GIT_COMMIT_PANEL_DEFAULT,
                            crate::settings::SavePolicy::Immediate,
                            cx,
                        );
                    }
                }),
            )
            .into_any_element()
    }

    fn resize_panel(
        &mut self,
        event: &gpui::DragMoveEvent<CommitPanelResize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let max = (f32::from(event.bounds.size.width) - HISTORY_KEEP_WIDTH)
            .min(crate::settings::GIT_COMMIT_PANEL_MAX)
            .max(crate::settings::GIT_COMMIT_PANEL_MIN);
        let width = f32::from(event.bounds.right() - event.event.position.x)
            .clamp(crate::settings::GIT_COMMIT_PANEL_MIN, max);
        self.set_panel_width(width, crate::settings::SavePolicy::Debounced, cx);
    }

    fn set_panel_width(
        &mut self,
        width: f32,
        policy: crate::settings::SavePolicy,
        cx: &mut Context<Self>,
    ) {
        if crate::settings::update(policy, cx, |settings| {
            settings.git_commit_panel_width = width;
        }) {
            cx.notify();
        }
    }

    fn render_sticky_commit_file(
        &self,
        detail: &CommitDetail,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (diff, _) = detail.diff.ready()?;
        let starts: Vec<usize> = detail
            .rows
            .iter()
            .enumerate()
            .filter_map(|(ix, row)| matches!(row, DetailRow::File(_)).then_some(ix))
            .collect();
        let ranges: Vec<std::ops::Range<usize>> = starts
            .iter()
            .enumerate()
            .map(|(n, &start)| start..starts.get(n + 1).copied().unwrap_or(detail.rows.len()))
            .collect();
        let top = detail.list.logical_scroll_top();
        let sticky =
            changes::sticky_file_header(&ranges, top.item_ix, top.offset_in_item.as_f32())?;
        let DetailRow::File(index) = *detail.rows.get(sticky.header_row)? else {
            return None;
        };
        let file = diff.files.get(index)?;
        if !detail.open_files.contains(&file.path) {
            return None;
        }
        let next_y = sticky.next_header_row.and_then(|row| {
            let bounds = detail.list.bounds_for_item(row)?;
            Some((bounds.origin.y - detail.list.viewport_bounds().origin.y).as_f32())
        });
        let offset = next_y.map_or(0.0, |y| (y - COMMIT_FILE_ROW_HEIGHT).min(0.0));
        let paint = changes::sticky_file_header_paint(theme);
        let row = div()
            .w_full()
            .bg(paint.rest_bg)
            .border_b_1()
            .border_color(paint.border)
            .child(self.render_commit_file(index, file, detail, true, theme, cx));
        let row = match paint.frost_tint {
            Some(tint) => div().w_full().bg(tint).child(row).into_any_element(),
            None => row.into_any_element(),
        };
        Some(
            div()
                .absolute()
                .top(px(offset))
                .left_0()
                .w_full()
                .child(crate::frost::frosted(
                    0.0,
                    changes::STICKY_FILE_HEADER_BLUR,
                    row,
                ))
                .into_any_element(),
        )
    }

    fn render_detail_row(
        &mut self,
        ix: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let Some((detail, row)) = self
            .detail
            .as_ref()
            .and_then(|detail| Some((detail, *detail.rows.get(ix)?)))
        else {
            return div().into_any_element();
        };
        let status = |text: SharedString, color: gpui::Hsla| {
            div()
                .px(px(12.0))
                .text_size(px(12.0))
                .text_color(color)
                .child(text)
                .into_any_element()
        };
        match (row, &detail.diff) {
            (DetailRow::Header, _) => self.render_commit_header(detail, &theme, cx),
            (DetailRow::File(index), Loadable::Ready((diff, _))) => {
                self.render_commit_file(index, &diff.files[index], detail, false, &theme, cx)
            }
            (DetailRow::Diff(index, row), Loadable::Ready((diff, parsed))) => {
                let path = &diff.files[index].path;
                let same_file = |at: Option<usize>| {
                    matches!(
                        at.and_then(|at| detail.rows.get(at)),
                        Some(DetailRow::Diff(other, _)) if *other == index
                    )
                };
                render_diff_row(
                    row,
                    parsed.iter().find(|parsed| &parsed.path == path),
                    None,
                    (!same_file(ix.checked_sub(1))).then(|| format!("git-commit-diff-{index}")),
                    !same_file(Some(ix + 1)),
                    &theme,
                )
            }
            (_, Loadable::Error(error)) => div()
                .id("git-commit-files-error")
                .debug_selector(|| "git-commit-files-error".into())
                .px(px(12.0))
                .text_size(px(12.0))
                .text_color(theme.danger)
                .child(error.clone())
                .into_any_element(),
            (_, Loadable::Ready(_)) => {
                status("No file changes".into(), theme.text_muted.opacity(0.6))
            }
            (_, Loadable::Idle | Loadable::Loading) => {
                status("Reading the commit…".into(), theme.text_muted)
            }
        }
    }

    fn render_commit_header(
        &self,
        detail: &CommitDetail,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let commit = &detail.commit;
        let tree = self.tree.as_ref();
        let pending = self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.target == PendingTarget::Detail);
        let checkout = self.checkout_action();
        let checkout_label = match &checkout {
            None => "Checked out".to_string(),
            Some(WorkingTreeAction::Checkout {
                target_ref,
                detach: false,
            }) => format!("Checkout {target_ref}"),
            Some(_) => "Checkout".to_string(),
        };
        let can_checkout = self.can_checkout(cx);
        let can_pick = self.can_cherry_pick(cx);
        let can_branch = detail.branch_form.is_none() && self.can_branch(false, cx);
        let refusal = (commit.parent_shas.len() > 1)
            .then_some("Merge commits cannot be cherry-picked here.")
            .into_iter()
            .chain(self.move_refusal(
                &WorkingTreeAction::CherryPick {
                    sha: commit.sha.clone(),
                },
                cx,
            ))
            .next()
            .filter(|_| tree.is_some());
        let copied = self.history.read(cx).copied_sha() == Some(commit.sha.as_str());
        let files: &[DiffFileSummary] = match detail.diff.ready() {
            Some((diff, _)) => &diff.files,
            None => &[],
        };
        let all_open = !files.is_empty() && detail.open_files.len() == files.len();
        let parents = if commit.parent_shas.is_empty() {
            "none".to_string()
        } else {
            commit
                .parent_shas
                .iter()
                .map(|sha| short_sha(sha))
                .collect::<Vec<_>>()
                .join("  ")
        };
        let meta = |name: &'static str, value: String| {
            div()
                .flex()
                .gap(px(10.0))
                .child(
                    div()
                        .w(px(60.0))
                        .flex_none()
                        .text_color(theme.text_muted)
                        .child(name),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_family(theme.font_mono.clone())
                        .text_size(px(11.0))
                        .text_color(theme.text)
                        .child(SharedString::from(value)),
                )
        };
        let confirm = match &self.confirm {
            Some(Confirm::Checkout { sha }) if *sha == commit.sha => Some((
                format!(
                    "Check out {} as a detached HEAD? New commits there belong to no branch until you create one.",
                    short_sha(sha)
                ),
                "Check out",
            )),
            Some(Confirm::CherryPick { sha }) if *sha == commit.sha => Some((
                format!(
                    "Cherry-pick {} onto {}? It adds a new commit with the same changes.",
                    short_sha(sha),
                    tree.and_then(|tree| tree.branch.clone())
                        .unwrap_or_else(|| "the detached HEAD".into())
                ),
                "Cherry-pick",
            )),
            _ => None,
        };
        let confirm = confirm.map(|(text, yes)| {
            div()
                .id("git-commit-confirm")
                .debug_selector(|| "git-commit-confirm".into())
                .track_focus(&self.confirm_focus)
                .px(px(10.0))
                .py(px(8.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(theme.warning.opacity(0.35))
                .flex()
                .flex_col()
                .gap(px(8.0))
                .text_size(px(12.0))
                .text_color(theme.text)
                .child(SharedString::from(text))
                .child(
                    div()
                        .flex()
                        .gap(px(6.0))
                        .child(
                            accent_button("git-commit-confirm-yes", yes, true, theme)
                                .on_click(cx.listener(|tab, _, _, cx| tab.confirm_detail(cx))),
                        )
                        .child(
                            small_button("git-commit-confirm-no", "Cancel", false, theme)
                                .on_click(cx.listener(|tab, _, _, cx| tab.cancel_confirm(cx))),
                        ),
                )
        });
        let branch_form = detail.branch_form.as_ref().map(|form| {
            let named = !form.name.read(cx).text().trim().is_empty();
            let checkout_allowed = self.can_branch(true, cx);
            let checked = form.checkout && checkout_allowed;
            let enabled = named && !pending && self.can_branch(checked, cx);
            div()
                .id("git-branch-form")
                .debug_selector(|| "git-branch-form".into())
                .capture_action(cx.listener(|tab, _: &crate::composer::Submit, _, cx| {
                    cx.stop_propagation();
                    tab.create_branch(cx);
                }))
                .px(px(10.0))
                .py(px(8.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(theme.hairline(0.12))
                .flex()
                .flex_col()
                .gap(px(8.0))
                .text_size(px(12.0))
                .text_color(theme.text)
                .child(SharedString::from(format!(
                    "New branch at {}",
                    short_sha(&commit.sha)
                )))
                .child(
                    div()
                        .id("git-branch-name")
                        .debug_selector(|| "git-branch-name".into())
                        .role(gpui::Role::TextInput)
                        .aria_label("Branch name")
                        .px(px(8.0))
                        .py(px(5.0))
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(theme.hairline(0.12))
                        .bg(theme.ink(0.02))
                        .text_size(px(13.0))
                        .child(form.name.clone()),
                )
                .child(
                    div()
                        .id("git-branch-checkout")
                        .debug_selector(|| "git-branch-checkout".into())
                        .role(gpui::Role::CheckBox)
                        .aria_label("Check it out")
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            div()
                                .size(px(14.0))
                                .rounded(px(4.0))
                                .border_1()
                                .border_color(if checked {
                                    theme.accent_strong
                                } else {
                                    theme.hairline(0.2)
                                })
                                .when(checked, |el| el.bg(theme.accent_strong))
                                .flex()
                                .items_center()
                                .justify_center()
                                .when(checked, |el| {
                                    el.child(
                                        icon(icons::LUCIDE_CHECK)
                                            .size(px(10.0))
                                            .text_color(theme.on_accent),
                                    )
                                }),
                        )
                        .child("Check it out")
                        .map(|el| {
                            if checkout_allowed {
                                el.cursor_pointer().on_click(cx.listener(|tab, _, _, cx| {
                                    if let Some(form) = tab
                                        .detail
                                        .as_mut()
                                        .and_then(|detail| detail.branch_form.as_mut())
                                    {
                                        form.checkout = !form.checkout;
                                        cx.notify();
                                    }
                                }))
                            } else {
                                el.opacity(0.45)
                            }
                        }),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(6.0))
                        .child(
                            accent_button("git-branch-create", "Create branch", enabled, theme)
                                .when(enabled, |el| {
                                    el.on_click(cx.listener(|tab, _, _, cx| tab.create_branch(cx)))
                                }),
                        )
                        .child(
                            small_button("git-branch-cancel", "Cancel", false, theme)
                                .on_click(cx.listener(|tab, _, _, cx| tab.close_branch_form(cx))),
                        ),
                )
        });
        let subject = if commit.subject.is_empty() {
            "(no subject)".to_string()
        } else {
            commit.subject.clone()
        };
        div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(
                div()
                    .px(px(12.0))
                    .pt(px(12.0))
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .when(tree.is_some_and(|tree| tree.agent_active), |el| {
                        el.child(render_agent_banner(busy_policy(cx), theme))
                    })
                    .child(
                        div()
                            .id("git-commit-subject")
                            .debug_selector(|| "git-commit-subject".into())
                            .text_size(px(14.0))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child(SharedString::from(subject)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .text_size(px(12.0))
                            .child(meta("Author", commit.author_name.clone()))
                            .child(meta(
                                "Date",
                                crate::history::format_date(&commit.authored_at),
                            ))
                            .child(meta("Parents", parents))
                            .child(meta("SHA", commit.sha.clone())),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(6.0))
                            .child(
                                detail_button(
                                    "git-commit-checkout",
                                    checkout_label,
                                    can_checkout,
                                    theme,
                                )
                                .when(can_checkout, |el| {
                                    el.on_click(cx.listener(|tab, _, window, cx| {
                                        tab.start_checkout(window, cx)
                                    }))
                                }),
                            )
                            .child(
                                detail_button(
                                    "git-commit-cherry-pick",
                                    "Cherry-pick",
                                    can_pick,
                                    theme,
                                )
                                .when(can_pick, |el| {
                                    el.on_click(cx.listener(|tab, _, window, cx| {
                                        tab.start_cherry_pick(window, cx)
                                    }))
                                }),
                            )
                            .child(
                                detail_button(
                                    "git-commit-copy-sha",
                                    if copied { "Copied" } else { "Copy SHA" },
                                    true,
                                    theme,
                                )
                                .on_click(cx.listener(|tab, _, _, cx| tab.copy_commit_sha(cx))),
                            )
                            .child(
                                detail_button(
                                    "git-commit-open-diff",
                                    if all_open { "Fold diff" } else { "Open diff" },
                                    !files.is_empty(),
                                    theme,
                                )
                                .when(!files.is_empty(), |el| {
                                    el.on_click(
                                        cx.listener(|tab, _, _, cx| {
                                            tab.toggle_commit_file(None, cx)
                                        }),
                                    )
                                }),
                            )
                            .child(
                                detail_button(
                                    "git-commit-branch",
                                    "Create branch",
                                    can_branch,
                                    theme,
                                )
                                .when(can_branch, |el| {
                                    el.on_click(cx.listener(|tab, _, window, cx| {
                                        tab.open_branch_form(window, cx)
                                    }))
                                }),
                            ),
                    )
                    .when_some(refusal, |el, reason| {
                        el.child(
                            div()
                                .id("git-commit-refusal")
                                .debug_selector(|| "git-commit-refusal".into())
                                .text_size(px(12.0))
                                .text_color(theme.text_muted)
                                .child(reason),
                        )
                    })
                    .children(confirm)
                    .children(branch_form)
                    .when(pending, |el| {
                        el.child(
                            div()
                                .text_size(px(12.0))
                                .text_color(theme.text_muted)
                                .child("Working…"),
                        )
                    })
                    .when_some(detail.outcome.clone(), |el, (text, failed)| {
                        el.child(
                            div()
                                .id("git-commit-outcome")
                                .debug_selector(|| "git-commit-outcome".into())
                                .text_size(px(12.0))
                                .text_color(if failed { theme.danger } else { theme.success })
                                .child(text),
                        )
                    }),
            )
            .child(
                div()
                    .h(px(30.0))
                    .px(px(12.0))
                    .flex()
                    .items_center()
                    .text_size(px(12.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text_muted)
                    .child(SharedString::from(format!("Files ({})", files.len()))),
            )
            .into_any_element()
    }

    fn render_commit_file(
        &self,
        index: usize,
        file: &DiffFileSummary,
        detail: &CommitDetail,
        sticky: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = match file.status.as_str() {
            "added" | "untracked" => GitFileState::Added,
            "deleted" => GitFileState::Deleted,
            "renamed" => GitFileState::Renamed,
            "copied" => GitFileState::Copied,
            _ => GitFileState::Modified,
        };
        let (letter, color) = (status_letter(state), state_color(state, theme));
        let (dir, name) = match file.path.rsplit_once('/') {
            Some((dir, name)) => (format!("{dir}/"), name.to_owned()),
            None => (String::new(), file.path.clone()),
        };
        let open = detail.open_files.contains(&file.path);
        let selector = if sticky {
            "git-commit-sticky-file".to_string()
        } else {
            format!("git-commit-file-{index}")
        };
        let label = if file.binary {
            format!("{}, binary", file.path)
        } else {
            format!("{}, +{} -{}", file.path, file.additions, file.deletions)
        };
        let path = file.path.clone();
        div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector.clone())
            .role(gpui::Role::Button)
            .aria_label(SharedString::from(label))
            .aria_expanded(open)
            .h(px(COMMIT_FILE_ROW_HEIGHT))
            .px(px(12.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .text_size(px(12.0))
            .cursor_pointer()
            .when(open, |el| el.bg(theme.ink(0.04)))
            .hover(|style| style.bg(theme.ink(0.04)))
            .on_click(
                cx.listener(move |tab, _, _, cx| tab.toggle_commit_file(Some(path.clone()), cx)),
            )
            .child(
                div()
                    .w(px(12.0))
                    .flex_none()
                    .font_family(theme.font_mono.clone())
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(color)
                    .child(letter),
            )
            .child(div().flex_1().min_w_0().truncate().child(
                gpui::StyledText::new(SharedString::from(format!("{dir}{name}"))).with_highlights(
                    [(
                        0..dir.len(),
                        gpui::HighlightStyle {
                            color: Some(theme.text_muted),
                            ..Default::default()
                        },
                    )],
                ),
            ))
            .child(
                div()
                    .flex_none()
                    .flex()
                    .gap(px(4.0))
                    .font_family(theme.font_mono.clone())
                    .text_size(px(11.0))
                    .map(|el| {
                        if file.binary {
                            el.text_color(theme.text_muted).child("binary")
                        } else {
                            el.child(
                                div()
                                    .text_color(theme.success)
                                    .child(SharedString::from(format!("+{}", file.additions))),
                            )
                            .child(
                                div()
                                    .text_color(theme.danger)
                                    .child(SharedString::from(format!("-{}", file.deletions))),
                            )
                        }
                    }),
            )
            .into_any_element()
    }
}

fn top_chip(id: &'static str, label: &'static str, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .role(gpui::Role::Button)
        .aria_label(label)
        .flex_none()
        .flex()
        .items_center()
        .gap(px(6.0))
        .h(px(28.0))
        .px(px(10.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(theme.hairline(0.09))
        .text_size(px(13.0))
        .text_color(theme.text)
        .cursor_pointer()
        .hover(|style| style.bg(theme.ink(0.06)))
}

fn menu_option(
    selector: String,
    label: SharedString,
    detail: SharedString,
    selected: bool,
    live: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    popover::menu_row(theme, false, selector.clone())
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector.clone())
        .role(gpui::Role::MenuItemRadio)
        .aria_label(label.clone())
        .aria_selected(selected)
        .when(live, |el| {
            el.child(
                div()
                    .size(px(6.0))
                    .flex_none()
                    .rounded_full()
                    .bg(theme.success),
            )
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().truncate().child(label))
                .child(
                    div()
                        .truncate()
                        .text_size(px(11.0))
                        .text_color(theme.text_muted)
                        .child(detail),
                ),
        )
        .child(
            div()
                .ml_auto()
                .size(px(14.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .when(selected, |el| {
                    el.child(div().size(px(6.0)).rounded_full().bg(theme.text))
                }),
        )
}

fn render_agent_banner(policy: GitAgentBusyPolicy, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id("git-agent-active")
        .debug_selector(|| "git-agent-active".into())
        .px(px(8.0))
        .py(px(6.0))
        .rounded(px(6.0))
        .bg(theme.warning.opacity(0.1))
        .text_size(px(12.0))
        .text_color(theme.warning)
        .child(agent_banner(policy))
}

fn button_shell(
    selector: impl Into<SharedString>,
    label: SharedString,
    height: f32,
    padding: f32,
    radius: f32,
) -> gpui::Stateful<gpui::Div> {
    let selector = selector.into();
    div()
        .id(selector.clone())
        .debug_selector(move || selector.to_string())
        .role(gpui::Role::Button)
        .aria_label(label.clone())
        .flex_none()
        .h(px(height))
        .px(px(padding))
        .rounded(px(radius))
        .flex()
        .items_center()
        .text_size(px(12.0))
        .child(label)
}

fn back_to_history(id: &'static str, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .role(gpui::Role::Button)
        .aria_label("Back to history")
        .flex_none()
        .h(px(24.0))
        .pl(px(4.0))
        .pr(px(8.0))
        .rounded(px(6.0))
        .flex()
        .items_center()
        .gap(px(2.0))
        .text_size(px(12.0))
        .text_color(theme.text_muted)
        .cursor_pointer()
        .hover(|style| style.text_color(theme.text).bg(theme.ink(0.06)))
        .child(
            icon(icons::ALT_ARROW_LEFT)
                .size(px(14.0))
                .text_color(theme.text_muted),
        )
        .child("History")
}

fn detail_button(
    selector: &'static str,
    label: impl Into<SharedString>,
    enabled: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    button_shell(selector, label.into(), 26.0, 10.0, 7.0)
        .border_1()
        .border_color(theme.hairline(0.12))
        .text_color(theme.text)
        .map(|el| {
            if enabled {
                el.cursor_pointer().hover(|style| style.bg(theme.ink(0.06)))
            } else {
                el.opacity(0.45)
            }
        })
}

fn accent_button(
    selector: &'static str,
    label: &'static str,
    enabled: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    button_shell(selector, label.into(), 24.0, 9.0, 6.0)
        .font_weight(gpui::FontWeight::MEDIUM)
        .bg(theme.accent_strong)
        .text_color(theme.on_accent)
        .map(|el| {
            if enabled {
                el.cursor_pointer().hover(|style| style.opacity(0.9))
            } else {
                el.opacity(0.45)
            }
        })
}

fn small_button(
    selector: &'static str,
    label: &'static str,
    danger: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    button_shell(selector, label.into(), 24.0, 9.0, 6.0)
        .cursor_pointer()
        .map(|el| {
            if danger {
                el.bg(theme.danger_strong)
                    .text_color(gpui::white())
                    .hover(|style| style.opacity(0.9))
            } else {
                el.border_1()
                    .border_color(theme.hairline(0.12))
                    .text_color(theme.text)
                    .hover(|style| style.bg(theme.ink(0.06)))
            }
        })
}

fn splice_rows<T: PartialEq>(list: &ListState, rows: &mut Vec<T>, next: Vec<T>) {
    let prefix = rows.iter().zip(&next).take_while(|(a, b)| a == b).count();
    let suffix = rows[prefix..]
        .iter()
        .rev()
        .zip(next[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    list.splice(prefix..rows.len() - suffix, next.len() - prefix - suffix);
    *rows = next;
}

fn diff_rows(file: Option<&FileDiff>, binary: bool, truncated: bool) -> Vec<DiffRow> {
    let mut rows = Vec::new();
    match file.filter(|_| !binary) {
        Some(file) if !file.hunks.is_empty() => {
            let mut budget = DIFF_LINE_LIMIT;
            let mut hidden = 0usize;
            for (h, hunk) in file.hunks.iter().enumerate() {
                if budget == 0 {
                    hidden += hunk.lines.len();
                    continue;
                }
                rows.push(DiffRow::Hunk(h));
                let shown = hunk.lines.len().min(budget);
                rows.extend((0..shown).map(|l| DiffRow::Line(h, l)));
                budget -= shown;
                hidden += hunk.lines.len() - shown;
            }
            if hidden > 0 {
                rows.push(DiffRow::Hidden(hidden));
            }
            if truncated {
                rows.push(DiffRow::Truncated);
            }
        }
        _ if binary => rows.push(DiffRow::Binary),
        _ if truncated => rows.push(DiffRow::Truncated),
        _ => rows.push(DiffRow::NoText),
    }
    rows
}

fn hunk_header_parts(header: &str) -> (&str, &str) {
    header
        .strip_prefix("@@")
        .and_then(|rest| rest.find("@@"))
        .map(|end| header.split_at(end + 4))
        .map_or((header, ""), |(range, context)| (range, context.trim()))
}

fn render_diff_row(
    row: DiffRow,
    file: Option<&FileDiff>,
    error: Option<SharedString>,
    first: Option<String>,
    last: bool,
    theme: &Theme,
) -> AnyElement {
    let note = |text: SharedString, color: gpui::Hsla| {
        div()
            .px(px(8.0))
            .py(px(6.0))
            .text_size(px(11.0))
            .text_color(color)
            .child(text)
            .into_any_element()
    };
    let content = match (row, file) {
        (DiffRow::Hunk(h), Some(file)) => {
            let header = &file.hunks[h].header;
            let (range, context) = hunk_header_parts(header);
            div()
                .px(px(8.0))
                .flex()
                .gap(px(8.0))
                .bg(theme.ink(0.03))
                .font_family(theme.font_mono.clone())
                .text_size(px(11.0))
                .text_color(theme.text_muted)
                .child(
                    div()
                        .flex_none()
                        .child(SharedString::from(range.to_owned())),
                )
                .when(!context.is_empty(), |el| {
                    el.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(theme.text_faint)
                            .child(SharedString::from(context.to_owned())),
                    )
                })
                .into_any_element()
        }
        (DiffRow::Line(h, l), Some(file)) => changes::diff_line_row(
            &file.hunks[h].lines[l],
            &[],
            theme,
            changes::gutter_width(file),
            changes::DiffCodeWidth::Wrapped,
            None,
        ),
        (DiffRow::Failed, _) => note(error.unwrap_or_default(), theme.danger),
        (DiffRow::Hidden(hidden), _) => note(
            format!("{hidden} more lines not shown").into(),
            theme.text_muted,
        ),
        (DiffRow::Binary, _) => note("Binary file, no text diff.".into(), theme.text_muted),
        (DiffRow::Truncated, _) => note(
            "The diff is too large to show in full.".into(),
            theme.text_muted,
        ),
        (DiffRow::NoText, _) => note("No text changes.".into(), theme.text_muted),
        _ => note("Loading diff…".into(), theme.text_muted),
    };
    let top = first.is_some();
    div()
        .pl(px(32.0))
        .pr(px(12.0))
        .when(last, |el| el.pb(px(6.0)))
        .child(
            div()
                .when_some(first, |el, selector| el.debug_selector(move || selector))
                .border_l_1()
                .border_r_1()
                .border_color(theme.hairline(0.09))
                .when(top, |el| el.border_t_1().rounded_t(px(8.0)))
                .when(last, |el| el.border_b_1().rounded_b(px(8.0)))
                .overflow_hidden()
                .child(content),
        )
        .into_any_element()
}

impl Render for GitTab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.activation.is_none() {
            self.window_active = window.is_window_active();
            self.activation = Some(cx.observe_window_activation(window, |tab, window, cx| {
                let resumed = window.is_window_active() && !tab.window_active;
                tab.window_active = window.is_window_active();
                if resumed && tab.visible && !tab.loading() {
                    tab.load(cx);
                }
            }));
        }
        let theme = Theme::of(cx).clone();
        let root = div()
            .id("pull-requests-git")
            .debug_selector(|| "pull-requests-git".into())
            .track_focus(&self.focus)
            .role(gpui::Role::Group)
            .aria_label("Git")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .on_key_down(cx.listener(|tab, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && tab.dismiss_overlay(window, cx) {
                    cx.stop_propagation();
                }
            }));
        let lead = std::mem::take(&mut self.toolbar_lead);
        let root = root.child(self.render_toolbar(lead, &theme, cx));
        if self.space.is_none() {
            return root.child(self.render_empty(&theme));
        }
        if self.focus_message && self.composer_open {
            self.focus_message = false;
            if !self.writing() {
                window.focus(&self.message.focus_handle(cx), cx);
            }
        }
        let files = self.tree.as_ref().map(|tree| tree.files.len()).unwrap_or(0);
        let clearance = self.bottom_clearance();
        self.history.update(cx, |history, cx| {
            history.set_working_tree(Some(files), cx);
            history.set_bottom_inset(clearance, cx);
        });
        let main = match &self.expanded {
            Some(open) => self.render_file_diff(open, &theme, cx),
            None => div()
                .size_full()
                .child(self.history.clone())
                .into_any_element(),
        };
        let main = div()
            .flex_1()
            .min_w(px(HISTORY_KEEP_WIDTH))
            .min_h_0()
            .overflow_hidden()
            .child(main);
        let body = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_row()
            .on_drag_move(cx.listener(Self::resize_panel));
        let body = match &self.detail {
            Some(detail) if self.compact => {
                body.child(self.render_detail_panel(detail, &theme, cx))
            }
            Some(detail) => body
                .child(main)
                .child(self.render_detail_panel(detail, &theme, cx)),
            None => body.child(main),
        };
        root.child(body).when(self.composer_open, |el| {
            el.child(self.render_composer(&theme, cx))
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PageTab {
    #[default]
    PullRequests,
    Git,
}

const PAGE_TABS: [(PageTab, &str, &str, &str); 2] = [
    (
        PageTab::PullRequests,
        "PRs",
        "pull-requests-tab-PullRequests",
        icons::PULL_REQUEST,
    ),
    (PageTab::Git, "Git", "pull-requests-tab-Git", icons::FORK),
];

const COMPACT_WIDTH: f32 = 760.0;

pub(super) fn tab_slot(tab: PageTab) -> f32 {
    PAGE_TABS
        .iter()
        .position(|(candidate, ..)| *candidate == tab)
        .unwrap_or(0) as f32
}

impl super::PullRequestsPage {
    pub(crate) fn select_tab(&mut self, tab: PageTab, cx: &mut Context<Self>) {
        if self.tab == tab {
            return;
        }
        self.tab_slide.retarget(
            tab_slot(tab),
            crate::motion::reduced_motion(cx),
            std::time::Instant::now(),
        );
        self.tab = tab;
        if let Some((view, _)) = &self.git_view {
            view.update(cx, |tab, cx| {
                tab.composer_open = false;
                cx.notify();
            });
        }
        crate::settings::update(crate::settings::SavePolicy::Debounced, cx, |settings| {
            settings.pull_requests_tab = tab;
        });
        self.sync_git_visibility(cx);
        cx.notify();
    }

    pub(super) fn sync_git_visibility(&mut self, cx: &mut Context<Self>) {
        let visible = self.visible && self.tab == PageTab::Git;
        if let Some(view) = self.git_view.as_ref().map(|(view, _)| view.clone()) {
            view.update(cx, |tab, cx| tab.set_visible(visible, cx));
        }
    }

    fn git_space(&self, cx: &App) -> Option<GitSpace> {
        let project = self.git_project.as_ref()?;
        let state = self.state.read(cx);
        let space = state.spaces.iter().find(|space| {
            space.git_detected && space.path == project.path && space.device_id == project.device
        })?;
        Some(GitSpace {
            space_id: space.id.clone(),
            path: space.path.clone(),
            device: (state.local_device_id.as_deref() != Some(space.device_id.as_str()))
                .then(|| space.device_id.clone()),
        })
    }

    fn sync_git_tab(&mut self, compact: bool, cx: &mut Context<Self>) -> Entity<GitTab> {
        let view = match &self.git_view {
            Some((view, _)) => view.clone(),
            None => {
                let state = self.state.clone();
                let view = cx.new(|cx| GitTab::new(state, cx));
                let observe = cx.observe(&view, |_, _, cx| cx.notify());
                self.git_view = Some((view.clone(), observe));
                view
            }
        };
        let space = self.git_space(cx);
        let visible = self.visible;
        view.update(cx, |tab, cx| {
            tab.set_space(space, cx);
            tab.drop_missing_checkout(cx);
            tab.set_visible(visible, cx);
            tab.set_compact(compact, cx);
        });
        view
    }

    fn page_segments(
        &self,
        pull_requests: Option<usize>,
    ) -> [crate::pull_request_detail::DockSegment; 2] {
        PAGE_TABS.map(
            |(tab, label, id, glyph)| crate::pull_request_detail::DockSegment {
                id,
                label,
                glyph,
                count: match tab {
                    PageTab::PullRequests => pull_requests,
                    PageTab::Git => None,
                },
            },
        )
    }

    fn select_page_tab(&mut self, slot: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.select_tab(PAGE_TABS[slot].0, cx);
        window.focus(&self.tab_focus[slot], cx);
    }

    pub(super) fn render_page_dock(
        &mut self,
        pull_requests: Option<usize>,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let now = std::time::Instant::now();
        if self.tab_slide.animating_at(now) {
            window.request_animation_frame();
        }
        let segments = self.page_segments(pull_requests);
        let dock = crate::pull_request_detail::dock_pill(
            "pull-requests-page-tabs",
            "Pull requests or Git",
            &segments,
            tab_slot(self.tab) as usize,
            self.tab_slide.value_at(now),
            &self.tab_focus,
            theme,
            cx,
            Self::select_page_tab,
        );
        if self.tab == PageTab::PullRequests
            && let Some(detail) = self.shown_detail().cloned()
        {
            let back = self.full_detail.is_some();
            return detail.update(cx, |detail, cx| {
                detail.navigation(dock, back, theme, window, cx)
            });
        }
        let orb = (self.tab == PageTab::Git)
            .then(|| self.commit_orb(theme, cx))
            .flatten();
        crate::pull_request_detail::dock_row(dock, orb)
    }

    pub(crate) fn page_tab_focused(&self, window: &Window) -> bool {
        self.tab_focus.iter().any(|focus| focus.is_focused(window))
    }

    fn commit_orb(&self, theme: &Theme, cx: &App) -> Option<AnyElement> {
        let (view, _) = self.git_view.as_ref()?;
        let tab = view.read(cx);
        tab.space.as_ref()?;
        let count = tab.tree.as_ref().map_or(0, |tree| tree.files.len());
        let label = match count {
            0 => "Commit".to_owned(),
            1 => "Commit, 1 changed file".to_owned(),
            n => format!("Commit, {n} changed files"),
        };
        let view = view.downgrade();
        Some(
            crate::dock_orb::DockOrb {
                id: "git-commit-orb",
                glyph: icons::LUCIDE_GIT_COMMIT_HORIZONTAL,
                tint: theme.text,
                label: label.into(),
                tooltip: "Commit  c",
                marks: crate::dock_orb::Marks {
                    working: false,
                    count,
                    dot: !tab.composer_open && tab.has_draft(cx),
                },
                open: tab.composer_open,
            }
            .render(
                theme,
                &tab.float,
                {
                    let view = view.clone();
                    move |_, cx| {
                        view.update(cx, |tab, cx| tab.open_composer(cx)).ok();
                    }
                },
                move |window, cx| {
                    view.update(cx, |tab, cx| {
                        tab.fold_composer(window, cx);
                    })
                    .ok();
                },
                cx,
            ),
        )
    }

    pub(crate) fn open_commit_composer_on_key(
        &mut self,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let modifiers = event.keystroke.modifiers;
        if event.keystroke.key_char.as_deref() != Some("c")
            || modifiers.platform
            || modifiers.control
            || modifiers.alt
            || modifiers.function
            || self.tab != PageTab::Git
        {
            return false;
        }
        let Some((view, _)) = &self.git_view else {
            return false;
        };
        view.update(cx, |tab, cx| {
            let open = tab.space.is_some();
            if open {
                tab.open_composer(cx);
            }
            open
        })
    }

    fn git_focus_parked(&self, window: &Window, cx: &App) -> bool {
        self.tab_focus.iter().any(|focus| focus.is_focused(window))
            || self
                .git_view
                .as_ref()
                .is_some_and(|(view, _)| view.read(cx).focus.is_focused(window))
    }

    pub(super) fn render_git_page(
        &mut self,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let compact = f32::from(window.viewport_size().width) < COMPACT_WIDTH;
        let view = self.sync_git_tab(compact, cx);
        let lead = vec![
            self.render_repository_menu(theme, window, cx),
            self.render_device_switcher(theme, cx),
        ];
        view.update(cx, |tab, _| tab.toolbar_lead = lead);
        let count = self.pull_request_count();
        let dock = self.render_page_dock(count, theme, window, cx);
        div()
            .id("pull-requests-page")
            .debug_selector(|| "pull-requests-page".into())
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .pt(px(Theme::TITLEBAR_HEIGHT + Theme::SPACE_LG))
            .on_key_down(cx.listener(|page, event: &KeyDownEvent, window, cx| {
                if page.git_focus_parked(window, cx) && page.open_commit_composer_on_key(event, cx)
                {
                    cx.stop_propagation();
                }
            }))
            .child(div().flex_1().min_h_0().child(view))
            .child(dock)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use zeron_proto::{Chat, Session, Space};
    use zeron_rpc::{RpcReply, RpcService};

    use super::super::PullRequestsPage;
    use super::*;

    type Calls = Arc<Mutex<Vec<(String, serde_json::Value)>>>;

    #[derive(Default)]
    struct GitLab {
        calls: Calls,
        tree: Mutex<Option<serde_json::Value>>,
        action_error: Mutex<Option<String>>,
        action_reply: Mutex<Option<serde_json::Value>>,
        write_error: Mutex<Option<String>>,
        hold_writes: std::sync::atomic::AtomicBool,
        release_write: tokio::sync::Notify,
        writes_answered: std::sync::atomic::AtomicUsize,
        sync_error: Mutex<Option<String>>,
        hold_trees: std::sync::atomic::AtomicBool,
        release_tree: tokio::sync::Notify,
        hold_actions: std::sync::atomic::AtomicBool,
        release_action: tokio::sync::Notify,
        hold_syncs: std::sync::atomic::AtomicBool,
        release_sync: tokio::sync::Notify,
        watch: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>>>,
    }

    async fn held(hold: &std::sync::atomic::AtomicBool, release: &tokio::sync::Notify) {
        if hold.load(std::sync::atomic::Ordering::SeqCst) {
            release.notified().await;
        }
    }

    fn watch_frame(revision: &str) -> serde_json::Value {
        serde_json::json!({"status": {
            "checkoutId": "checkout-1", "deviceId": "dev-local", "revision": revision,
            "complete": true, "files": [],
        }})
    }

    const WRITTEN: &str =
        "Panel: keep the scroll position\n\nThe list reset its offset on every poll.";

    const PATCH: &str = "diff --git a/src/a b.rs b/src/a b.rs\n--- a/src/a b.rs\n+++ b/src/a b.rs\n@@ -1,2 +1,3 @@\n one\n-two\n+TWO\n+three\n";

    const COMMIT_PATCH: &str = "diff --git a/src/panel.rs b/src/panel.rs\n--- a/src/panel.rs\n+++ b/src/panel.rs\n@@ -1,2 +1,2 @@\n one\n-two\n+TWO\ndiff --git a/docs/new note.md b/docs/new note.md\nnew file mode 100644\n--- /dev/null\n+++ b/docs/new note.md\n@@ -0,0 +1 @@\n+hello\n";

    fn tree() -> serde_json::Value {
        serde_json::json!({
            "checkoutId": "checkout-1", "root": "/lab/repo", "branch": "feature/7",
            "headSha": "aaa", "upstream": "origin/feature/7", "ahead": 2, "behind": 0,
            "pushRemote": "origin", "defaultBranch": "main",
            "revision": "r1",
            "files": [
                {"path": "src/a b.rs", "index": "modified", "worktree": "modified",
                 "staged": {"additions": 3, "deletions": 1}, "unstaged": {"additions": 1, "deletions": 0}},
                {"path": "notes/new file.txt", "index": "untracked", "worktree": "untracked",
                 "unstaged": {"additions": 4, "deletions": 0}}
            ]
        })
    }

    #[async_trait::async_trait]
    impl RpcService for GitLab {
        async fn handle(
            &self,
            method: &str,
            params: serde_json::Value,
        ) -> Result<RpcReply, RpcError> {
            self.calls
                .lock()
                .unwrap()
                .push((method.to_string(), params.clone()));
            match method {
                methods::LIST_GIT_HISTORY => RpcReply::value(&serde_json::json!({
                    "commits": [{
                        "sha": "aaa", "parentShas": [], "subject": "Tip of feature",
                        "authorName": "Alice", "authorEmail": "a@example.com",
                        "authoredAt": "2026-10-01T00:00:00Z",
                        "refs": [{"kind": "remote", "label": "origin/feature/7"}]
                    }],
                    "headSha": "aaa", "nextCursor": null, "totalCount": 1, "headCommitCount": 1
                })),
                methods::GET_WORKING_TREE => {
                    held(&self.hold_trees, &self.release_tree).await;
                    RpcReply::value(&self.tree.lock().unwrap().clone().unwrap_or_else(tree))
                }
                methods::WATCH_WORKSPACE_GIT_STATUS => {
                    let Some(receiver) = self.watch.lock().unwrap().take() else {
                        return Err(RpcError::UnknownMethod(method.to_string()));
                    };
                    Ok(RpcReply::Stream(Box::pin(futures::stream::unfold(
                        receiver,
                        |mut receiver| async move {
                            receiver.recv().await.map(|frame| (frame, receiver))
                        },
                    ))))
                }
                methods::RUN_WORKING_TREE_ACTION => {
                    held(&self.hold_actions, &self.release_action).await;
                    if let Some(error) = self.action_error.lock().unwrap().clone() {
                        return Err(RpcError::Failed(error));
                    }
                    let mut next = self.tree.lock().unwrap().clone().unwrap_or_else(tree);
                    next["revision"] = "r2".into();
                    match params["action"]["kind"].as_str() {
                        Some("abortCherryPick") => {
                            next.as_object_mut().unwrap().remove("cherryPickHead");
                        }
                        Some("cherryPick") => {
                            next["cherryPickHead"] = params["action"]["sha"].clone()
                        }
                        _ => {}
                    }
                    let reply = if params["action"]["kind"] == "commit" {
                        next["headSha"] = "bbb".into();
                        if next["upstream"].is_null() {
                            next["ahead"] = (next["ahead"].as_u64().unwrap_or(0) + 1).into();
                        }
                        serde_json::json!({"commit": {
                            "sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                            "branch": next["branch"], "subject": "Fix the panel",
                        }})
                    } else {
                        self.action_reply
                            .lock()
                            .unwrap()
                            .clone()
                            .unwrap_or_else(|| serde_json::json!({}))
                    };
                    *self.tree.lock().unwrap() = Some(next);
                    RpcReply::value(&reply)
                }
                methods::GET_CHECKOUT_DIFF => RpcReply::value(&serde_json::json!({
                    "checkoutId": "checkout-1", "deviceId": "dev-local", "cwd": params["cwd"],
                    "patch": COMMIT_PATCH,
                    "files": [
                        {"path": "src/panel.rs", "status": "modified", "additions": 1, "deletions": 1},
                        {"path": "docs/new note.md", "status": "added", "additions": 1, "deletions": 0}
                    ],
                    "additions": 2, "deletions": 1, "truncated": false, "checksum": "x",
                    "updatedAt": "2026-10-01T00:00:00Z",
                })),
                methods::GET_WORKING_TREE_FILE_DIFF => RpcReply::value(&serde_json::json!({
                    "checkoutId": "checkout-1", "path": params["path"],
                    "staged": params["staged"], "patch": PATCH,
                })),
                methods::GENERATE_COMMIT_MESSAGE => {
                    if self.hold_writes.load(std::sync::atomic::Ordering::SeqCst) {
                        self.release_write.notified().await;
                    }
                    self.writes_answered
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if let Some(error) = self.write_error.lock().unwrap().clone() {
                        return Err(RpcError::Failed(error));
                    }
                    RpcReply::value(&serde_json::json!({
                        "message": WRITTEN,
                        "harness": params.get("harness").cloned().unwrap_or_else(|| {
                            serde_json::to_value(zeron_proto::HarnessId::ClaudeCode).unwrap()
                        }),
                        "model": params.get("model").cloned().unwrap_or_else(|| "claude-haiku-4-5".into()),
                    }))
                }
                methods::LIST_HARNESSES => RpcReply::value(&serde_json::json!([
                    {"id": "claude-code", "name": "Claude Code", "installed": true,
                     "canInstall": false, "enabled": true, "supportsSteering": false,
                     "steeringMode": "step-boundary", "reasoningLevels": []},
                    {"id": "codex", "name": "Codex", "installed": true,
                     "canInstall": false, "enabled": true, "supportsSteering": false,
                     "steeringMode": "step-boundary", "reasoningLevels": []},
                ])),
                methods::LIST_MODELS => RpcReply::value(&if params["harness"] == "claude-code" {
                    serde_json::json!({"models": [
                        {"id": "claude-opus-5-5", "label": "Opus 5.5"},
                        {"id": "claude-fable-5-1", "label": "Fable 5.1"},
                        {"id": "claude-fable-5", "label": "Fable 5"},
                        {"id": "claude-opus-4-8", "label": "Opus 4.8"},
                        {"id": "claude-haiku-4-5", "label": "Haiku 4.5"},
                        {"id": "claude-sonnet-5", "label": "Sonnet 5"},
                    ]})
                } else {
                    serde_json::json!({"models": [{"id": "gpt-5-mini", "label": "GPT-5 mini"}]})
                }),
                methods::SYNC_BRANCH => {
                    held(&self.hold_syncs, &self.release_sync).await;
                    if let Some(error) = self.sync_error.lock().unwrap().clone() {
                        return Err(RpcError::Failed(error));
                    }
                    let mut next = self.tree.lock().unwrap().clone().unwrap_or_else(tree);
                    let branch = next["branch"].as_str().unwrap_or_default().to_owned();
                    let commits = if params["direction"]["kind"] == "push" {
                        let ahead = next["ahead"].clone();
                        next["upstream"] = format!("origin/{branch}").into();
                        next["ahead"] = 0.into();
                        ahead
                    } else {
                        next["headSha"] = "ccc".into();
                        next["behind"] = 0.into();
                        2.into()
                    };
                    let reply = serde_json::json!({
                        "headSha": next["headSha"], "upstream": next["upstream"],
                        "ahead": next["ahead"], "behind": 0, "commits": commits,
                    });
                    *self.tree.lock().unwrap() = Some(next);
                    RpcReply::value(&reply)
                }
                methods::GET_CHANGE_REQUEST_REPOSITORY => RpcReply::value(&"acme/zeron"),
                methods::LIST_CHANGE_REQUEST_PAGE => {
                    RpcReply::value(&zeron_proto::ChangeRequestPage::default())
                }
                _ => Err(RpcError::UnknownMethod(method.to_string())),
            }
        }
    }

    fn space(id: &str, path: &str) -> Space {
        serde_json::from_value(serde_json::json!({
            "id": id, "deviceId": "dev-local", "path": path, "gitDetected": true,
            "createdAt": "2026-10-01T00:00:00Z"
        }))
        .unwrap()
    }

    fn chat(id: &str, cwd: &str, branch: Option<&str>, title: &str) -> Chat {
        serde_json::from_value(serde_json::json!({
            "id": id, "deviceId": "dev-local", "title": title, "archived": false,
            "cwd": cwd, "branch": branch, "spaceId": "space-1",
            "createdAt": "2026-10-01T00:00:00Z"
        }))
        .unwrap()
    }

    fn lab_state() -> AppState {
        let mut state = AppState::new();
        state.local_device_id = Some("dev-local".into());
        state.spaces = vec![
            space("space-1", "/lab/repo"),
            space("space-2", "/lab/plain"),
        ];
        state.chats = vec![
            chat("c-main", "/lab/repo", Some("main"), "On main"),
            chat(
                "c-wt",
                "/lab/worktrees/feat",
                Some("feat/wt"),
                "Worktree thread",
            ),
            chat(
                "c-wt-2",
                "/lab/worktrees/feat/",
                None,
                "Second on same worktree",
            ),
            chat("c-idle", "/lab/worktrees/idle", None, "Idle thread"),
        ];
        state.devices = vec![
            serde_json::from_value(serde_json::json!({
                "id": "dev-local", "name": "Lab Mac", "platform": "macos",
                "capabilities": [
                    zeron_proto::capabilities::SOURCE_CONTROL_V1,
                    zeron_proto::capabilities::SOURCE_CONTROL_SYNC_V1,
                    zeron_proto::capabilities::SOURCE_CONTROL_COMMITS_V1,
                ],
            }))
            .unwrap(),
        ];
        state.sessions = vec![
            serde_json::from_value::<Session>(serde_json::json!({
                "chatId": "c-wt-2", "deviceId": "dev-local", "status": "working",
                "updatedAt": "2026-10-01T00:00:00Z"
            }))
            .unwrap(),
        ];
        state
    }

    #[test]
    fn checkout_options_list_the_main_checkout_then_agent_worktrees_once() {
        let state = lab_state();
        let options = checkout_options(
            &state,
            &GitSpace {
                space_id: "space-1".into(),
                path: "/lab/repo".into(),
                device: None,
            },
        );
        let rows: Vec<_> = options
            .iter()
            .map(|option| {
                (
                    option.path.as_deref(),
                    option.label.to_string(),
                    option.detail.to_string(),
                    option.live,
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                (None, "Main checkout".into(), "/lab/repo".into(), false),
                (
                    Some("/lab/worktrees/feat"),
                    "feat/wt".into(),
                    "Thread \"Worktree thread\" · running".into(),
                    true
                ),
                (
                    Some("/lab/worktrees/idle"),
                    "idle".into(),
                    "Thread \"Idle thread\"".into(),
                    false
                ),
            ]
        );
    }

    #[test]
    fn sections_split_staged_and_changed_files() {
        let tree: WorkingTree = serde_json::from_value(tree()).unwrap();
        let staged: Vec<_> = staged_files(&tree).iter().map(|f| f.path.clone()).collect();
        let changed: Vec<_> = changed_files(&tree)
            .iter()
            .map(|f| f.path.clone())
            .collect();
        assert_eq!(staged, ["src/a b.rs"]);
        assert_eq!(changed, ["src/a b.rs", "notes/new file.txt"]);
    }

    fn settle(
        cx: &mut gpui::VisualTestContext,
        mut done: impl FnMut(&mut gpui::VisualTestContext) -> bool,
    ) {
        for _ in 0..200 {
            cx.run_until_parked();
            if done(cx) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("condition never settled");
    }

    fn lab_space() -> GitSpace {
        GitSpace {
            space_id: "space-1".into(),
            path: "/lab/repo".into(),
            device: None,
        }
    }

    fn init_lab(
        cx: &mut gpui::TestAppContext,
        settings: crate::settings::UiSettings,
        state: AppState,
        lab: Arc<GitLab>,
    ) -> (Entity<AppState>, Calls, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let calls = lab.calls.clone();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            crate::settings::init(settings, dir.path(), cx);
            crate::history::init(
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                cx,
            );
        });
        cx.executor().allow_parking();
        let engine = crate::state::EngineHandle::from_test_client(zeron_rpc::memory_client(lab));
        let state = cx.new(|_| state);
        state.update(cx, |state, _| state.set_test_engine(engine));
        (state, calls, dir)
    }

    fn open_tab(
        cx: &mut gpui::TestAppContext,
        state: AppState,
    ) -> (
        Entity<GitTab>,
        &mut gpui::VisualTestContext,
        Calls,
        tempfile::TempDir,
    ) {
        open_lab(cx, state, Arc::default())
    }

    fn open_lab(
        cx: &mut gpui::TestAppContext,
        state: AppState,
        lab: Arc<GitLab>,
    ) -> (
        Entity<GitTab>,
        &mut gpui::VisualTestContext,
        Calls,
        tempfile::TempDir,
    ) {
        open_lab_at(cx, state, lab, lab_space())
    }

    fn open_lab_at(
        cx: &mut gpui::TestAppContext,
        state: AppState,
        lab: Arc<GitLab>,
        space: GitSpace,
    ) -> (
        Entity<GitTab>,
        &mut gpui::VisualTestContext,
        Calls,
        tempfile::TempDir,
    ) {
        let (state, calls, dir) = init_lab(cx, Default::default(), state, lab);
        let (tab, cx) = cx.add_window_view(|_, cx| GitTab::new(state, cx));
        cx.simulate_resize(gpui::size(px(1200.0), px(900.0)));
        tab.update(cx, |tab, cx| {
            tab.set_space(Some(space), cx);
            tab.tray_open = true;
            tab.open_composer(cx);
        });
        (tab, cx, calls, dir)
    }

    fn open_page(
        cx: &mut gpui::TestAppContext,
        state: AppState,
        tab: PageTab,
    ) -> (
        Entity<PullRequestsPage>,
        &mut gpui::VisualTestContext,
        Calls,
        tempfile::TempDir,
    ) {
        let mut settings = crate::settings::UiSettings::default();
        settings.pull_requests_tab = tab;
        let (state, calls, dir) = init_lab(cx, settings, state, Arc::default());
        let (page, cx) = cx.add_window_view(|_, cx| PullRequestsPage::new(state, cx));
        (page, cx, calls, dir)
    }

    fn page_git_tab(
        page: &Entity<PullRequestsPage>,
        cx: &mut gpui::VisualTestContext,
    ) -> Entity<GitTab> {
        page.read_with(cx, |page, _| page.git_view.as_ref().unwrap().0.clone())
    }

    fn open_page_tray(page: &Entity<PullRequestsPage>, cx: &mut gpui::VisualTestContext) {
        settle(cx, |cx| cx.debug_bounds("git-commit-orb").is_some());
        let tab = page_git_tab(page, cx);
        tab.update(cx, |tab, cx| {
            tab.tray_open = true;
            tab.open_composer(cx);
        });
    }

    fn press(cx: &mut gpui::VisualTestContext, selector: &'static str) {
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is drawn"));
        cx.simulate_mouse_down(
            bounds.center(),
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
        cx.simulate_mouse_up(
            bounds.center(),
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
        cx.run_until_parked();
    }

    #[gpui::test]
    fn the_git_tab_reads_the_project_the_page_was_opened_from(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let mut state = lab_state();
        state.selected_chat = Some("c-main".into());
        let (page, cx, calls, _dir) = open_page(cx, state, PageTab::Git);
        open_page_tray(&page, cx);
        settle(cx, |cx| cx.debug_bounds("git-staged-row-0").is_some());
        assert!(cx.debug_bounds("pull-requests-page-tabs").is_some());
        assert!(cx.debug_bounds("pr-repository").is_some());
        assert!(cx.debug_bounds("git-changes-row-1").is_some());
        assert!(cx.debug_bounds("git-commit-button").is_some());
        let calls_now = calls.lock().unwrap().clone();
        let tree_call = calls_now
            .iter()
            .find(|(method, _)| method == methods::GET_WORKING_TREE)
            .expect("working tree read");
        assert_eq!(tree_call.1, serde_json::json!({"spaceId": "space-1"}));
        let history_call = calls_now
            .iter()
            .find(|(method, _)| method == methods::LIST_GIT_HISTORY)
            .expect("history read");
        assert_eq!(history_call.1["cwd"], "/lab/repo");
    }

    #[gpui::test]
    fn picking_a_project_in_the_repository_menu_moves_the_git_tab(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let mut state = lab_state();
        state.selected_chat = Some("c-main".into());
        let (page, cx, calls, _dir) = open_page(cx, state, PageTab::Git);
        open_page_tray(&page, cx);
        settle(cx, |cx| cx.debug_bounds("git-staged-row-0").is_some());
        calls.lock().unwrap().clear();
        press(cx, "pr-repository");
        press(cx, "pr-project-space-2");
        settle(cx, |_| {
            calls_of(&calls, methods::GET_WORKING_TREE)
                .iter()
                .any(|params| params["spaceId"] == "space-2")
        });
        let tab = page_git_tab(&page, cx);
        tab.read_with(cx, |tab, _| {
            assert_eq!(
                tab.space.as_ref().map(|space| space.path.as_str()),
                Some("/lab/plain")
            );
        });
        assert!(
            calls_of(&calls, methods::LIST_GIT_HISTORY)
                .iter()
                .any(|params| params["cwd"] == "/lab/plain")
        );
    }

    #[gpui::test]
    fn the_dock_persists_the_tab_and_the_git_tab_stops_reading_off_screen(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let mut state = lab_state();
        state.selected_chat = Some("c-main".into());
        let (page, cx, calls, _dir) = open_page(cx, state, PageTab::PullRequests);
        settle(cx, |cx| cx.debug_bounds("pull-requests-tab-Git").is_some());
        assert!(cx.debug_bounds("git-staged-row-0").is_none());
        assert!(calls_of(&calls, methods::GET_WORKING_TREE).is_empty());

        press(cx, "pull-requests-tab-Git");
        open_page_tray(&page, cx);
        settle(cx, |cx| cx.debug_bounds("git-staged-row-0").is_some());
        page.read_with(cx, |page, cx| {
            assert_eq!(page.tab, PageTab::Git);
            assert_eq!(crate::settings::current(cx).pull_requests_tab, PageTab::Git);
        });
        assert!(cx.debug_bounds("pull-requests-tab-Git-count").is_none());

        press(cx, "pull-requests-tab-PullRequests");
        page.read_with(cx, |page, cx| {
            assert_eq!(page.tab, PageTab::PullRequests);
            assert_eq!(
                crate::settings::current(cx).pull_requests_tab,
                PageTab::PullRequests
            );
        });
        assert!(cx.debug_bounds("git-staged-row-0").is_none());
        let tab = page_git_tab(&page, cx);
        tab.read_with(cx, |tab, _| assert!(!tab.visible));

        press(cx, "pull-requests-tab-Git");
        tab.read_with(cx, |tab, _| assert!(tab.visible));
        page.update(cx, |page, cx| page.on_hidden(cx));
        tab.read_with(cx, |tab, _| assert!(!tab.visible));
    }

    #[gpui::test]
    fn the_page_dock_floats_at_the_bottom_of_the_board_and_the_git_tab(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let mut state = lab_state();
        state.selected_chat = Some("c-main".into());
        let (_page, cx, _calls, _dir) = open_page(cx, state, PageTab::PullRequests);
        for width in [600.0, 1200.0] {
            cx.simulate_resize(gpui::size(px(width), px(800.0)));
            for tab in ["pull-requests-tab-PullRequests", "pull-requests-tab-Git"] {
                press(cx, tab);
                settle(cx, |cx| cx.debug_bounds("pr-repository").is_some());
                let dock = cx.debug_bounds("pull-requests-page-tabs").unwrap();
                let page = cx.debug_bounds("pull-requests-page").unwrap();
                let repository = cx.debug_bounds("pr-repository").unwrap();
                let right = match cx.debug_bounds("git-commit-orb") {
                    Some(orb) => {
                        assert_eq!(tab, "pull-requests-tab-Git", "the orb is Git's only");
                        assert_eq!(orb.left(), dock.right() + px(8.0));
                        assert!((orb.center().y - dock.center().y).abs() <= px(1.0));
                        orb.right()
                    }
                    None => dock.right(),
                };
                assert_eq!(dock.bottom(), page.bottom() - px(16.0), "{tab} at {width}");
                assert!(((dock.left() + right) / 2.0 - page.center().x).abs() <= px(1.0));
                assert!(dock.left() >= page.left() && right <= page.right());
                assert!(
                    repository.bottom() < dock.top(),
                    "the switch left the header"
                );
                assert!(cx.debug_bounds("pull-requests-page-tabs-thumb").is_some());
            }
            let toolbar = cx.debug_bounds("pull-requests-git-toolbar").unwrap();
            let repository = cx.debug_bounds("pr-repository").unwrap();
            assert!(
                toolbar.top() <= repository.top() && repository.bottom() <= toolbar.bottom(),
                "on Git the repository menu shares the toolbar row"
            );
            let content = cx.debug_bounds("pull-requests-git").unwrap();
            let page = cx.debug_bounds("pull-requests-page").unwrap();
            assert_eq!(
                content.bottom(),
                page.bottom(),
                "Git content runs under the dock"
            );
        }
    }

    #[gpui::test]
    fn the_history_and_its_file_diff_run_under_the_dock_to_the_page_bottom(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (_page, tab, cx, _calls, _dir) = git_page(cx);
        let history = tab.read_with(cx, |tab, _| tab.history.clone());
        let inset = |cx: &mut gpui::VisualTestContext| {
            history.read_with(cx, |history, _| history.bottom_inset())
        };
        settle(cx, |cx| cx.debug_bounds("git-working-tree-row").is_some());
        let page = cx.debug_bounds("pull-requests-page").unwrap();
        let dock = cx.debug_bounds("pull-requests-page-tabs").unwrap();
        let rows = cx.debug_bounds("git-history-rows").unwrap();
        assert_eq!(rows.bottom(), page.bottom(), "no band under the history");
        assert!(rows.top() < dock.top(), "the dock floats over the history");
        assert_eq!(
            inset(cx),
            crate::pull_request_detail::NAV_CLEARANCE,
            "the last row scrolls clear of the dock"
        );
        assert!(page.bottom() - px(inset(cx)) <= dock.top());

        press(cx, "git-commit-orb");
        if cx.debug_bounds("git-tray").is_none() {
            press(cx, "git-chip-files");
        }
        settle(cx, |cx| {
            cx.update(|window, _| window.refresh());
            let height = cx
                .debug_bounds("git-composer")
                .map(|bounds| f32::from(bounds.size.height));
            height.is_some_and(|height| {
                inset(cx) >= crate::pull_request_detail::NAV_CLEARANCE + height
            })
        });
        let composer = cx.debug_bounds("git-composer").unwrap();
        assert_eq!(
            cx.debug_bounds("git-history-rows").unwrap(),
            rows,
            "the composer floats without moving the history"
        );
        assert!(composer.bottom() <= dock.top());
        assert!(
            page.bottom() - px(inset(cx)) <= composer.top(),
            "the last row scrolls clear of the composer and the dock"
        );

        press(cx, "git-staged-row-0");
        settle(cx, |cx| cx.debug_bounds("git-file-diff").is_some());
        let diff = cx.debug_bounds("git-diff-scroll").unwrap();
        assert_eq!(
            diff.bottom(),
            page.bottom(),
            "the diff runs under the dock too"
        );
        assert!(cx.debug_bounds("git-composer").is_some());
    }

    #[test]
    fn a_clean_tree_reads_as_one_status_line() {
        let mut tree: WorkingTree = serde_json::from_value(tree()).unwrap();
        tree.files.clear();
        assert_eq!(
            clean_status(&tree),
            "Working tree clean · feature/7 · 2 ahead"
        );
        tree.behind = 3;
        tree.ahead = 0;
        assert_eq!(
            clean_status(&tree),
            "Working tree clean · feature/7 · 3 behind"
        );
        tree.branch = None;
        tree.behind = 0;
        assert_eq!(clean_status(&tree), "Working tree clean · detached HEAD");
    }

    fn git_page(
        cx: &mut gpui::TestAppContext,
    ) -> (
        Entity<PullRequestsPage>,
        Entity<GitTab>,
        &mut gpui::VisualTestContext,
        Calls,
        tempfile::TempDir,
    ) {
        let runtime_state = {
            let mut state = lab_state();
            state.selected_chat = Some("c-main".into());
            state
        };
        let (page, cx, calls, dir) = open_page(cx, runtime_state, PageTab::Git);
        cx.simulate_resize(gpui::size(px(1200.0), px(900.0)));
        settle(cx, |cx| cx.debug_bounds("git-commit-orb-count").is_some());
        let tab = page_git_tab(&page, cx);
        (page, tab, cx, calls, dir)
    }

    fn composer_open(tab: &Entity<GitTab>, cx: &mut gpui::VisualTestContext) -> bool {
        cx.run_until_parked();
        let open = tab.read_with(cx, |tab, _| tab.composer_open);
        assert_eq!(open, cx.debug_bounds("git-composer").is_some());
        open
    }

    #[gpui::test]
    fn the_commit_orb_counts_changes_and_opens_on_click_or_c_and_escape_folds_it(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (page, tab, cx, _calls, _dir) = git_page(cx);
        page.read_with(cx, |page, cx| {
            let tab = page.git_view.as_ref().unwrap().0.read(cx);
            assert_eq!(tab.tree.as_ref().unwrap().files.len(), 2);
        });
        assert!(!composer_open(&tab, cx), "the Git tab opens on the orb");
        assert!(cx.debug_bounds("git-commit-orb-dot").is_none());
        assert!(cx.debug_bounds("pull-requests-git-panel").is_none());
        for width in [600.0, 1200.0] {
            cx.simulate_resize(gpui::size(px(width), px(900.0)));
            cx.run_until_parked();
            let git = cx.debug_bounds("pull-requests-git").unwrap();
            let page = cx.debug_bounds("pull-requests-page").unwrap();
            assert_eq!(git.size.width, page.size.width, "history spans the page");
            let orb = cx.debug_bounds("git-commit-orb").unwrap();
            assert!(orb.right() <= page.right());
        }

        for width in [600.0, 1200.0] {
            cx.simulate_resize(gpui::size(px(width), px(900.0)));
            press(cx, "git-commit-orb");
            if cx.debug_bounds("git-tray").is_none() {
                press(cx, "git-chip-files");
            }
            let page = cx.debug_bounds("pull-requests-page").unwrap();
            let composer = cx.debug_bounds("git-composer").unwrap();
            let dock = cx.debug_bounds("pull-requests-page-tabs").unwrap();
            assert!(composer.left() >= page.left() && composer.right() <= page.right());
            assert!(
                composer.bottom() <= dock.top(),
                "the composer rises above the dock"
            );
            for selector in [
                "git-tray",
                "git-staged-row-0",
                "git-write-ai",
                "git-commit-count",
                "git-commit-more",
                "git-commit-button",
            ] {
                let bounds = cx.debug_bounds(selector).unwrap();
                assert!(
                    bounds.left() >= composer.left() && bounds.right() <= composer.right(),
                    "{selector} fits the composer at {width}"
                );
            }
            tab.update_in(cx, |tab, window, cx| {
                assert!(tab.message.focus_handle(cx).is_focused(window))
            });
            cx.simulate_keystrokes("escape");
            assert!(!composer_open(&tab, cx));
        }
        press(cx, "git-commit-orb");
        assert!(composer_open(&tab, cx));
        cx.simulate_keystrokes("escape");
        assert!(!composer_open(&tab, cx));
        tab.update_in(cx, |tab, window, _| assert!(tab.focus.is_focused(window)));

        cx.simulate_keystrokes("c");
        assert!(
            composer_open(&tab, cx),
            "c opens it with nothing typed into"
        );
        cx.simulate_keystrokes("c");
        assert_eq!(message(&tab, cx), "c", "inside the box c is text");
        cx.simulate_keystrokes("escape");
        assert!(!composer_open(&tab, cx), "Esc folds a draft too");
        assert!(cx.debug_bounds("git-commit-orb-dot").is_some());
        cx.simulate_keystrokes("c");
        assert!(composer_open(&tab, cx));
        assert_eq!(message(&tab, cx), "c", "the draft is kept");
        assert!(cx.debug_bounds("git-commit-orb-dot").is_none());
    }

    #[gpui::test]
    fn an_empty_composer_folds_on_an_outside_click_or_leaving_git_and_a_draft_stays(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (_page, tab, cx, _calls, _dir) = git_page(cx);
        let outside = |cx: &mut gpui::VisualTestContext| {
            let toolbar = cx.debug_bounds("pull-requests-git-toolbar").unwrap();
            cx.simulate_click(toolbar.center(), gpui::Modifiers::default());
        };

        press(cx, "git-commit-orb");
        press(cx, "git-commit-count");
        assert!(composer_open(&tab, cx), "a click inside keeps it");
        outside(cx);
        assert!(
            !composer_open(&tab, cx),
            "an empty box folds on an outside click"
        );

        press(cx, "git-commit-orb");
        set_message(&tab, cx, "Fix the panel");
        outside(cx);
        assert!(
            composer_open(&tab, cx),
            "a draft is not dropped by a stray click"
        );

        press(cx, "pull-requests-tab-PullRequests");
        press(cx, "pull-requests-tab-Git");
        assert!(!composer_open(&tab, cx), "leaving Git folds it");
        assert!(cx.debug_bounds("git-commit-orb-dot").is_some());
        assert_eq!(message(&tab, cx), "Fix the panel");

        set_message(&tab, cx, "");
        press(cx, "git-commit-orb");
        press(cx, "pull-requests-tab-PullRequests");
        assert!(!composer_open(&tab, cx));
        assert!(
            cx.debug_bounds("git-commit-orb").is_none(),
            "the orb is Git's only"
        );
    }

    #[gpui::test]
    fn the_orb_folds_the_floating_composer_and_a_history_click_folds_it_and_lands(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (_page, tab, cx, _calls, _dir) = git_page(cx);
        let history = tab.read_with(cx, |tab, _| tab.history.clone());
        settle(cx, |cx| cx.debug_bounds("git-working-tree-row").is_some());
        let rows = cx.debug_bounds("git-working-tree-row").unwrap();

        press(cx, "git-commit-orb");
        assert!(composer_open(&tab, cx));
        settle(cx, |cx| {
            cx.update(|window, _| window.refresh());
            let height = cx
                .debug_bounds("git-composer")
                .map(|bounds| f32::from(bounds.size.height));
            history.read_with(cx, |history, _| {
                height.is_some_and(|height| history.bottom_inset() >= height)
            })
        });
        let composer = cx.debug_bounds("git-composer").unwrap();
        let dock = cx.debug_bounds("pull-requests-page-tabs").unwrap();
        assert!(
            history.read_with(cx, |history, _| history.bottom_inset())
                >= f32::from(composer.size.height),
            "the history's last rows scroll clear of the composer"
        );
        assert_eq!(
            cx.debug_bounds("git-working-tree-row").unwrap(),
            rows,
            "the history keeps its place under the composer"
        );
        assert!(
            composer.bottom() <= dock.top() && composer.top() > rows.bottom(),
            "{composer:?} {dock:?} {rows:?}"
        );

        let orb = cx.debug_bounds("git-commit-orb").unwrap().center();
        cx.simulate_mouse_down(orb, gpui::MouseButton::Left, gpui::Modifiers::default());
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert!(
            composer_open(&tab, cx),
            "a press on the orb is not an outside click"
        );
        cx.simulate_mouse_up(orb, gpui::MouseButton::Left, gpui::Modifiers::default());
        assert!(!composer_open(&tab, cx), "the orb folds an empty composer");

        press(cx, "git-commit-orb");
        set_message(&tab, cx, "Fix the panel");
        press(cx, "git-commit-orb");
        assert!(!composer_open(&tab, cx), "the orb folds a draft");
        assert!(cx.debug_bounds("git-commit-orb-dot").is_some());
        assert_eq!(message(&tab, cx), "Fix the panel");
        set_message(&tab, cx, "");

        press(cx, "git-commit-orb");
        assert!(composer_open(&tab, cx));
        let commit_row = gpui::point(rows.left() + px(120.0), rows.center().y + rows.size.height);
        cx.simulate_click(commit_row, gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(!composer_open(&tab, cx), "an outside click folds it");
        assert_eq!(
            history.read_with(cx, |history, _| history.selected_sha().map(str::to_owned)),
            Some("aaa".to_owned()),
            "and the history row under the click still opens its commit"
        );
    }

    #[gpui::test]
    fn the_chips_toggle_the_file_tray_and_its_checkboxes_stage(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        let (tab, cx, calls, _dir) = open_loaded(cx, lab_state(), lab);
        tab.update(cx, |tab, cx| {
            tab.tray_open = false;
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-tray").is_none());
        assert!(cx.debug_bounds("git-staged-row-0").is_none());
        for chip in ["git-chip-staged", "git-chip-changed", "git-chip-files"] {
            assert!(cx.debug_bounds(chip).is_some(), "{chip}");
        }
        assert!(cx.debug_bounds("git-clean-status").is_none());

        click(cx, "git-chip-files");
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-tray").is_some());
        assert!(cx.debug_bounds("git-stash-button").is_some());
        let tray = cx.debug_bounds("git-tray").unwrap();
        let message = cx.debug_bounds("git-commit-message").unwrap();
        assert!(
            tray.bottom() <= message.top(),
            "the tray sits above the message"
        );
        let checkbox = cx.debug_bounds("git-changes-toggle-1").unwrap();
        let row = cx.debug_bounds("git-changes-row-1").unwrap();
        assert!(
            checkbox.left() - row.left() < px(24.0),
            "the checkbox leads the row"
        );

        click(cx, "git-changes-toggle-1");
        settle_action(cx, &tab, &calls, 1);
        assert_eq!(
            calls_of(&calls, methods::RUN_WORKING_TREE_ACTION)[0]["action"],
            serde_json::json!({"kind": "stage", "paths": ["notes/new file.txt"]})
        );
        assert!(
            calls_of(&calls, methods::GET_WORKING_TREE_FILE_DIFF).is_empty(),
            "the checkbox does not open the diff"
        );

        click(cx, "git-chip-staged");
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-tray").is_none());
    }

    #[gpui::test]
    fn a_clean_tree_shows_its_status_and_cannot_commit(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        let mut clean = tree();
        clean["files"] = serde_json::json!([]);
        *lab.tree.lock().unwrap() = Some(clean);
        let (tab, cx, calls, _dir) = open_loaded(cx, lab_state(), lab);
        assert!(cx.debug_bounds("git-clean-status").is_some());
        assert!(cx.debug_bounds("git-chip-files").is_none());
        assert!(cx.debug_bounds("git-tray").is_none());
        assert!(cx.debug_bounds("git-stash-button").is_some());
        set_message(&tab, cx, "Nothing here");
        click(cx, "git-commit-button");
        cx.simulate_keystrokes("cmd-enter");
        cx.run_until_parked();
        assert!(calls_of(&calls, methods::RUN_WORKING_TREE_ACTION).is_empty());
    }

    fn calls_of(calls: &Calls, wanted: &str) -> Vec<serde_json::Value> {
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(method, _)| method == wanted)
            .map(|(_, params)| params.clone())
            .collect()
    }

    #[gpui::test]
    fn the_checkout_picker_retargets_the_graph_and_the_working_tree(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (_tab, cx, calls, _dir) = open_tab(cx, lab_state());
        settle(cx, |cx| cx.debug_bounds("git-staged-row-0").is_some());
        click(cx, "pull-requests-git-checkout");
        cx.run_until_parked();
        assert!(cx.debug_bounds("pull-requests-git-checkout-0").is_some());
        assert!(cx.debug_bounds("pull-requests-git-checkout-3").is_none());
        let worktree = cx
            .debug_bounds("pull-requests-git-checkout-1")
            .expect("the agent worktree is listed");
        calls.lock().unwrap().clear();
        cx.simulate_click(worktree.center(), gpui::Modifiers::default());
        settle(cx, |_| {
            let calls = calls.lock().unwrap();
            [methods::LIST_GIT_HISTORY, methods::GET_WORKING_TREE]
                .iter()
                .all(|wanted| calls.iter().any(|(method, _)| method == wanted))
        });
        let calls_now = calls.lock().unwrap().clone();
        let tree_call = calls_now
            .iter()
            .find(|(method, _)| method == methods::GET_WORKING_TREE)
            .unwrap();
        assert_eq!(
            tree_call.1,
            serde_json::json!({"spaceId": "space-1", "checkoutPath": "/lab/worktrees/feat"})
        );
        let history_call = calls_now
            .iter()
            .find(|(method, _)| method == methods::LIST_GIT_HISTORY)
            .unwrap();
        assert_eq!(history_call.1["cwd"], "/lab/worktrees/feat");
    }

    fn press_escape(tab: &Entity<GitTab>, cx: &mut gpui::VisualTestContext) {
        assert!(tab.update_in(cx, |tab, window, cx| tab.dismiss_overlay(window, cx)));
    }

    fn click(cx: &mut gpui::VisualTestContext, selector: &'static str) {
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is drawn"));
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    }

    fn settle_action(
        cx: &mut gpui::VisualTestContext,
        tab: &Entity<GitTab>,
        calls: &Calls,
        actions: usize,
    ) {
        settle(cx, |cx| {
            calls_of(calls, methods::RUN_WORKING_TREE_ACTION).len() == actions
                && tab.read_with(cx, |tab, _| tab.pending.is_none() && !tab.loading())
        });
    }

    #[gpui::test]
    fn stage_and_unstage_send_the_shown_revision_and_the_policy_then_reread(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        let (tab, cx, calls, _dir) = open_lab(cx, lab_state(), lab.clone());
        settle(cx, |cx| cx.debug_bounds("git-changes-toggle-1").is_some());

        let reads_before = calls_of(&calls, methods::GET_WORKING_TREE).len();
        click(cx, "git-changes-toggle-1");
        settle_action(cx, &tab, &calls, 1);
        assert_eq!(
            calls_of(&calls, methods::RUN_WORKING_TREE_ACTION)[0],
            serde_json::json!({
                "spaceId": "space-1", "expectedRevision": "r1", "busyPolicy": "safeOnly",
                "action": {"kind": "stage", "paths": ["notes/new file.txt"]},
            })
        );
        assert!(calls_of(&calls, methods::GET_WORKING_TREE).len() > reads_before);
        tab.read_with(cx, |tab, _| {
            assert_eq!(
                tab.tree.as_ref().unwrap().revision,
                "r2",
                "the tree is read again"
            );
        });
        assert!(
            calls_of(&calls, methods::GET_WORKING_TREE_FILE_DIFF).is_empty(),
            "the button click does not expand the row"
        );

        cx.update(|_, cx| {
            crate::settings::update(crate::settings::SavePolicy::Immediate, cx, |settings| {
                settings.git_agent_busy_policy = crate::settings::GitAgentBusyPolicy::AllowAll;
            });
        });
        click(cx, "git-staged-toggle-0");
        settle_action(cx, &tab, &calls, 2);
        click(cx, "git-changes-all");
        settle_action(cx, &tab, &calls, 3);
        click(cx, "git-staged-all");
        settle_action(cx, &tab, &calls, 4);
        let sent = calls_of(&calls, methods::RUN_WORKING_TREE_ACTION);
        assert_eq!(
            sent[1],
            serde_json::json!({
                "spaceId": "space-1", "expectedRevision": "r2", "busyPolicy": "allowAll",
                "action": {"kind": "unstage", "paths": ["src/a b.rs"]},
            })
        );
        assert_eq!(sent[2]["action"], serde_json::json!({"kind": "stageAll"}));
        assert_eq!(sent[3]["action"], serde_json::json!({"kind": "unstageAll"}));
    }

    #[gpui::test]
    fn clicking_a_file_opens_its_diff_in_place_of_the_history_and_back_returns(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (tab, cx, calls, _dir) = open_tab(cx, lab_state());
        settle(cx, |cx| cx.debug_bounds("git-staged-row-0").is_some());
        click(cx, "git-staged-row-0");
        settle(cx, |cx| {
            tab.read_with(cx, |tab, _| {
                matches!(
                    tab.expanded.as_ref().map(|open| &open.view),
                    Some(Loadable::Ready(_))
                )
            }) && cx.debug_bounds("git-file-diff").is_some()
        });
        assert_eq!(
            calls_of(&calls, methods::GET_WORKING_TREE_FILE_DIFF),
            [serde_json::json!({"spaceId": "space-1", "path": "src/a b.rs", "staged": true})]
        );
        tab.read_with(cx, |tab, _| {
            let Some(Loadable::Ready((_, Some(file)))) =
                tab.expanded.as_ref().map(|open| &open.view)
            else {
                panic!("a parsed diff");
            };
            assert_eq!(file.hunks.len(), 1);
            assert_eq!(file.hunks[0].lines.len(), 4);
            assert_eq!((file.additions, file.deletions), (2, 1));
        });
        assert!(
            calls_of(&calls, methods::RUN_WORKING_TREE_ACTION).is_empty(),
            "expanding writes nothing"
        );
        assert!(cx.debug_bounds("git-diff-view").is_some());
        assert!(
            cx.debug_bounds("git-working-tree-row").is_none(),
            "the diff takes the history's place"
        );
        click(cx, "git-diff-back");
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-diff-view").is_none());
        assert!(cx.debug_bounds("git-working-tree-row").is_some());
        tab.update(cx, |tab, cx| tab.open_composer(cx));
        cx.run_until_parked();
        click(cx, "git-staged-row-0");
        settle(cx, |cx| cx.debug_bounds("git-file-diff").is_some());
        click(cx, "git-staged-row-0");
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-file-diff").is_none());
    }

    #[gpui::test]
    fn read_only_while_an_agent_runs_disables_every_write(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        let mut busy = tree();
        busy["agentActive"] = true.into();
        *lab.tree.lock().unwrap() = Some(busy);
        let (tab, cx, calls, _dir) = open_lab(cx, lab_state(), lab);
        cx.update(|_, cx| {
            crate::settings::update(crate::settings::SavePolicy::Immediate, cx, |settings| {
                settings.git_agent_busy_policy = crate::settings::GitAgentBusyPolicy::BlockAll;
            });
        });
        settle(cx, |cx| cx.debug_bounds("git-agent-active").is_some());
        for selector in [
            "git-changes-toggle-1",
            "git-staged-toggle-0",
            "git-changes-all",
            "git-staged-all",
        ] {
            click(cx, selector);
            cx.run_until_parked();
        }
        assert!(calls_of(&calls, methods::RUN_WORKING_TREE_ACTION).is_empty());
        tab.update(cx, |tab, cx| {
            assert!(!tab.allows(&WorkingTreeAction::StageAll, busy_policy(cx), cx));
            assert!(tab.allows(
                &WorkingTreeAction::StageAll,
                GitAgentBusyPolicy::SafeOnly,
                cx
            ));
            assert!(tab.allows(
                &WorkingTreeAction::StageAll,
                GitAgentBusyPolicy::AllowAll,
                cx
            ));
            assert!(!tab.allows(
                &WorkingTreeAction::Discard { paths: vec![] },
                GitAgentBusyPolicy::SafeOnly,
                cx
            ));
        });
        assert_eq!(
            agent_banner(GitAgentBusyPolicy::BlockAll),
            "An agent is running in this worktree. Git is read-only here until it stops."
        );
    }

    #[gpui::test]
    fn safe_only_still_stages_while_an_agent_runs(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        let mut busy = tree();
        busy["agentActive"] = true.into();
        *lab.tree.lock().unwrap() = Some(busy);
        let (tab, cx, calls, _dir) = open_lab(cx, lab_state(), lab);
        settle(cx, |cx| cx.debug_bounds("git-agent-active").is_some());
        click(cx, "git-changes-toggle-0");
        settle_action(cx, &tab, &calls, 1);
        assert_eq!(
            calls_of(&calls, methods::RUN_WORKING_TREE_ACTION)[0]["busyPolicy"],
            "safeOnly"
        );
    }

    #[gpui::test]
    fn a_device_without_source_control_gets_no_write_controls(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let mut state = lab_state();
        state.devices[0].capabilities.clear();
        let (_tab, cx, calls, _dir) = open_tab(cx, state);
        settle(cx, |cx| cx.debug_bounds("git-changes-row-1").is_some());
        for selector in [
            "git-changes-toggle-0",
            "git-staged-toggle-0",
            "git-changes-all",
            "git-staged-all",
        ] {
            assert!(cx.debug_bounds(selector).is_none(), "{selector}");
        }
        click(cx, "git-changes-row-1");
        cx.run_until_parked();
        assert!(calls_of(&calls, methods::GET_WORKING_TREE_FILE_DIFF).is_empty());
        assert!(cx.debug_bounds("git-file-diff").is_none());
    }

    #[gpui::test]
    fn a_refused_write_shows_the_reason_and_rereads_the_tree(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        *lab.action_error.lock().unwrap() = Some(
            "The working tree changed since it was shown. Check the refreshed list and try again."
                .into(),
        );
        let (tab, cx, calls, _dir) = open_lab(cx, lab_state(), lab);
        settle(cx, |cx| cx.debug_bounds("git-changes-toggle-0").is_some());
        let reads_before = calls_of(&calls, methods::GET_WORKING_TREE).len();
        click(cx, "git-changes-toggle-0");
        settle_action(cx, &tab, &calls, 1);
        settle(cx, |cx| cx.debug_bounds("git-error-0").is_some());
        assert!(calls_of(&calls, methods::GET_WORKING_TREE).len() > reads_before);
        tab.read_with(cx, |tab, _| {
            assert!(
                tab.action_error
                    .as_ref()
                    .is_some_and(|error| error.contains("changed since"))
            );
        });
    }

    fn set_message(tab: &Entity<GitTab>, cx: &mut gpui::VisualTestContext, text: &str) {
        tab.update(cx, |tab, cx| {
            tab.message.update(cx, |input, cx| input.set_text(text, cx));
        });
        cx.run_until_parked();
    }

    fn message(tab: &Entity<GitTab>, cx: &mut gpui::VisualTestContext) -> String {
        tab.read_with(cx, |tab, cx| tab.message.read(cx).text().to_string())
    }

    fn commit_kinds(calls: &Calls) -> Vec<serde_json::Value> {
        calls_of(calls, methods::RUN_WORKING_TREE_ACTION)
            .into_iter()
            .map(|params| params["action"].clone())
            .collect()
    }

    #[test]
    fn the_subject_counter_counts_characters_of_the_first_line() {
        assert_eq!(subject_chars(""), 0);
        assert_eq!(subject_chars("Fix the panel\n\nBody text"), 13);
        assert_eq!(subject_chars("Sửa lỗi"), 7);
    }

    #[gpui::test]
    fn enter_breaks_the_line_and_cmd_enter_commits_then_the_box_clears(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        let (tab, cx, calls, _dir) = open_lab(cx, lab_state(), lab);
        cx.update(|_, cx| crate::composer::init(cx, Default::default()));
        settle(cx, |cx| cx.debug_bounds("git-commit-button").is_some());
        settle(cx, |cx| tab.read_with(cx, |tab, _| tab.tree.is_some()));

        click(cx, "git-commit-button");
        cx.simulate_keystrokes("cmd-enter");
        cx.run_until_parked();
        assert!(
            calls_of(&calls, methods::RUN_WORKING_TREE_ACTION).is_empty(),
            "an empty message never commits"
        );

        click(cx, "git-commit-message");
        cx.simulate_input("Fix the panel");
        cx.simulate_keystrokes("enter");
        cx.simulate_input("Why it broke.");
        cx.run_until_parked();
        assert_eq!(message(&tab, cx), "Fix the panel\nWhy it broke.");
        assert!(
            calls_of(&calls, methods::RUN_WORKING_TREE_ACTION).is_empty(),
            "plain Enter is a newline, not a commit"
        );

        let history_reads = calls_of(&calls, methods::LIST_GIT_HISTORY).len();
        cx.simulate_keystrokes("cmd-enter");
        settle_action(cx, &tab, &calls, 1);
        assert_eq!(
            calls_of(&calls, methods::RUN_WORKING_TREE_ACTION)[0],
            serde_json::json!({
                "spaceId": "space-1", "expectedRevision": "r1", "busyPolicy": "safeOnly",
                "action": {
                    "kind": "commit", "message": "Fix the panel\nWhy it broke.",
                    "amend": false, "all": false,
                },
            })
        );
        settle(cx, |_| {
            calls_of(&calls, methods::LIST_GIT_HISTORY).len() > history_reads
        });
        assert_eq!(message(&tab, cx), "", "the box clears after a commit");
        tab.read_with(cx, |tab, _| {
            let commit = tab.last_commit.as_ref().expect("the reply is kept");
            assert_eq!(commit.subject, "Fix the panel");
            assert_eq!(commit.branch.as_deref(), Some("feature/7"));
            assert_eq!(tab.tree.as_ref().unwrap().head_sha.as_deref(), Some("bbb"));
        });
        assert!(cx.debug_bounds("git-last-commit").is_some());
    }

    #[gpui::test]
    fn the_split_menu_amends_and_commits_all_when_the_policy_allows(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        let mut busy = tree();
        busy["agentActive"] = true.into();
        *lab.tree.lock().unwrap() = Some(busy);
        let (tab, cx, calls, _dir) = open_lab(cx, lab_state(), lab);
        settle(cx, |cx| cx.debug_bounds("git-agent-active").is_some());

        set_message(&tab, cx, "Fix the panel");
        click(cx, "git-commit-more");
        cx.run_until_parked();
        click(cx, "git-commit-menu-all");
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-commit-menu-amend").is_some());
        click(cx, "git-commit-menu-amend");
        cx.run_until_parked();
        assert!(
            calls_of(&calls, methods::RUN_WORKING_TREE_ACTION).is_empty(),
            "SafeOnly refuses commit all and amend while an agent runs"
        );
        click(cx, "git-commit-menu-commit");
        settle_action(cx, &tab, &calls, 1);
        assert_eq!(
            commit_kinds(&calls),
            [
                serde_json::json!({"kind": "commit", "message": "Fix the panel", "amend": false, "all": false})
            ],
            "commit of staged files still runs, under the warning banner"
        );
        assert!(cx.debug_bounds("git-commit-menu").is_none());

        cx.update(|_, cx| {
            crate::settings::update(crate::settings::SavePolicy::Immediate, cx, |settings| {
                settings.git_agent_busy_policy = crate::settings::GitAgentBusyPolicy::AllowAll;
            });
        });
        set_message(&tab, cx, "Everything");
        click(cx, "git-commit-more");
        cx.run_until_parked();
        click(cx, "git-commit-menu-all");
        settle_action(cx, &tab, &calls, 2);
        set_message(&tab, cx, "Reworded");
        click(cx, "git-commit-more");
        cx.run_until_parked();
        click(cx, "git-commit-menu-amend");
        settle_action(cx, &tab, &calls, 3);
        let sent = calls_of(&calls, methods::RUN_WORKING_TREE_ACTION);
        assert_eq!(
            sent[1]["action"],
            serde_json::json!({"kind": "commit", "message": "Everything", "amend": false, "all": true})
        );
        assert_eq!(sent[1]["busyPolicy"], "allowAll");
        assert_eq!(
            sent[2]["action"],
            serde_json::json!({"kind": "commit", "message": "Reworded", "amend": true, "all": false})
        );

        cx.update(|_, cx| {
            crate::settings::update(crate::settings::SavePolicy::Immediate, cx, |settings| {
                settings.git_agent_busy_policy = crate::settings::GitAgentBusyPolicy::BlockAll;
            });
        });
        set_message(&tab, cx, "Blocked");
        click(cx, "git-commit-button");
        click(cx, "git-commit-message");
        cx.simulate_keystrokes("cmd-enter");
        cx.run_until_parked();
        assert_eq!(
            calls_of(&calls, methods::RUN_WORKING_TREE_ACTION).len(),
            3,
            "BlockAll leaves the commit box inert"
        );
    }

    #[gpui::test]
    fn a_refused_commit_keeps_the_message_and_shows_why(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        *lab.action_error.lock().unwrap() = Some(
            "Git does not know who you are. Set your name and email with `git config --global user.name \"Your Name\"` and `git config --global user.email you@example.com`, then commit again."
                .into(),
        );
        let (tab, cx, calls, _dir) = open_lab(cx, lab_state(), lab);
        settle(cx, |cx| cx.debug_bounds("git-commit-button").is_some());
        settle(cx, |cx| tab.read_with(cx, |tab, _| tab.tree.is_some()));
        set_message(&tab, cx, "Fix the panel");
        click(cx, "git-commit-button");
        settle_action(cx, &tab, &calls, 1);
        settle(cx, |cx| cx.debug_bounds("git-error-0").is_some());
        assert_eq!(message(&tab, cx), "Fix the panel");
        tab.read_with(cx, |tab, _| {
            assert!(tab.last_commit.is_none());
            assert!(
                tab.action_error
                    .as_ref()
                    .is_some_and(|error| error.contains("does not know who you are"))
            );
        });
    }

    fn pick_model(
        tab: &Entity<GitTab>,
        cx: &mut gpui::VisualTestContext,
        choice: &CommitModelChoice,
    ) {
        let picker = tab.read_with(cx, |tab, _| tab.model_picker.clone());
        picker.update(cx, |_, cx| cx.emit(CommitModelPicked(choice.clone())));
        cx.run_until_parked();
    }

    fn writes(calls: &Calls) -> Vec<serde_json::Value> {
        calls_of(calls, methods::GENERATE_COMMIT_MESSAGE)
    }

    #[gpui::test]
    fn write_with_ai_fills_the_box_and_the_chip_override_lasts_one_commit(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        let (tab, cx, calls, _dir) = open_lab(cx, lab_state(), lab);
        cx.update(|_, cx| crate::composer::init(cx, Default::default()));
        settle(cx, |cx| cx.debug_bounds("git-write-ai").is_some());
        settle(cx, |cx| tab.read_with(cx, |tab, _| tab.tree.is_some()));

        click(cx, "git-write-ai");
        settle(cx, |cx| message(&tab, cx) == WRITTEN);
        assert_eq!(
            writes(&calls),
            [serde_json::json!({
                "spaceId": "space-1",
                "instruction": crate::settings::DEFAULT_COMMIT_INSTRUCTION,
            })]
        );
        tab.read_with(cx, |tab, _| {
            assert!(!tab.writing());
            let written = tab.written.as_ref().expect("the reply is kept");
            assert_eq!(written.model, "claude-haiku-4-5");
        });
        assert!(cx.debug_bounds("git-write-note").is_some());
        assert!(
            calls_of(&calls, methods::RUN_WORKING_TREE_ACTION).is_empty(),
            "writing a message never commits"
        );

        cx.update(|_, cx| {
            crate::settings::update(crate::settings::SavePolicy::Immediate, cx, |settings| {
                settings.commit_message_instruction = "  Use Conventional Commits.  ".into();
                settings.commit_message_model = Some(CommitModelChoice {
                    harness: zeron_proto::HarnessId::Codex,
                    model: "gpt-5-mini".into(),
                });
            });
        });
        let opus = CommitModelChoice {
            harness: zeron_proto::HarnessId::ClaudeCode,
            model: "claude-opus".into(),
        };
        pick_model(&tab, cx, &opus);
        assert!(cx.debug_bounds("git-write-model-override").is_some());
        click(cx, "git-write-ai");
        settle(cx, |_| writes(&calls).len() == 2);
        settle(cx, |cx| tab.read_with(cx, |tab, _| !tab.writing()));
        assert_eq!(
            writes(&calls)[1],
            serde_json::json!({
                "spaceId": "space-1", "instruction": "Use Conventional Commits.",
                "harness": serde_json::to_value(zeron_proto::HarnessId::ClaudeCode).unwrap(),
                "model": "claude-opus",
            })
        );

        click(cx, "git-commit-message");
        cx.simulate_keystrokes("cmd-enter");
        settle_action(cx, &tab, &calls, 1);
        assert_eq!(
            calls_of(&calls, methods::RUN_WORKING_TREE_ACTION)[0]["action"]["message"],
            WRITTEN
        );
        tab.read_with(cx, |tab, _| {
            assert!(
                tab.model_override.is_none(),
                "the override ends with the commit"
            );
            assert!(tab.written.is_none());
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-write-model-override").is_none());
        click(cx, "git-write-ai");
        settle(cx, |_| writes(&calls).len() == 3);
        assert_eq!(
            writes(&calls)[2]["harness"],
            serde_json::to_value(zeron_proto::HarnessId::Codex).unwrap(),
            "the next commit goes back to the default model"
        );
        assert_eq!(writes(&calls)[2]["model"], "gpt-5-mini");
    }

    #[gpui::test]
    fn escape_stops_waiting_and_a_late_reply_leaves_the_box_untouched(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        lab.hold_writes
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let (tab, cx, calls, _dir) = open_lab(cx, lab_state(), lab.clone());
        cx.update(|_, cx| crate::composer::init(cx, Default::default()));
        settle(cx, |cx| cx.debug_bounds("git-write-ai").is_some());
        settle(cx, |cx| tab.read_with(cx, |tab, _| tab.tree.is_some()));
        set_message(&tab, cx, "My own words");

        click(cx, "git-write-ai");
        settle(cx, |_| writes(&calls).len() == 1);
        cx.run_until_parked();
        assert!(tab.read_with(cx, |tab, _| tab.writing()));
        assert!(cx.debug_bounds("git-write-preview").is_some());
        cx.simulate_keystrokes("cmd-enter");
        cx.run_until_parked();
        assert!(
            calls_of(&calls, methods::RUN_WORKING_TREE_ACTION).is_empty(),
            "no commit while a message is being written"
        );

        press_escape(&tab, cx);
        cx.run_until_parked();
        assert!(!tab.read_with(cx, |tab, _| tab.writing()));
        assert!(cx.debug_bounds("git-write-preview").is_none());

        lab.release_write.notify_one();
        settle(cx, |_| {
            lab.writes_answered
                .load(std::sync::atomic::Ordering::SeqCst)
                == 1
        });
        for _ in 0..20 {
            cx.run_until_parked();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(message(&tab, cx), "My own words");
        tab.read_with(cx, |tab, _| {
            assert!(tab.written.is_none() && tab.write_error.is_none() && !tab.writing());
        });
        assert!(calls_of(&calls, methods::RUN_WORKING_TREE_ACTION).is_empty());
    }

    #[gpui::test]
    fn a_failed_write_keeps_the_message_and_says_why(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        *lab.write_error.lock().unwrap() = Some("Claude usage limit reached.".into());
        let (tab, cx, calls, _dir) = open_lab(cx, lab_state(), lab);
        settle(cx, |cx| cx.debug_bounds("git-write-ai").is_some());
        settle(cx, |cx| tab.read_with(cx, |tab, _| tab.tree.is_some()));
        set_message(&tab, cx, "Draft");
        click(cx, "git-write-ai");
        settle(cx, |cx| cx.debug_bounds("git-write-error").is_some());
        assert_eq!(message(&tab, cx), "Draft");
        assert!(calls_of(&calls, methods::RUN_WORKING_TREE_ACTION).is_empty());
        tab.read_with(cx, |tab, _| {
            assert!(
                tab.write_error
                    .as_ref()
                    .is_some_and(|error| error.contains("usage limit"))
            );
        });
    }

    #[gpui::test]
    fn nothing_staged_disables_writing_and_the_gear_opens_source_control_settings(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        let mut unstaged = tree();
        unstaged["files"][0]["index"] = "unchanged".into();
        *lab.tree.lock().unwrap() = Some(unstaged);
        let (tab, cx, calls, _dir) = open_lab(cx, lab_state(), lab);
        settle(cx, |cx| cx.debug_bounds("git-write-ai").is_some());
        settle(cx, |cx| tab.read_with(cx, |tab, _| tab.tree.is_some()));
        click(cx, "git-write-ai");
        cx.run_until_parked();
        assert!(writes(&calls).is_empty());

        let opened = std::rc::Rc::new(std::cell::Cell::new(0));
        cx.update(|_, cx| {
            let opened = opened.clone();
            cx.on_action(move |_: &OpenSourceControlSettings, _| {
                opened.set(opened.get() + 1);
            });
        });
        click(cx, "git-write-settings");
        cx.run_until_parked();
        assert_eq!(opened.get(), 1);
    }

    #[gpui::test]
    fn clicking_a_model_row_in_the_chip_menu_sets_the_override_and_leaves_the_box_alone(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        let (tab, cx, calls, _dir) = open_lab(cx, lab_state(), lab);
        cx.update(|_, cx| crate::composer::init(cx, Default::default()));
        settle(cx, |cx| cx.debug_bounds("picker-model").is_some());
        settle(cx, |cx| tab.read_with(cx, |tab, _| tab.tree.is_some()));
        set_message(&tab, cx, "Draft");

        click(cx, "picker-model");
        settle(cx, |cx| cx.debug_bounds("model-row-5").is_some());
        let row = cx.debug_bounds("model-row-5").unwrap();
        let commit_box = cx.debug_bounds("git-commit-message").unwrap();
        assert!(
            commit_box.contains(&row.center()),
            "the last row sits over the commit box, as in the app"
        );
        click(cx, "model-row-5");
        cx.run_until_parked();
        tab.read_with(cx, |tab, cx| {
            assert!(
                !tab.model_picker.read(cx).is_open(),
                "the pick closes the menu"
            );
            assert_eq!(
                tab.model_override,
                Some(CommitModelChoice {
                    harness: zeron_proto::HarnessId::ClaudeCode,
                    model: "claude-sonnet-5".into(),
                })
            );
        });
        let note = cx
            .debug_bounds("git-write-model-override")
            .expect("the override is named");
        assert!(
            note.top() >= cx.debug_bounds("git-commit-message").unwrap().bottom(),
            "the note sits under the box, leaving the chip row to the model name"
        );
        assert_eq!(message(&tab, cx), "Draft");

        std::thread::sleep(Duration::from_millis(500));
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("model-row-5").is_none(),
            "the menu unmounted"
        );
        click(cx, "git-write-ai");
        settle(cx, |_| writes(&calls).len() == 1);
        assert_eq!(writes(&calls)[0]["model"], "claude-sonnet-5");
    }

    fn stashed_tree(agent_active: bool) -> serde_json::Value {
        let mut tree = tree();
        tree["agentActive"] = agent_active.into();
        tree["stashes"] = serde_json::json!([
            {"oid": "1111111111111111111111111111111111111111", "message": "On feature/7: parked panel",
             "createdAt": "2026-10-01T00:00:00Z"},
            {"oid": "2222222222222222222222222222222222222222", "message": "WIP on feature/7: aaa Tip"},
        ]);
        tree
    }

    fn actions(calls: &Calls) -> Vec<serde_json::Value> {
        calls_of(calls, methods::RUN_WORKING_TREE_ACTION)
    }

    #[gpui::test]
    fn discard_waits_for_the_inline_confirm_and_cancel_or_escape_send_nothing(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        let (tab, cx, calls, _dir) = open_lab(cx, lab_state(), lab);
        settle(cx, |cx| cx.debug_bounds("git-changes-discard-1").is_some());
        assert!(
            cx.debug_bounds("git-staged-discard-0").is_none(),
            "staged rows have no discard"
        );

        click(cx, "git-changes-discard-1");
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-discard-confirm").is_some());
        assert!(
            calls_of(&calls, methods::GET_WORKING_TREE_FILE_DIFF).is_empty(),
            "the discard button does not expand the row"
        );
        click(cx, "git-discard-no");
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-discard-confirm").is_none());

        click(cx, "git-changes-discard-1");
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-discard-confirm").is_some());
        press_escape(&tab, cx);
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-discard-confirm").is_none());
        assert!(
            actions(&calls).is_empty(),
            "nothing runs before the confirm"
        );

        click(cx, "git-changes-discard-1");
        cx.run_until_parked();
        click(cx, "git-discard-yes");
        settle_action(cx, &tab, &calls, 1);
        assert_eq!(
            actions(&calls)[0],
            serde_json::json!({
                "spaceId": "space-1", "expectedRevision": "r1", "busyPolicy": "safeOnly",
                "action": {"kind": "discard", "paths": ["notes/new file.txt"]},
            })
        );
        assert!(cx.debug_bounds("git-discard-confirm").is_none());
    }

    #[gpui::test]
    fn a_discard_confirm_closes_when_the_file_changes_and_a_late_change_is_refused(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        let (tab, cx, calls, _dir) = open_lab(cx, lab_state(), lab.clone());
        settle(cx, |cx| cx.debug_bounds("git-changes-discard-0").is_some());

        click(cx, "git-changes-discard-0");
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-discard-confirm").is_some());
        let mut changed = tree();
        changed["revision"] = "r-edited".into();
        *lab.tree.lock().unwrap() = Some(changed);
        tab.update(cx, |tab, cx| tab.load(cx));
        settle(cx, |cx| tab.read_with(cx, |tab, _| !tab.loading()));
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("git-discard-confirm").is_none(),
            "a confirm never outlives the revision it was asked for"
        );
        assert!(actions(&calls).is_empty());

        *lab.action_error.lock().unwrap() = Some(
            "The working tree changed since it was shown. Check the refreshed list and try again."
                .into(),
        );
        click(cx, "git-changes-discard-0");
        cx.run_until_parked();
        click(cx, "git-discard-yes");
        settle(cx, |cx| cx.debug_bounds("git-error-0").is_some());
        assert_eq!(actions(&calls).len(), 1);
        assert_eq!(actions(&calls)[0]["expectedRevision"], "r-edited");
        assert_eq!(
            actions(&calls)[0]["action"],
            serde_json::json!({"kind": "discard", "paths": ["src/a b.rs"]})
        );
        tab.read_with(cx, |tab, _| {
            assert!(
                tab.action_error
                    .as_ref()
                    .is_some_and(|error| error.contains("changed since it was shown"))
            );
        });
    }

    #[gpui::test]
    fn the_stash_menu_pushes_pops_and_deletes_only_after_a_confirm(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        *lab.tree.lock().unwrap() = Some(stashed_tree(false));
        let (tab, cx, calls, _dir) = open_lab(cx, lab_state(), lab.clone());
        settle(cx, |cx| cx.debug_bounds("git-stash-button").is_some());
        settle(cx, |cx| tab.read_with(cx, |tab, _| tab.tree.is_some()));

        click(cx, "git-stash-button");
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-stash-push").is_some());
        assert!(cx.debug_bounds("git-stash-pop-1").is_some());
        click(cx, "git-stash-push");
        settle_action(cx, &tab, &calls, 1);
        assert_eq!(
            actions(&calls)[0],
            serde_json::json!({
                "spaceId": "space-1", "expectedRevision": "r1", "busyPolicy": "safeOnly",
                "action": {"kind": "stashPush"},
            })
        );
        assert!(
            cx.debug_bounds("git-stash-push").is_none(),
            "the menu closed"
        );

        click(cx, "git-stash-button");
        cx.run_until_parked();
        click(cx, "git-stash-drop-1");
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-stash-drop-confirm").is_some());
        press_escape(&tab, cx);
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-stash-drop-confirm").is_none());
        assert!(
            cx.debug_bounds("git-stash-drop-1").is_some(),
            "the menu stays open"
        );
        click(cx, "git-stash-drop-1");
        cx.run_until_parked();
        click(cx, "git-stash-drop-no");
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-stash-drop-confirm").is_none());
        assert_eq!(actions(&calls).len(), 1, "Delete runs nothing on its own");

        click(cx, "git-stash-drop-1");
        cx.run_until_parked();
        click(cx, "git-stash-drop-yes");
        settle_action(cx, &tab, &calls, 2);
        assert_eq!(
            actions(&calls)[1]["action"],
            serde_json::json!({"kind": "stashDrop", "oid": "2222222222222222222222222222222222222222"})
        );
        assert_eq!(actions(&calls)[1]["expectedRevision"], "r2");

        *lab.action_reply.lock().unwrap() =
            Some(serde_json::json!({"conflicts": ["src/a b.rs", "notes/new file.txt"]}));
        click(cx, "git-stash-button");
        cx.run_until_parked();
        click(cx, "git-stash-pop-0");
        settle_action(cx, &tab, &calls, 3);
        assert_eq!(
            actions(&calls)[2]["action"],
            serde_json::json!({"kind": "stashPop", "oid": "1111111111111111111111111111111111111111"})
        );
        assert!(cx.debug_bounds("git-stash-note").is_some());
        tab.read_with(cx, |tab, _| {
            assert_eq!(
                tab.stash_note.as_deref(),
                Some(
                    "The pop stopped on conflicts in 2 files (src/a b.rs, notes/new file.txt). The stash is kept; resolve the conflicts, then delete it."
                )
            );
        });
    }

    #[gpui::test]
    fn discard_and_stash_follow_the_busy_policy_while_an_agent_runs(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        *lab.tree.lock().unwrap() = Some(stashed_tree(true));
        let (tab, cx, calls, _dir) = open_lab(cx, lab_state(), lab);
        settle(cx, |cx| cx.debug_bounds("git-agent-active").is_some());
        for policy in [
            crate::settings::GitAgentBusyPolicy::SafeOnly,
            crate::settings::GitAgentBusyPolicy::BlockAll,
        ] {
            cx.update(|_, cx| {
                crate::settings::update(crate::settings::SavePolicy::Immediate, cx, |settings| {
                    settings.git_agent_busy_policy = policy;
                });
            });
            cx.run_until_parked();
            click(cx, "git-changes-discard-0");
            cx.run_until_parked();
            assert!(
                cx.debug_bounds("git-discard-confirm").is_none(),
                "{policy:?}"
            );
            click(cx, "git-stash-button");
            cx.run_until_parked();
            for selector in ["git-stash-push", "git-stash-pop-0", "git-stash-drop-0"] {
                click(cx, selector);
                cx.run_until_parked();
            }
            assert!(cx.debug_bounds("git-stash-drop-confirm").is_none());
            tab.update(cx, |tab, cx| {
                tab.stash_menu = false;
                cx.notify();
            });
            cx.run_until_parked();
        }
        assert!(actions(&calls).is_empty());

        cx.update(|_, cx| {
            crate::settings::update(crate::settings::SavePolicy::Immediate, cx, |settings| {
                settings.git_agent_busy_policy = crate::settings::GitAgentBusyPolicy::AllowAll;
            });
        });
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("git-agent-active").is_some(),
            "the banner stays"
        );
        click(cx, "git-changes-discard-0");
        cx.run_until_parked();
        click(cx, "git-discard-yes");
        settle_action(cx, &tab, &calls, 1);
        assert_eq!(actions(&calls)[0]["busyPolicy"], "allowAll");
        assert_eq!(actions(&calls)[0]["action"]["kind"], "discard");
    }

    fn new_branch_tree() -> serde_json::Value {
        let mut tree = tree();
        tree["branch"] = "feature/new".into();
        tree["upstream"] = serde_json::Value::Null;
        tree["ahead"] = 1.into();
        tree
    }

    fn open_loaded(
        cx: &mut gpui::TestAppContext,
        state: AppState,
        lab: Arc<GitLab>,
    ) -> (
        Entity<GitTab>,
        &mut gpui::VisualTestContext,
        Calls,
        tempfile::TempDir,
    ) {
        let (tab, cx, calls, dir) = open_lab(cx, state, lab);
        settle(cx, |cx| {
            cx.debug_bounds("git-commit-button").is_some()
                && tab.read_with(cx, |tab, _| tab.tree.is_some())
        });
        (tab, cx, calls, dir)
    }

    fn commit_now(cx: &mut gpui::VisualTestContext, tab: &Entity<GitTab>, calls: &Calls) {
        set_message(tab, cx, "Fix the panel");
        click(cx, "git-commit-button");
        settle_action(cx, tab, calls, 1);
        settle(cx, |cx| {
            tab.read_with(cx, |tab, _| {
                tab.tree.as_ref().unwrap().head_sha.as_deref() == Some("bbb")
            })
        });
    }

    fn remote_calls(calls: &Calls) -> Vec<(String, serde_json::Value)> {
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(method, _)| method == methods::SYNC_BRANCH)
            .cloned()
            .collect()
    }

    #[gpui::test]
    fn push_asks_first_naming_the_remote_branch_and_count(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (tab, cx, calls, _dir) = open_loaded(cx, lab_state(), Arc::new(GitLab::default()));
        click(cx, "pull-requests-git-push");
        cx.run_until_parked();
        tab.read_with(cx, |tab, _| {
            assert_eq!(
                tab.push_prompt().unwrap().as_ref(),
                "Push 2 commits from feature/7 to origin/feature/7?"
            )
        });
        assert!(cx.debug_bounds("git-push-popover").is_some());
        assert!(remote_calls(&calls).is_empty(), "the chip only asks");
        click(cx, "git-push-confirm");
        settle(cx, |cx| {
            tab.read_with(cx, |tab, _| tab.sync_outcome(SyncKind::Push).is_some())
        });
        assert_eq!(
            calls_of(&calls, methods::SYNC_BRANCH),
            [serde_json::json!({
                "spaceId": "space-1", "expectedHeadSha": "aaa", "busyPolicy": "safeOnly",
                "direction": {"kind": "push", "setUpstream": false},
            })]
        );
        tab.read_with(cx, |tab, _| {
            let outcome = tab.sync_outcome(SyncKind::Push).unwrap();
            assert_eq!(
                outcome.text.as_ref(),
                "Pushed 2 commits to origin/feature/7."
            );
            assert!(!outcome.failed);
        });
        assert!(
            remote_calls(&calls)
                .iter()
                .all(|(_, params)| !params.to_string().contains("force"))
        );

        let tree: WorkingTree = serde_json::from_value(new_branch_tree()).unwrap();
        tab.update(cx, |tab, _| tab.tree = Some(tree));
        tab.read_with(cx, |tab, _| {
            assert_eq!(
                tab.push_prompt().unwrap().as_ref(),
                "Push 1 commit to origin as the new branch feature/new? It will track origin/feature/new."
            )
        });
    }

    #[gpui::test]
    fn the_uncommitted_row_follows_the_tree(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        let (tab, cx, _calls, _dir) = open_loaded(cx, lab_state(), lab.clone());
        settle(cx, |cx| cx.debug_bounds("git-working-tree-row").is_some());
        let history = tab.read_with(cx, |tab, _| tab.history.clone());
        history.read_with(cx, |history, _| assert_eq!(history.working_tree(), Some(2)));

        let mut clean = tree();
        clean["files"] = serde_json::json!([]);
        *lab.tree.lock().unwrap() = Some(clean);
        tab.update(cx, |tab, cx| tab.load(cx));
        settle(cx, |cx| {
            tab.read_with(cx, |tab, _| tab.tree.as_ref().unwrap().files.is_empty())
        });
        history.read_with(cx, |history, _| assert_eq!(history.working_tree(), Some(0)));
        assert!(cx.debug_bounds("git-working-tree-row").is_some());
    }

    fn tracking_main_tree() -> serde_json::Value {
        let mut tree = new_branch_tree();
        tree["upstream"] = "origin/main".into();
        tree
    }

    #[gpui::test]
    fn a_branch_tracking_origin_main_is_published_under_its_own_name(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        *lab.tree.lock().unwrap() = Some(tracking_main_tree());
        let (tab, cx, calls, _dir) = open_loaded(cx, lab_state(), lab);
        click(cx, "pull-requests-git-push");
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-push-popover").is_some());
        tab.read_with(cx, |tab, _| {
            assert_eq!(
                tab.push_prompt().unwrap().as_ref(),
                "Push 1 commit from feature/new to origin/feature/new? It will track origin/feature/new instead of origin/main."
            )
        });
        assert!(remote_calls(&calls).is_empty(), "the chip only asks");
        click(cx, "git-push-confirm");
        settle(cx, |cx| {
            tab.read_with(cx, |tab, _| tab.sync_outcome(SyncKind::Push).is_some())
        });
        assert_eq!(
            calls_of(&calls, methods::SYNC_BRANCH),
            [serde_json::json!({
                "spaceId": "space-1", "expectedHeadSha": "aaa", "busyPolicy": "safeOnly",
                "direction": {"kind": "push", "setUpstream": true},
            })]
        );
        tab.read_with(cx, |tab, _| {
            assert_eq!(
                tab.sync_outcome(SyncKind::Push).unwrap().text.as_ref(),
                "Pushed 1 commit to origin/feature/new."
            )
        });

        let mut triangular = tracking_main_tree();
        triangular["pushRemote"] = "fork".into();
        let triangular: WorkingTree = serde_json::from_value(triangular).unwrap();
        assert!(push_sets_upstream(&triangular));
        tab.update(cx, |tab, _| tab.tree = Some(triangular));
        tab.read_with(cx, |tab, _| {
            assert_eq!(
                tab.push_prompt().unwrap().as_ref(),
                "Push 1 commit from feature/new to fork/feature/new?",
                "a push remote other than the tracked one keeps the upstream"
            )
        });
    }

    #[gpui::test]
    fn pull_runs_on_click_and_shows_the_result_or_gits_refusal(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        let (tab, cx, calls, _dir) = open_loaded(cx, lab_state(), lab.clone());
        click(cx, "pull-requests-git-pull");
        settle(cx, |cx| {
            tab.read_with(cx, |tab, _| tab.sync_outcome(SyncKind::Pull).is_some())
        });
        assert_eq!(
            calls_of(&calls, methods::SYNC_BRANCH),
            [serde_json::json!({
                "spaceId": "space-1", "expectedHeadSha": "aaa", "busyPolicy": "safeOnly",
                "direction": {"kind": "pull"},
            })]
        );
        assert!(cx.debug_bounds("git-pull-popover").is_some());
        tab.read_with(cx, |tab, _| {
            assert_eq!(
                tab.sync_outcome(SyncKind::Pull).unwrap().text.as_ref(),
                "Pulled 2 commits into feature/7."
            )
        });

        *lab.sync_error.lock().unwrap() = Some(
            "This branch and its upstream have diverged, and Pull only fast-forwards. Merge or rebase in a terminal.\ngit: fatal: Not possible to fast-forward, aborting."
                .into(),
        );
        settle(cx, |cx| {
            tab.read_with(cx, |tab, _| {
                tab.tree.as_ref().unwrap().head_sha.as_deref() == Some("ccc")
            })
        });
        tab.update(cx, |tab, cx| tab.open_sync(SyncKind::Pull, cx));
        settle(cx, |cx| {
            tab.read_with(cx, |tab, _| tab.sync_outcome(SyncKind::Pull).is_some())
        });
        tab.read_with(cx, |tab, _| {
            let outcome = tab.sync_outcome(SyncKind::Pull).unwrap();
            assert!(outcome.failed);
            assert!(outcome.text.contains("Not possible to fast-forward"));
        });
    }

    #[gpui::test]
    fn the_busy_policy_lets_push_through_safe_only_and_holds_pull(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        let mut busy = tree();
        busy["agentActive"] = true.into();
        *lab.tree.lock().unwrap() = Some(busy);
        let (tab, cx, calls, _dir) = open_loaded(cx, lab_state(), lab);
        assert!(cx.debug_bounds("git-agent-active").is_some());
        click(cx, "pull-requests-git-pull");
        cx.run_until_parked();
        assert!(calls_of(&calls, methods::SYNC_BRANCH).is_empty());
        tab.read_with(cx, |tab, cx| {
            assert!(
                tab.sync_refusal(SyncKind::Pull, cx)
                    .unwrap()
                    .contains("Pull waits")
            );
            assert_eq!(tab.sync_refusal(SyncKind::Push, cx), None);
        });

        for (policy, push, pull) in [
            (crate::settings::GitAgentBusyPolicy::BlockAll, false, false),
            (crate::settings::GitAgentBusyPolicy::AllowAll, true, true),
        ] {
            cx.update(|_, cx| {
                crate::settings::update(crate::settings::SavePolicy::Immediate, cx, |settings| {
                    settings.git_agent_busy_policy = policy;
                });
            });
            tab.read_with(cx, |tab, cx| {
                assert_eq!(
                    tab.sync_refusal(SyncKind::Push, cx).is_none(),
                    push,
                    "{policy:?}"
                );
                assert_eq!(
                    tab.sync_refusal(SyncKind::Pull, cx).is_none(),
                    pull,
                    "{policy:?}"
                );
            });
        }
        cx.update(|_, cx| {
            crate::settings::update(crate::settings::SavePolicy::Immediate, cx, |settings| {
                settings.git_agent_busy_policy = crate::settings::GitAgentBusyPolicy::BlockAll;
            });
        });
        tab.update(cx, |tab, cx| {
            tab.start_sync(SyncKind::Push, cx);
            tab.start_sync(SyncKind::Pull, cx);
        });
        cx.run_until_parked();
        assert!(calls_of(&calls, methods::SYNC_BRANCH).is_empty());
    }

    #[gpui::test]
    fn an_engine_without_branch_sync_gets_no_push_or_pull(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        *lab.tree.lock().unwrap() = Some(new_branch_tree());
        let mut state = lab_state();
        state.devices[0].capabilities = vec![
            zeron_proto::capabilities::SOURCE_CONTROL_V1.into(),
            zeron_proto::capabilities::SOURCE_CONTROL_COMMITS_V1.into(),
        ];
        let (tab, cx, calls, _dir) = open_loaded(cx, state, lab);
        assert!(cx.debug_bounds("pull-requests-git-checkout").is_some());
        assert!(cx.debug_bounds("pull-requests-git-push").is_none());
        assert!(cx.debug_bounds("pull-requests-git-pull").is_none());
        commit_now(cx, &tab, &calls);
        assert!(cx.debug_bounds("git-last-commit").is_some());
        tab.update(cx, |tab, cx| tab.start_sync(SyncKind::Push, cx));
        cx.run_until_parked();
        assert!(remote_calls(&calls).is_empty());
    }

    #[gpui::test]
    fn line_counts_line_up_across_staged_and_unstaged_rows(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (_tab, cx, _calls, _dir) = open_tab(cx, lab_state());
        settle(cx, |cx| cx.debug_bounds("git-changes-count-0").is_some());
        let staged = cx.debug_bounds("git-staged-count-0").unwrap();
        let changed = cx.debug_bounds("git-changes-count-0").unwrap();
        assert_eq!(staged.right(), changed.right());
    }

    const PICKED: &str = "c0ffee0000000000000000000000000000000001";
    const LOOSE: &str = "c0ffee0000000000000000000000000000000002";

    fn history_commit(sha: &str, branches: &[&str], parents: usize) -> GitHistoryCommit {
        serde_json::from_value(serde_json::json!({
            "sha": sha,
            "parentShas": (0..parents).map(|n| format!("{n}a{n}b{n}c{n}d")).collect::<Vec<_>>(),
            "subject": "Panel: keep the scroll", "authorName": "Alice",
            "authorEmail": "a@example.com", "authoredAt": "2026-10-01T00:00:00Z",
            "refs": branches.iter().map(|label| serde_json::json!({"kind": "branch", "label": label})).collect::<Vec<_>>(),
        }))
        .unwrap()
    }

    fn clean_tree() -> serde_json::Value {
        let mut tree = tree();
        tree["files"] = serde_json::json!([
            {"path": "notes/new file.txt", "index": "untracked", "worktree": "untracked",
             "unstaged": {"additions": 4, "deletions": 0}}
        ]);
        tree
    }

    fn open_detail_lab(
        cx: &mut gpui::TestAppContext,
        state: AppState,
        lab: Arc<GitLab>,
    ) -> (
        &mut gpui::VisualTestContext,
        Calls,
        tempfile::TempDir,
        Entity<GitTab>,
    ) {
        let (tab, cx, calls, dir) = open_lab(cx, state, lab);
        settle(cx, |cx| cx.debug_bounds("git-working-tree-row").is_some());
        settle(cx, |cx| tab.read_with(cx, |tab, _| tab.tree.is_some()));
        (cx, calls, dir, tab)
    }

    fn open_commit(
        cx: &mut gpui::VisualTestContext,
        tab: &Entity<GitTab>,
        commit: GitHistoryCommit,
    ) {
        let history = tab.read_with(cx, |tab, _| tab.history.clone());
        history.update(cx, |_, cx| cx.emit(GitHistoryEvent::OpenCommit(commit)));
        cx.run_until_parked();
    }

    fn show_commit(
        cx: &mut gpui::VisualTestContext,
        tab: &Entity<GitTab>,
        commit: GitHistoryCommit,
    ) {
        open_commit(cx, tab, commit);
        settle(cx, |cx| cx.debug_bounds("git-commit-file-1").is_some());
    }

    fn text_of(tab: &Entity<GitTab>, cx: &mut gpui::VisualTestContext) -> Option<(String, bool)> {
        tab.read_with(cx, |tab, _| {
            tab.detail
                .as_ref()
                .and_then(|detail| detail.outcome.clone())
                .map(|(text, failed)| (text.to_string(), failed))
        })
    }

    #[gpui::test]
    fn a_selected_commit_shows_its_detail_until_the_working_tree_is_selected_again(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        *lab.tree.lock().unwrap() = Some(clean_tree());
        let (cx, calls, _dir, tab) = open_detail_lab(cx, lab_state(), lab);
        assert!(cx.debug_bounds("git-commit-button").is_some());

        show_commit(cx, &tab, history_commit(PICKED, &["feature/x"], 1));
        assert_eq!(
            calls_of(&calls, methods::GET_CHECKOUT_DIFF),
            [serde_json::json!({"cwd": "/lab/repo", "mode": "commit", "commitSha": PICKED})]
        );
        assert!(cx.debug_bounds("git-commit-detail").is_some());
        assert!(cx.debug_bounds("git-commit-subject").is_some());
        let selected = |cx: &mut gpui::VisualTestContext| {
            let history = tab.read_with(cx, |tab, _| tab.history.clone());
            history.read_with(cx, |history, _| history.selected_sha().map(str::to_owned))
        };
        assert_eq!(selected(cx).as_deref(), Some(PICKED));

        click(cx, "git-commit-file-0");
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-commit-diff-0").is_some());
        assert!(cx.debug_bounds("git-commit-diff-1").is_none());
        click(cx, "git-commit-open-diff");
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-commit-diff-0").is_some());
        assert!(cx.debug_bounds("git-commit-diff-1").is_some());
        click(cx, "git-commit-open-diff");
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-commit-diff-0").is_none());

        click(cx, "git-commit-copy-sha");
        cx.run_until_parked();
        assert_eq!(
            cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text())),
            Some(PICKED.to_string())
        );
        tab.read_with(cx, |tab, cx| {
            assert_eq!(tab.history.read(cx).copied_sha(), Some(PICKED))
        });

        click(cx, "git-working-tree-row");
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-commit-button").is_some());
        assert!(cx.debug_bounds("git-commit-detail").is_none());
        assert_eq!(selected(cx), None);

        show_commit(cx, &tab, history_commit(PICKED, &[], 1));
        open_commit(cx, &tab, history_commit(PICKED, &[], 1));
        assert!(
            cx.debug_bounds("git-commit-detail").is_none(),
            "clicking the selected row again goes back to the history"
        );
        show_commit(cx, &tab, history_commit(LOOSE, &[], 1));
        click(cx, "git-commit-close");
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-commit-detail").is_none());
        assert!(calls_of(&calls, methods::RUN_WORKING_TREE_ACTION).is_empty());
    }

    #[gpui::test]
    fn an_open_commit_diff_lays_out_only_the_rows_near_the_viewport(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        *lab.tree.lock().unwrap() = Some(clean_tree());
        let (cx, _calls, _dir, tab) = open_detail_lab(cx, lab_state(), lab);
        show_commit(cx, &tab, history_commit(PICKED, &["feature/x"], 1));
        let lines: String = (0..DIFF_LINE_LIMIT)
            .map(|n| format!("+line {n}\n"))
            .collect();
        let patch = format!(
            "diff --git a/src/panel.rs b/src/panel.rs\n--- a/src/panel.rs\n+++ b/src/panel.rs\n@@ -0,0 +1,{DIFF_LINE_LIMIT} @@\n{lines}"
        );
        tab.update(cx, |tab, cx| {
            let detail = tab.detail.as_mut().unwrap();
            let Loadable::Ready((diff, parsed)) = &mut detail.diff else {
                panic!("a loaded commit");
            };
            diff.files.truncate(1);
            *parsed = changes::parse_patch(&patch);
            detail.sync_rows();
            cx.notify();
        });
        click(cx, "git-commit-open-diff");
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-commit-diff-0").is_some());
        tab.read_with(cx, |tab, _| {
            let detail = tab.detail.as_ref().unwrap();
            assert_eq!(detail.rows.len(), DIFF_LINE_LIMIT + 3);
            assert!(detail.list.bounds_for_item(3).is_some());
            assert!(
                detail.list.bounds_for_item(detail.rows.len() - 1).is_none(),
                "rows far below the viewport are never laid out"
            );
        });
        assert!(cx.debug_bounds("git-commit-sticky-file").is_none());
        tab.update(cx, |tab, cx| {
            tab.detail
                .as_ref()
                .unwrap()
                .list
                .scroll_to(gpui::ListOffset {
                    item_ix: 40,
                    offset_in_item: px(0.0),
                });
            cx.notify();
        });
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("git-commit-sticky-file").is_some(),
            "the open file's row stays pinned while its diff scrolls"
        );
    }

    #[test]
    fn hunk_headers_split_the_range_from_the_function_context() {
        assert_eq!(
            hunk_header_parts("@@ -10,7 +10,8 @@ fn render(&mut self)"),
            ("@@ -10,7 +10,8 @@", "fn render(&mut self)")
        );
        assert_eq!(hunk_header_parts("@@ -0,0 +1 @@"), ("@@ -0,0 +1 @@", ""));
        assert_eq!(hunk_header_parts("not a hunk"), ("not a hunk", ""));
    }

    #[gpui::test]
    fn checkout_switches_to_a_branch_at_once_and_asks_before_detaching(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        *lab.tree.lock().unwrap() = Some(clean_tree());
        let (cx, calls, _dir, tab) = open_detail_lab(cx, lab_state(), lab);

        show_commit(cx, &tab, history_commit(PICKED, &["feature/x"], 1));
        click(cx, "git-commit-checkout");
        settle_action(cx, &tab, &calls, 1);
        assert_eq!(
            actions(&calls)[0],
            serde_json::json!({
                "spaceId": "space-1", "expectedRevision": "r1", "busyPolicy": "safeOnly",
                "action": {"kind": "checkout", "targetRef": "feature/x", "detach": false},
            })
        );
        assert_eq!(
            text_of(&tab, cx),
            Some(("Checked out feature/x.".into(), false))
        );

        show_commit(cx, &tab, history_commit(LOOSE, &[], 1));
        click(cx, "git-commit-checkout");
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-commit-confirm").is_some());
        press_escape(&tab, cx);
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-commit-confirm").is_none());
        click(cx, "git-commit-checkout");
        cx.run_until_parked();
        click(cx, "git-commit-confirm-no");
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-commit-confirm").is_none());
        assert_eq!(
            actions(&calls).len(),
            1,
            "no detached checkout before the confirm"
        );

        click(cx, "git-commit-checkout");
        cx.run_until_parked();
        click(cx, "git-commit-confirm-yes");
        settle_action(cx, &tab, &calls, 2);
        assert_eq!(
            actions(&calls)[1]["action"],
            serde_json::json!({"kind": "checkout", "targetRef": LOOSE, "detach": true})
        );
        assert!(text_of(&tab, cx).unwrap().0.contains("HEAD is detached"));

        let mut on_branch = clean_tree();
        on_branch["headSha"] = PICKED.into();
        on_branch["branch"] = "feature/x".into();
        let tree: WorkingTree = serde_json::from_value(on_branch).unwrap();
        let commit = history_commit(PICKED, &["feature/x"], 1);
        assert_eq!(checkout_target(&commit, &tree), None, "already checked out");
        let commit = history_commit(PICKED, &["feature/x", "feature/y"], 1);
        assert_eq!(
            checkout_target(&commit, &tree),
            Some(("feature/y".into(), false))
        );
    }

    #[gpui::test]
    fn cherry_pick_asks_first_and_a_conflict_opens_the_working_tree_with_abort(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        *lab.tree.lock().unwrap() = Some(clean_tree());
        *lab.action_reply.lock().unwrap() =
            Some(serde_json::json!({"conflicts": ["src/panel.rs"]}));
        let (cx, calls, _dir, tab) = open_detail_lab(cx, lab_state(), lab.clone());

        show_commit(cx, &tab, history_commit(LOOSE, &[], 2));
        assert!(cx.debug_bounds("git-commit-refusal").is_some());
        click(cx, "git-commit-cherry-pick");
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("git-commit-confirm").is_none(),
            "a merge commit is not picked"
        );

        show_commit(cx, &tab, history_commit(PICKED, &[], 1));
        assert!(cx.debug_bounds("git-commit-refusal").is_none());
        click(cx, "git-commit-cherry-pick");
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-commit-confirm").is_some());
        click(cx, "git-commit-confirm-no");
        cx.run_until_parked();
        assert!(actions(&calls).is_empty(), "cancel sends nothing");
        click(cx, "git-commit-cherry-pick");
        cx.run_until_parked();
        click(cx, "git-commit-confirm-yes");
        settle_action(cx, &tab, &calls, 1);
        assert_eq!(
            actions(&calls)[0]["action"],
            serde_json::json!({"kind": "cherryPick", "sha": PICKED})
        );
        settle(cx, |cx| cx.debug_bounds("git-cherry-pick-banner").is_some());
        assert!(cx.debug_bounds("git-commit-detail").is_none());
        assert_eq!(message(&tab, cx), "Panel: keep the scroll");
        tab.read_with(cx, |tab, _| assert!(tab.stash_note.is_none()));

        show_commit(cx, &tab, history_commit(LOOSE, &["feature/x"], 1));
        let refusal = tab.read_with(cx, |tab, cx| {
            tab.move_refusal(&WorkingTreeAction::CherryPick { sha: LOOSE.into() }, cx)
        });
        assert_eq!(
            refusal,
            Some("A cherry-pick is in progress. Finish or abort it first.")
        );
        click(cx, "git-commit-checkout");
        click(cx, "git-commit-cherry-pick");
        cx.run_until_parked();
        assert_eq!(actions(&calls).len(), 1);

        click(cx, "git-working-tree-row");
        cx.run_until_parked();
        *lab.action_reply.lock().unwrap() = None;
        click(cx, "git-cherry-pick-abort");
        settle_action(cx, &tab, &calls, 2);
        assert_eq!(
            actions(&calls)[1]["action"],
            serde_json::json!({"kind": "abortCherryPick"})
        );
        settle(cx, |cx| cx.debug_bounds("git-cherry-pick-banner").is_none());

        *lab.action_reply.lock().unwrap() = Some(serde_json::json!({"commit": {
            "sha": "d00d000000000000000000000000000000000000", "branch": "feature/7",
            "subject": "Panel: keep the scroll",
        }}));
        show_commit(cx, &tab, history_commit(PICKED, &[], 1));
        click(cx, "git-commit-cherry-pick");
        cx.run_until_parked();
        click(cx, "git-commit-confirm-yes");
        settle_action(cx, &tab, &calls, 3);
        assert_eq!(
            text_of(&tab, cx),
            Some((
                "Cherry-picked c0ffee0 onto feature/7 as d00d000.".into(),
                false
            ))
        );
    }

    fn type_branch(tab: &Entity<GitTab>, cx: &mut gpui::VisualTestContext, name: &str) {
        tab.update(cx, |tab, cx| {
            let input = tab
                .detail
                .as_ref()
                .unwrap()
                .branch_form
                .as_ref()
                .unwrap()
                .name
                .clone();
            input.update(cx, |input, cx| input.set_text(name, cx));
            cx.notify();
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    fn a_dirty_tree_and_the_busy_policy_gate_checkout_and_cherry_pick_but_not_a_new_branch(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        let (cx, calls, _dir, tab) = open_detail_lab(cx, lab_state(), lab.clone());

        show_commit(cx, &tab, history_commit(PICKED, &["feature/x"], 1));
        assert!(cx.debug_bounds("git-commit-refusal").is_some());
        tab.read_with(cx, |tab, cx| {
            assert_eq!(
                tab.move_refusal(&WorkingTreeAction::CherryPick { sha: PICKED.into() }, cx),
                Some("Commit or stash your changes to check out or cherry-pick.")
            )
        });
        click(cx, "git-commit-checkout");
        click(cx, "git-commit-cherry-pick");
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-commit-confirm").is_none());
        assert!(actions(&calls).is_empty());

        click(cx, "git-commit-branch");
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-branch-form").is_some());
        click(cx, "git-branch-checkout");
        cx.run_until_parked();
        tab.read_with(cx, |tab, cx| {
            assert!(!tab.can_branch(true, cx), "moving HEAD needs a clean tree")
        });
        type_branch(&tab, cx, "feature/y");
        click(cx, "git-branch-create");
        settle_action(cx, &tab, &calls, 1);
        assert_eq!(
            actions(&calls)[0]["action"],
            serde_json::json!({"kind": "createBranch", "name": "feature/y", "startPoint": PICKED, "checkout": false})
        );
        assert_eq!(
            text_of(&tab, cx),
            Some(("Created feature/y at c0ffee0.".into(), false))
        );
        assert!(cx.debug_bounds("git-branch-form").is_none());

        let mut busy = clean_tree();
        busy["agentActive"] = true.into();
        *lab.tree.lock().unwrap() = Some(busy);
        tab.update(cx, |tab, cx| tab.load(cx));
        settle(cx, |cx| cx.debug_bounds("git-agent-active").is_some());
        tab.read_with(cx, |tab, cx| {
            assert!(
                !tab.can_checkout(cx) && !tab.can_cherry_pick(cx),
                "SafeOnly holds both"
            );
        });
        click(cx, "git-commit-branch");
        cx.run_until_parked();
        type_branch(&tab, cx, "feature/busy");
        click(cx, "git-branch-checkout");
        cx.run_until_parked();
        click(cx, "git-branch-create");
        settle_action(cx, &tab, &calls, 2);
        assert_eq!(
            actions(&calls)[1]["action"],
            serde_json::json!({"kind": "createBranch", "name": "feature/busy", "startPoint": PICKED, "checkout": false}),
            "SafeOnly creates the branch but cannot check it out"
        );

        for (policy, checkout, branch) in [
            (crate::settings::GitAgentBusyPolicy::BlockAll, false, false),
            (crate::settings::GitAgentBusyPolicy::AllowAll, true, true),
        ] {
            cx.update(|_, cx| {
                crate::settings::update(crate::settings::SavePolicy::Immediate, cx, |settings| {
                    settings.git_agent_busy_policy = policy;
                });
            });
            tab.update(cx, |_, cx| cx.notify());
            cx.run_until_parked();
            tab.read_with(cx, |tab, cx| {
                assert_eq!(tab.can_checkout(cx), checkout, "{policy:?}");
                assert_eq!(tab.can_branch(false, cx), branch, "{policy:?}");
            });
        }
        click(cx, "git-commit-checkout");
        settle_action(cx, &tab, &calls, 3);
        assert_eq!(actions(&calls)[2]["busyPolicy"], "allowAll");
        assert_eq!(actions(&calls)[2]["action"]["kind"], "checkout");
    }

    #[gpui::test]
    fn create_branch_needs_a_name_and_shows_what_the_engine_refused(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        *lab.tree.lock().unwrap() = Some(clean_tree());
        let (cx, calls, _dir, tab) = open_detail_lab(cx, lab_state(), lab.clone());
        show_commit(cx, &tab, history_commit(PICKED, &[], 1));
        click(cx, "git-commit-branch");
        cx.run_until_parked();
        click(cx, "git-branch-create");
        cx.run_until_parked();
        assert!(actions(&calls).is_empty(), "no name, nothing sent");

        *lab.action_error.lock().unwrap() = Some("\"bad name\" is not a valid branch name.".into());
        type_branch(&tab, cx, "bad name");
        click(cx, "git-branch-create");
        settle_action(cx, &tab, &calls, 1);
        settle(cx, |cx| cx.debug_bounds("git-commit-outcome").is_some());
        assert_eq!(
            text_of(&tab, cx),
            Some(("\"bad name\" is not a valid branch name.".into(), true))
        );
        assert!(
            cx.debug_bounds("git-branch-form").is_some(),
            "the form stays for a fix"
        );
        assert!(cx.debug_bounds("git-error-0").is_none());

        *lab.action_error.lock().unwrap() = None;
        type_branch(&tab, cx, "feature/z");
        click(cx, "git-branch-checkout");
        cx.run_until_parked();
        click(cx, "git-branch-create");
        settle_action(cx, &tab, &calls, 2);
        assert_eq!(
            actions(&calls)[1]["action"],
            serde_json::json!({"kind": "createBranch", "name": "feature/z", "startPoint": PICKED, "checkout": true})
        );
        assert_eq!(
            text_of(&tab, cx),
            Some((
                "Created feature/z at c0ffee0 and checked it out.".into(),
                false
            ))
        );
    }

    #[gpui::test]
    fn an_engine_without_commit_actions_still_lists_files_and_copies_the_sha(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        *lab.tree.lock().unwrap() = Some(clean_tree());
        let mut state = lab_state();
        state.devices[0].capabilities.retain(|capability| {
            capability.as_str() != zeron_proto::capabilities::SOURCE_CONTROL_COMMITS_V1
        });
        let (cx, calls, _dir, tab) = open_detail_lab(cx, state, lab);
        show_commit(cx, &tab, history_commit(PICKED, &["feature/x"], 1));
        assert!(cx.debug_bounds("git-commit-refusal").is_some());
        for selector in [
            "git-commit-checkout",
            "git-commit-cherry-pick",
            "git-commit-branch",
        ] {
            click(cx, selector);
            cx.run_until_parked();
        }
        assert!(cx.debug_bounds("git-commit-confirm").is_none());
        assert!(cx.debug_bounds("git-branch-form").is_none());
        assert!(actions(&calls).is_empty());
        click(cx, "git-commit-copy-sha");
        cx.run_until_parked();
        assert_eq!(
            cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text())),
            Some(PICKED.to_string())
        );
    }

    fn tree_reads(calls: &Calls) -> usize {
        calls_of(calls, methods::GET_WORKING_TREE).len()
    }

    fn watched_lab() -> (
        Arc<GitLab>,
        tokio::sync::mpsc::UnboundedSender<serde_json::Value>,
    ) {
        let lab = Arc::new(GitLab::default());
        let (frames, receiver) = tokio::sync::mpsc::unbounded_channel();
        *lab.watch.lock().unwrap() = Some(receiver);
        (lab, frames)
    }

    fn idle_for(cx: &mut gpui::VisualTestContext, wait: Duration) {
        for _ in 0..10 {
            cx.executor().advance_clock(wait / 10);
            cx.run_until_parked();
            std::thread::sleep(Duration::from_millis(5));
        }
        cx.run_until_parked();
    }

    #[gpui::test]
    fn the_git_tab_stops_reading_the_working_tree_once_it_is_hidden(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (lab, frames) = watched_lab();
        let (tab, cx, calls, _dir) = open_lab(cx, lab_state(), lab);
        settle(cx, |cx| cx.debug_bounds("git-staged-row-0").is_some());
        tab.update(cx, |tab, _| tab.window_active = true);
        frames.send(watch_frame("w0")).unwrap();
        cx.run_until_parked();
        let before = tree_reads(&calls);
        idle_for(cx, POLL_INTERVAL * 2);
        assert!(tree_reads(&calls) > before, "the poll reads while shown");

        tab.update(cx, |tab, cx| tab.set_visible(false, cx));
        cx.run_until_parked();
        let hidden = tree_reads(&calls);
        idle_for(cx, POLL_INTERVAL * 3);
        frames.send(watch_frame("w1")).ok();
        for _ in 0..10 {
            cx.run_until_parked();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            tree_reads(&calls),
            hidden,
            "neither the poll nor the watch reads a tab nobody sees"
        );

        tab.update(cx, |tab, cx| tab.set_visible(true, cx));
        settle(cx, |_| tree_reads(&calls) > hidden);
    }

    #[gpui::test]
    fn watch_frames_during_a_slow_read_queue_one_reread_instead_of_restarting_it(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (lab, frames) = watched_lab();
        let (tab, cx, calls, _dir) = open_lab(cx, lab_state(), lab.clone());
        settle(cx, |cx| cx.debug_bounds("git-staged-row-0").is_some());
        settle(cx, |cx| tab.read_with(cx, |tab, _| !tab.loading()));
        frames.send(watch_frame("w0")).unwrap();
        cx.run_until_parked();
        lab.hold_trees
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let before = tree_reads(&calls);
        frames.send(watch_frame("w1")).unwrap();
        settle(cx, |_| tree_reads(&calls) == before + 1);
        for revision in ["w2", "w3", "w4"] {
            frames.send(watch_frame(revision)).unwrap();
            for _ in 0..5 {
                cx.run_until_parked();
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        assert_eq!(
            tree_reads(&calls),
            before + 1,
            "the in-flight read is not restarted"
        );
        lab.hold_trees
            .store(false, std::sync::atomic::Ordering::SeqCst);
        lab.release_tree.notify_one();
        settle(cx, |cx| {
            tree_reads(&calls) == before + 2 && tab.read_with(cx, |tab, _| !tab.loading())
        });
        for _ in 0..10 {
            cx.run_until_parked();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(tree_reads(&calls), before + 2, "one reread, not three");
    }

    #[gpui::test]
    fn archiving_the_picked_worktrees_thread_falls_back_to_the_main_checkout(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let mut state = lab_state();
        state.selected_chat = Some("c-main".into());
        let (page, cx, calls, _dir) = open_page(cx, state, PageTab::Git);
        open_page_tray(&page, cx);
        settle(cx, |cx| cx.debug_bounds("git-staged-row-0").is_some());
        let tab = page_git_tab(&page, cx);
        tab.update(cx, |tab, cx| {
            tab.set_checkout(Some("/lab/worktrees/idle".into()), cx)
        });
        settle(cx, |cx| tab.read_with(cx, |tab, _| tab.tree.is_some()));
        let state = page.read_with(cx, |page, _| page.state.clone());
        state.update(cx, |state, cx| {
            for chat in &mut state.chats {
                if chat.id == "c-idle" {
                    chat.archived = true;
                }
            }
            cx.notify();
        });
        cx.run_until_parked();
        tab.read_with(cx, |tab, _| assert_eq!(tab.checkout_path(), None));
        assert!(cx.debug_bounds("pull-requests-git-checkout").is_some());
        settle(cx, |_| {
            calls_of(&calls, methods::GET_WORKING_TREE)
                .last()
                .is_some_and(|params| params.get("checkoutPath").is_none())
        });
    }

    fn remote_state() -> AppState {
        let mut state = lab_state();
        state.spaces.push(
            serde_json::from_value(serde_json::json!({
                "id": "space-r", "deviceId": "studio", "path": "/studio/repo", "gitDetected": true,
                "createdAt": "2026-10-01T00:00:00Z"
            }))
            .unwrap(),
        );
        state.devices.push(
            serde_json::from_value(serde_json::json!({
                "id": "studio", "name": "Studio", "platform": "macos",
                "lastSeenAt": chrono::Utc::now().to_rfc3339(),
                "capabilities": [
                    zeron_proto::capabilities::PULL_REQUESTS_PAGE_V1,
                    zeron_proto::capabilities::SOURCE_CONTROL_V1,
                    zeron_proto::capabilities::SOURCE_CONTROL_SYNC_V1,
                    zeron_proto::capabilities::SOURCE_CONTROL_COMMITS_V1,
                ],
            }))
            .unwrap(),
        );
        state
    }

    fn remote_space() -> GitSpace {
        GitSpace {
            space_id: "space-r".into(),
            path: "/studio/repo".into(),
            device: Some("studio".into()),
        }
    }

    #[gpui::test]
    fn retargeting_drops_an_ai_written_message_but_keeps_typed_text(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (tab, cx, calls, _dir) = open_tab(cx, lab_state());
        cx.update(|_, cx| crate::composer::init(cx, Default::default()));
        settle(cx, |cx| cx.debug_bounds("git-write-ai").is_some());
        settle(cx, |cx| tab.read_with(cx, |tab, _| tab.tree.is_some()));
        click(cx, "git-write-ai");
        settle(cx, |cx| tab.read_with(cx, |tab, _| tab.written.is_some()));
        assert_eq!(writes(&calls).len(), 1);
        assert_eq!(message(&tab, cx), WRITTEN);
        tab.update(cx, |tab, cx| {
            tab.set_checkout(Some("/lab/worktrees/idle".into()), cx)
        });
        cx.run_until_parked();
        assert_eq!(
            message(&tab, cx),
            "",
            "the other checkout's message is gone"
        );

        set_message(&tab, cx, "My own words");
        tab.update(cx, |tab, cx| tab.set_checkout(None, cx));
        cx.run_until_parked();
        assert_eq!(message(&tab, cx), "My own words");
    }

    #[gpui::test]
    fn text_typed_while_a_commit_runs_survives_its_reply(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        let (tab, cx, calls, _dir) = open_lab(cx, lab_state(), lab.clone());
        settle(cx, |cx| cx.debug_bounds("git-commit-button").is_some());
        settle(cx, |cx| tab.read_with(cx, |tab, _| tab.tree.is_some()));
        set_message(&tab, cx, "First");
        lab.hold_actions
            .store(true, std::sync::atomic::Ordering::SeqCst);
        click(cx, "git-commit-button");
        settle(cx, |_| actions(&calls).len() == 1);
        set_message(&tab, cx, "Second");
        lab.release_action.notify_one();
        settle_action(cx, &tab, &calls, 1);
        tab.read_with(cx, |tab, _| assert!(tab.last_commit.is_some()));
        assert_eq!(message(&tab, cx), "Second");
    }

    #[gpui::test]
    fn a_detail_outcome_lands_only_on_the_commit_it_was_for(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        *lab.tree.lock().unwrap() = Some(clean_tree());
        let (cx, calls, _dir, tab) = open_detail_lab(cx, lab_state(), lab.clone());
        lab.hold_actions
            .store(true, std::sync::atomic::Ordering::SeqCst);

        show_commit(cx, &tab, history_commit(PICKED, &["feature/x"], 1));
        click(cx, "git-commit-checkout");
        settle(cx, |_| actions(&calls).len() == 1);
        show_commit(cx, &tab, history_commit(LOOSE, &[], 1));
        lab.release_action.notify_one();
        settle_action(cx, &tab, &calls, 1);
        assert_eq!(text_of(&tab, cx), None, "the outcome was for {PICKED}");

        *lab.tree.lock().unwrap() = Some(clean_tree());
        tab.update(cx, |tab, cx| tab.load(cx));
        settle(cx, |cx| tab.read_with(cx, |tab, _| !tab.loading()));
        *lab.action_error.lock().unwrap() = Some("Git refused the checkout.".into());
        show_commit(cx, &tab, history_commit(PICKED, &["feature/x"], 1));
        click(cx, "git-commit-checkout");
        settle(cx, |_| actions(&calls).len() == 2);
        show_commit(cx, &tab, history_commit(LOOSE, &[], 1));
        lab.release_action.notify_one();
        settle_action(cx, &tab, &calls, 2);
        assert_eq!(text_of(&tab, cx), None);
        tab.read_with(cx, |tab, _| {
            assert!(
                tab.action_error
                    .as_ref()
                    .is_some_and(|error| error.contains("refused the checkout"))
            );
        });
    }

    #[gpui::test]
    fn a_device_from_before_commits_stages_but_cannot_commit_discard_or_stash(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        *lab.tree.lock().unwrap() = Some(stashed_tree(false));
        let mut state = lab_state();
        state.devices[0].capabilities = vec![zeron_proto::capabilities::SOURCE_CONTROL_V1.into()];
        let (tab, cx, calls, _dir) = open_lab(cx, state, lab);
        cx.update(|_, cx| crate::composer::init(cx, Default::default()));
        settle(cx, |cx| cx.debug_bounds("git-changes-toggle-0").is_some());
        for selector in ["git-changes-discard-0", "git-stash-button"] {
            assert!(cx.debug_bounds(selector).is_none(), "{selector}");
        }
        set_message(&tab, cx, "Fix the panel");
        tab.update(cx, |tab, cx| tab.commit(CommitKind::Staged, cx));
        if cx.debug_bounds("git-write-ai").is_some() {
            click(cx, "git-write-ai");
        }
        cx.run_until_parked();
        assert!(actions(&calls).is_empty());
        assert!(writes(&calls).is_empty());
        click(cx, "git-changes-toggle-0");
        settle_action(cx, &tab, &calls, 1);
        assert_eq!(actions(&calls)[0]["action"]["kind"], "stage");
    }

    #[gpui::test]
    fn a_remote_project_writes_its_message_with_the_remote_engines_own_pick(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (tab, cx, calls, _dir) =
            open_lab_at(cx, remote_state(), Arc::default(), remote_space());
        cx.update(|_, cx| {
            crate::composer::init(cx, Default::default());
            crate::settings::update(crate::settings::SavePolicy::Immediate, cx, |settings| {
                settings.commit_message_model = Some(CommitModelChoice {
                    harness: zeron_proto::HarnessId::Codex,
                    model: "gpt-5-mini".into(),
                });
            });
        });
        settle(cx, |cx| cx.debug_bounds("git-write-ai").is_some());
        settle(cx, |cx| tab.read_with(cx, |tab, _| tab.tree.is_some()));
        assert!(
            cx.debug_bounds("picker-model").is_none(),
            "this device's catalog is not offered for another device's project"
        );
        click(cx, "git-write-ai");
        settle(cx, |_| writes(&calls).len() == 1);
        let sent = &writes(&calls)[0];
        assert_eq!(sent["targetDeviceId"], "studio");
        assert!(sent.get("harness").is_none() && sent.get("model").is_none());
    }
}
