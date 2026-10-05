use std::collections::HashMap;
use std::time::Duration;

use gpui::{
    Animation, AnimationExt, AnyElement, App, Context, Entity, EntityInputHandler, EventEmitter,
    FocusHandle, Focusable, KeyDownEvent, SharedString, Subscription, Task, Window, div,
    percentage, prelude::*, px,
};
use zeron_proto::{
    ChangeRequestListItem, ChangeRequestState, CommitSummary, GenerateCommitMessageRequest,
    GeneratedCommitMessage, GetWorkingTreeFileDiffRequest, GetWorkingTreeRequest,
    GitAgentBusyPolicy, GitFileState, RunWorkingTreeActionRequest, SessionStatus, WorkingTree,
    WorkingTreeAction, WorkingTreeActionReply, WorkingTreeFile, WorkingTreeFileDiff,
    WorkspaceTarget,
};
use zeron_rpc::{RpcError, methods};

use super::{PageTab, PullRequestsPage, filters, routed, top_chip};
use crate::changes::{self, FileDiff};
use crate::composer::{ComposerInput, ComposerInputEvent};
use crate::history::{GitHistory, GitHistoryEvent, GitHistoryTarget};
use crate::icons::{self, icon};
use crate::pickers::{CommitModelPicked, Pickers};
use crate::popover;
use crate::settings::CommitModelChoice;
use crate::settings::widgets::text_tooltip;
use crate::state::AppState;
use crate::theme::Theme;

const POLL_INTERVAL: Duration = Duration::from_secs(4);
const WATCH_RETRY: Duration = Duration::from_secs(2);
const PANEL_WIDTH: f32 = 360.0;
const SUBJECT_LIMIT: usize = 72;
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

pub enum GitTabEvent {
    OpenPullRequest(String),
    OpenSourceControlSettings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingTarget {
    File { staged: bool, path: String },
    All { staged: bool },
    Commit,
    Stash,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Confirm {
    Discard { path: String, revision: String },
    DropStash { oid: String },
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

enum DiffView {
    Loading,
    Ready {
        diff: WorkingTreeFileDiff,
        parsed: Option<FileDiff>,
    },
    Failed(SharedString),
}

struct Expanded {
    staged: bool,
    path: String,
    view: DiffView,
}

pub struct GitTab {
    state: Entity<AppState>,
    history: Entity<GitHistory>,
    space: Option<GitSpace>,
    checkout_path: Option<String>,
    tree: Option<WorkingTree>,
    error: Option<SharedString>,
    loading: bool,
    generation: u64,
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
    writing: bool,
    write_generation: u64,
    write_task: Option<Task<()>>,
    write_focus: FocusHandle,
    written: Option<GeneratedCommitMessage>,
    write_error: Option<SharedString>,
    model_override: Option<CommitModelChoice>,
    shown_default: Option<CommitModelChoice>,
    model_picker: Entity<Pickers>,
    _model_picks: Subscription,
    _settings_changes: Subscription,
    _message_edits: Subscription,
    _history_events: Subscription,
}

impl EventEmitter<GitTabEvent> for GitTab {}

fn workspace_target(space: &GitSpace, checkout_path: Option<&str>) -> WorkspaceTarget {
    WorkspaceTarget {
        chat_id: None,
        space_id: Some(space.space_id.clone()),
        checkout_path: checkout_path.map(str::to_owned),
    }
}

fn target_params(space: &GitSpace, checkout_path: Option<&str>) -> serde_json::Value {
    let request = GetWorkingTreeRequest {
        target: workspace_target(space, checkout_path),
    };
    routed(
        serde_json::to_value(request).unwrap_or_default(),
        space.device.as_deref(),
    )
}

fn busy_policy(cx: &App) -> GitAgentBusyPolicy {
    crate::settings::current(cx).git_agent_busy_policy.into()
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

pub(super) fn staged_files(tree: &WorkingTree) -> Vec<&WorkingTreeFile> {
    tree.files
        .iter()
        .filter(|file| {
            !matches!(
                file.index,
                GitFileState::Unchanged | GitFileState::Untracked | GitFileState::Unmerged
            )
        })
        .collect()
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
        let history_events =
            cx.subscribe(&history, |tab: &mut Self, history, event, cx| match event {
                GitHistoryEvent::OpenCommit(commit) => {
                    let sha = commit.sha.clone();
                    history.update(cx, |history, cx| history.set_selected_sha(Some(sha), cx));
                }
                GitHistoryEvent::OpenRefBadge(branch) => {
                    cx.emit(GitTabEvent::OpenPullRequest(branch.clone()));
                }
                GitHistoryEvent::FetchSucceeded => tab.load(cx),
            });
        let poll_task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;
                let Ok(()) = this.update(cx, |tab, cx| {
                    if tab.visible && tab.window_active && !tab.loading {
                        tab.load(cx);
                    }
                    if tab.watch_task.is_none() {
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
                let default = crate::settings::current(cx).commit_message_model;
                tab.model_override = (default.as_ref() != Some(&event.0)).then(|| event.0.clone());
                cx.notify();
            },
        );
        let settings_changes = cx.observe_global::<crate::settings::SettingsStore>(|tab, cx| {
            let default = crate::settings::current(cx).commit_message_model;
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
            loading: false,
            generation: 0,
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
            writing: false,
            write_generation: 0,
            write_task: None,
            write_focus: cx.focus_handle(),
            written: None,
            write_error: None,
            model_override: None,
            shown_default,
            model_picker,
            _model_picks: model_picks,
            _settings_changes: settings_changes,
            _message_edits: message_edits,
            _history_events: history_events,
        }
    }

    pub(super) fn tree(&self) -> Option<&WorkingTree> {
        self.tree.as_ref()
    }

    pub(super) fn space(&self) -> Option<&GitSpace> {
        self.space.as_ref()
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

    pub(super) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        let resumed = visible && !self.visible;
        self.visible = visible;
        if resumed && !self.loading {
            self.load(cx);
        }
    }

    pub(super) fn set_ref_badges(
        &mut self,
        badges: HashMap<String, SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.history
            .update(cx, |history, cx| history.set_ref_badges(badges, cx));
    }

    fn retarget(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.tree = None;
        self.error = None;
        self.loading = false;
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
        self.cancel_writing();
        self.written = None;
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
        self.load(cx);
        self.watch(cx);
        cx.notify();
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(space) = self.space.clone() else {
            return;
        };
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let params = target_params(&space, self.checkout_path.as_deref());
        let generation = self.generation;
        self.loading = true;
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::GET_WORKING_TREE, params)
                .await
                .map_err(|error| error.to_string())
                .and_then(|value| {
                    serde_json::from_value::<WorkingTree>(value).map_err(|error| error.to_string())
                });
            this.update(cx, |tab, cx| {
                if tab.generation != generation {
                    return;
                }
                tab.loading = false;
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
                            if this.update(cx, |tab, cx| tab.load(cx)).is_err() {
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

    fn source_control(&self, cx: &App) -> bool {
        let Some(space) = &self.space else {
            return false;
        };
        let state = self.state.read(cx);
        space
            .device
            .as_deref()
            .or(state.local_device_id.as_deref())
            .is_some_and(|device| {
                state.device_supports(device, zeron_proto::capabilities::SOURCE_CONTROL_V1)
            })
    }

    fn allows(&self, action: &WorkingTreeAction, policy: GitAgentBusyPolicy) -> bool {
        self.pending.is_none()
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
        if !self.allows(&action, policy) || !self.source_control(cx) {
            return;
        }
        let (Some(space), Some(tree)) = (self.space.clone(), self.tree.as_ref()) else {
            return;
        };
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let request = RunWorkingTreeActionRequest {
            target: workspace_target(&space, self.checkout_path.as_deref()),
            expected_revision: tree.revision.clone(),
            busy_policy: policy,
            action,
        };
        let params = routed(
            serde_json::to_value(request).unwrap_or_default(),
            space.device.as_deref(),
        );
        let generation = self.generation;
        let committing = target == PendingTarget::Commit;
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
                if tab.generation != generation {
                    return;
                }
                match result {
                    Ok(value) => {
                        if let Some(pending) = tab.pending.as_mut() {
                            pending.replied = true;
                        }
                        let reply: WorkingTreeActionReply =
                            serde_json::from_value(value).unwrap_or_default();
                        if !reply.conflicts.is_empty() {
                            tab.stash_note = Some(conflict_note(&reply.conflicts).into());
                        }
                        if committing {
                            tab.last_commit = reply.commit;
                            tab.message.update(cx, |input, cx| input.set_text("", cx));
                            tab.written = None;
                            tab.write_error = None;
                            tab.reset_model_override(cx);
                        }
                    }
                    Err(error) => {
                        tab.pending = None;
                        tab.action_error = Some(error.to_string().into());
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
            && !self.writing
            && !self.message.read(cx).text().trim().is_empty()
            && self.allows(&kind.action(String::new()), policy)
            && self.source_control(cx)
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
            None => false,
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
            && self.source_control(cx)
    }

    fn write_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.writing {
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
            .or(settings.commit_message_model.clone());
        let request = GenerateCommitMessageRequest {
            target: workspace_target(&space, self.checkout_path.as_deref()),
            instruction: Some(settings.effective_commit_instruction().to_owned()),
            harness: choice.as_ref().map(|choice| choice.harness),
            model: choice.map(|choice| choice.model),
        };
        let params = routed(
            serde_json::to_value(request).unwrap_or_default(),
            space.device.as_deref(),
        );
        self.write_generation = self.write_generation.wrapping_add(1);
        let generation = self.write_generation;
        self.writing = true;
        self.write_error = None;
        self.commit_menu = false;
        window.focus(&self.write_focus, cx);
        cx.notify();
        self.write_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = engine
                .client()
                .call(methods::GENERATE_COMMIT_MESSAGE, params)
                .await
                .map_err(|error| error.to_string())
                .and_then(|value| {
                    serde_json::from_value::<GeneratedCommitMessage>(value)
                        .map_err(|error| error.to_string())
                });
            this.update_in(cx, |tab, window, cx| {
                if tab.write_generation != generation {
                    return;
                }
                tab.writing = false;
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
        self.write_generation = self.write_generation.wrapping_add(1);
        self.writing = false;
    }

    fn stop_writing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.writing {
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
        self.expanded = Some(Expanded {
            staged,
            path,
            view: DiffView::Loading,
        });
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
        let params = routed(
            serde_json::to_value(request).unwrap_or_default(),
            space.device.as_deref(),
        );
        let generation = self.generation;
        self.diff_task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::GET_WORKING_TREE_FILE_DIFF, params)
                .await
                .map_err(|error| error.to_string())
                .and_then(|value| {
                    serde_json::from_value::<WorkingTreeFileDiff>(value)
                        .map_err(|error| error.to_string())
                });
            this.update(cx, |tab, cx| {
                if tab.generation != generation {
                    return;
                }
                let Some(open) = tab
                    .expanded
                    .as_mut()
                    .filter(|open| open.staged == staged && open.path == path)
                else {
                    return;
                };
                open.view = match result {
                    Ok(diff) => {
                        let parsed = changes::parse_patch(&diff.patch).into_iter().next();
                        DiffView::Ready { diff, parsed }
                    }
                    Err(error) => DiffView::Failed(error.into()),
                };
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
                        "Choose a project in the top bar to see its commit graph and working tree.",
                    ),
            )
            .into_any_element()
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
            let enabled = self.allows(&all, policy);
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
            if let Some(open) = self
                .expanded
                .as_ref()
                .filter(|open| open.staged == staged && open.path == file.path)
            {
                rows.push(render_diff(open, theme));
            }
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
        let color = match state {
            GitFileState::Added | GitFileState::Untracked => theme.success,
            GitFileState::Deleted | GitFileState::Unmerged => theme.danger,
            GitFileState::Renamed | GitFileState::Copied => theme.accent,
            _ => theme.warning,
        };
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
            let enabled = self.allows(&action, policy);
            let label = if staged { "Unstage" } else { "Stage" };
            let selector = format!("{section}-toggle-{index}");
            div()
                .id(SharedString::from(selector.clone()))
                .debug_selector(move || selector.clone())
                .role(gpui::Role::Button)
                .aria_label(SharedString::from(format!("{label} {}", file.path)))
                .tooltip(text_tooltip(label))
                .size(px(20.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .font_family(theme.font_mono.clone())
                .text_size(px(14.0))
                .text_color(theme.text_muted)
                .child(if staged { "−" } else { "+" })
                .map(|el| {
                    if enabled {
                        el.opacity(0.0)
                            .group_hover(group.clone(), |style| style.opacity(1.0))
                            .cursor_pointer()
                            .hover(|style| style.text_color(theme.text).bg(theme.ink(0.08)))
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
                        el.opacity(0.0)
                            .group_hover(group.clone(), |style| style.opacity(0.3))
                            .on_click(|_, _, cx| cx.stop_propagation())
                    }
                })
        });
        let discard = writes.filter(|_| !staged).map(|policy| {
            let path = file.path.clone();
            let enabled = file.worktree != GitFileState::Unmerged
                && !path.ends_with('/')
                && self.allows(
                    &WorkingTreeAction::Discard {
                        paths: vec![path.clone()],
                    },
                    policy,
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
            .children(toggle)
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
            .on_key_down(cx.listener(|tab, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    tab.cancel_confirm(cx);
                }
            }))
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

    fn render_panel(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let tree = self.tree.as_ref();
        let policy = busy_policy(cx);
        let writes = self.source_control(cx).then_some(policy);
        let checkout = self.space.as_ref().and_then(|space| {
            checkout_options(self.state.read(cx), space)
                .into_iter()
                .find(|option| option.path.as_deref() == self.checkout_path.as_deref())
        });
        let mut detail = Vec::new();
        if let Some(branch) = tree.and_then(|tree| tree.branch.clone()) {
            detail.push(branch);
        } else if tree.is_some() {
            detail.push("detached HEAD".into());
        }
        if let Some(option) = &checkout {
            detail.push(if option.path.is_none() {
                "main checkout".into()
            } else {
                "agent worktree".into()
            });
        }
        let staged = tree.map(staged_files).unwrap_or_default();
        let changes = tree.map(changed_files).unwrap_or_default();
        let body: AnyElement = match (tree, &self.error) {
            (None, Some(error)) => div()
                .p(px(12.0))
                .text_size(px(12.0))
                .text_color(theme.danger)
                .child(error.clone())
                .into_any_element(),
            (None, None) => div()
                .p(px(12.0))
                .text_size(px(12.0))
                .text_color(theme.text_muted)
                .child("Reading the working tree…")
                .into_any_element(),
            (Some(tree), _) => div()
                .flex()
                .flex_col()
                .pb(px(8.0))
                .when(tree.truncated, |el| {
                    el.child(
                        div()
                            .mx(px(12.0))
                            .mt(px(8.0))
                            .text_size(px(12.0))
                            .text_color(theme.warning)
                            .child("Partial snapshot: only the first files are listed."),
                    )
                })
                .when_some(self.stash_note.clone(), |el, note| {
                    el.child(
                        div()
                            .id("git-stash-note")
                            .debug_selector(|| "git-stash-note".into())
                            .mx(px(12.0))
                            .mt(px(8.0))
                            .text_size(px(12.0))
                            .text_color(theme.warning)
                            .child(note),
                    )
                })
                .children(
                    [self.error.clone(), self.action_error.clone()]
                        .into_iter()
                        .flatten()
                        .enumerate()
                        .map(|(index, error)| {
                            let selector = format!("git-error-{index}");
                            div()
                                .id(SharedString::from(selector.clone()))
                                .debug_selector(move || selector.clone())
                                .mx(px(12.0))
                                .mt(px(8.0))
                                .text_size(px(12.0))
                                .text_color(theme.danger)
                                .child(error)
                        }),
                )
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
                ))
                .into_any_element(),
        };
        div()
            .id("pull-requests-git-panel")
            .w(px(PANEL_WIDTH))
            .flex_none()
            .h_full()
            .border_l_1()
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
                    .children(
                        writes.zip(tree).map(|(policy, tree)| {
                            self.render_stash_button(tree, policy, theme, cx)
                        }),
                    ),
            )
            .when(tree.is_some_and(|tree| tree.agent_active), |el| {
                el.child(
                    div()
                        .id("git-agent-active")
                        .debug_selector(|| "git-agent-active".into())
                        .mx(px(12.0))
                        .mt(px(8.0))
                        .px(px(8.0))
                        .py(px(6.0))
                        .rounded(px(6.0))
                        .bg(theme.warning.opacity(0.1))
                        .text_size(px(12.0))
                        .text_color(theme.warning)
                        .child(agent_banner(policy)),
                )
            })
            .child(
                div()
                    .id("git-working-tree-files")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(body),
            )
            .child(self.render_commit_box(staged.len(), writes, theme, cx))
            .into_any_element()
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
            .child("Stash")
            .child(
                icon(icons::LUCIDE_CHEVRON_DOWN)
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
        let push_enabled = files > 0 && self.allows(&push, policy);
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
                        .on_key_down(cx.listener(|tab, event: &KeyDownEvent, _, cx| {
                            if event.keystroke.key == "escape" {
                                cx.stop_propagation();
                                tab.cancel_confirm(cx);
                            }
                        }))
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
            let pop_enabled = self.allows(&pop, policy);
            let drop_enabled = self.allows(&drop, policy);
            let when = stash
                .created_at
                .map(|at| super::relative_time(now, at))
                .unwrap_or_default();
            let row_button = |selector: String, label: &'static str, enabled: bool| {
                div()
                    .id(SharedString::from(selector.clone()))
                    .debug_selector(move || selector.clone())
                    .role(gpui::Role::Button)
                    .aria_label(label)
                    .flex_none()
                    .h(px(22.0))
                    .px(px(8.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(popup.hairline(0.12))
                    .flex()
                    .items_center()
                    .text_size(px(12.0))
                    .child(label)
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
        let writing = self.writing;
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
            div().child("✦").into_any_element()
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
            .gap(px(4.0))
            .child(self.model_picker.clone())
            .when(self.model_override.is_some(), |el| {
                el.child(
                    div()
                        .id("git-write-model-override")
                        .debug_selector(|| "git-write-model-override".into())
                        .flex_none()
                        .text_size(px(12.0))
                        .text_color(theme.text_muted)
                        .child("· this commit"),
                )
            });
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
            .on_click(cx.listener(|_, _, _, cx| cx.emit(GitTabEvent::OpenSourceControlSettings)));
        let current_text = self.message.read(cx).text().to_string();
        let message_body: AnyElement = if writing {
            div()
                .id("git-write-preview")
                .debug_selector(|| "git-write-preview".into())
                .min_h(px(64.0))
                .max_h(px(160.0))
                .px(px(10.0))
                .pt(px(8.0))
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
                .min_h(px(64.0))
                .max_h(px(160.0))
                .px(px(10.0))
                .pt(px(8.0))
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
        let fill = if main_enabled {
            theme.accent_strong
        } else {
            theme.accent_strong.opacity(0.35)
        };
        div()
            .id("git-commit-box")
            .flex_none()
            .p(px(12.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .border_t_1()
            .border_color(theme.hairline(0.07))
            .when(committing, |el| el.opacity(0.6))
            .child(
                div()
                    .id("git-commit-message")
                    .debug_selector(|| "git-commit-message".into())
                    .role(gpui::Role::TextInput)
                    .aria_label("Commit message")
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(theme.hairline(0.12))
                    .bg(theme.ink(0.02))
                    .flex()
                    .flex_col()
                    .when(writing, |el| el.track_focus(&self.write_focus))
                    .capture_action(cx.listener(|tab, _: &crate::composer::Submit, window, cx| {
                        cx.stop_propagation();
                        tab.message.update(cx, |input, cx| {
                            input.replace_text_in_range(None, "\n", window, cx)
                        });
                    }))
                    .on_key_down(cx.listener(|tab, event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key == "escape" && tab.writing {
                            cx.stop_propagation();
                            tab.stop_writing(window, cx);
                            return;
                        }
                        let modifiers = &event.keystroke.modifiers;
                        if event.keystroke.key == "enter"
                            && (modifiers.platform || modifiers.control)
                        {
                            cx.stop_propagation();
                            tab.commit(CommitKind::Staged, cx);
                        }
                    }))
                    .on_click(cx.listener(|tab, _, window, cx| {
                        if !tab.writing {
                            window.focus(&tab.message.focus_handle(cx), cx);
                        }
                    }))
                    .child(message_body)
                    .child(
                        div()
                            .h(px(30.0))
                            .px(px(6.0))
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
                                    .child(SharedString::from(format!(
                                        "{subject}/{SUBJECT_LIMIT}"
                                    ))),
                            ),
                    ),
            )
            .when_some(self.written.as_ref().filter(|_| !writing), |el, written| {
                el.child(
                    div()
                        .id("git-write-note")
                        .debug_selector(|| "git-write-note".into())
                        .text_size(px(11.0))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(format!(
                            "Written by {} · {} from your instruction and the staged diff. Edit it before you commit.",
                            crate::composer::harness_display_name(written.harness),
                            written.model
                        ))),
                )
            })
            .when_some(self.write_error.clone(), |el, error| {
                el.child(
                    div()
                        .id("git-write-error")
                        .debug_selector(|| "git-write-error".into())
                        .text_size(px(11.0))
                        .text_color(theme.danger)
                        .child(SharedString::from(format!(
                            "Could not write a message: {}. Pick another model or write it yourself.",
                            error.trim_end_matches('.')
                        ))),
                )
            })
            .when_some(self.last_commit.as_ref(), |el, commit| {
                let short: String = commit.sha.chars().take(7).collect();
                let text = match &commit.branch {
                    Some(branch) => format!("Committed {short} to {branch}"),
                    None => format!("Committed {short}"),
                };
                el.child(
                    div()
                        .id("git-last-commit")
                        .debug_selector(|| "git-last-commit".into())
                        .text_size(px(11.0))
                        .text_color(theme.text_muted)
                        .truncate()
                        .child(SharedString::from(text)),
                )
            })
            .child(
                div()
                    .h(px(32.0))
                    .rounded(px(8.0))
                    .bg(fill)
                    .flex()
                    .text_size(px(13.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(if main_enabled {
                        theme.on_accent
                    } else {
                        theme.on_accent.opacity(0.55)
                    })
                    .child(
                        div()
                            .id("git-commit-button")
                            .debug_selector(|| "git-commit-button".into())
                            .role(gpui::Role::Button)
                            .aria_label(SharedString::from(label.clone()))
                            .flex_1()
                            .h_full()
                            .rounded_l(px(8.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap(px(8.0))
                            .child(
                                icon(icons::LUCIDE_CHECK)
                                    .size(px(14.0))
                                    .text_color(theme.on_accent),
                            )
                            .child(SharedString::from(label))
                            .child(
                                div()
                                    .px(px(4.0))
                                    .rounded(px(4.0))
                                    .bg(theme.on_accent.opacity(0.12))
                                    .font_family(theme.font_mono.clone())
                                    .text_size(px(10.0))
                                    .child("⌘↵"),
                            )
                            .when(main_enabled, |el| {
                                el.cursor_pointer()
                                    .hover(|style| style.bg(theme.on_accent.opacity(0.08)))
                                    .on_click(cx.listener(|tab, _, _, cx| {
                                        tab.commit(CommitKind::Staged, cx)
                                    }))
                            }),
                    )
                    .child(
                        div()
                            .id("git-commit-more")
                            .debug_selector(|| "git-commit-more".into())
                            .role(gpui::Role::Button)
                            .aria_label("More commit options")
                            .relative()
                            .w(px(32.0))
                            .h_full()
                            .rounded_r(px(8.0))
                            .border_l_1()
                            .border_color(theme.on_accent.opacity(0.18))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                icon(icons::LUCIDE_CHEVRON_DOWN)
                                    .size(px(14.0))
                                    .text_color(theme.on_accent),
                            )
                            .when(writes.is_some() && !committing, |el| {
                                el.cursor_pointer()
                                    .hover(|style| style.bg(theme.on_accent.opacity(0.08)))
                                    .on_click(cx.listener(|tab, _, _, cx| {
                                        tab.commit_menu = !tab.commit_menu;
                                        cx.notify();
                                    }))
                            })
                            .children(menu),
                    ),
            )
            .into_any_element()
    }
}

fn small_button(
    selector: &'static str,
    label: &'static str,
    danger: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(selector)
        .debug_selector(move || selector.into())
        .role(gpui::Role::Button)
        .aria_label(label)
        .flex_none()
        .h(px(24.0))
        .px(px(9.0))
        .rounded(px(6.0))
        .flex()
        .items_center()
        .text_size(px(12.0))
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
        .child(label)
}

fn render_diff(open: &Expanded, theme: &Theme) -> AnyElement {
    let note = |text: SharedString, color: gpui::Hsla| {
        div()
            .px(px(8.0))
            .py(px(6.0))
            .text_size(px(11.0))
            .text_color(color)
            .child(text)
            .into_any_element()
    };
    let mut body: Vec<AnyElement> = Vec::new();
    match &open.view {
        DiffView::Loading => body.push(note("Loading diff…".into(), theme.text_muted)),
        DiffView::Failed(error) => body.push(note(error.clone(), theme.danger)),
        DiffView::Ready { diff, parsed } => match parsed.as_ref().filter(|_| !diff.binary) {
            Some(file) if !file.hunks.is_empty() => {
                let gutter = changes::gutter_width(file);
                let mut budget = DIFF_LINE_LIMIT;
                let mut hidden = 0usize;
                for hunk in &file.hunks {
                    if budget == 0 {
                        hidden += hunk.lines.len();
                        continue;
                    }
                    body.push(
                        div()
                            .px(px(8.0))
                            .bg(theme.ink(0.03))
                            .font_family(theme.font_mono.clone())
                            .text_size(px(11.0))
                            .text_color(theme.text_muted)
                            .truncate()
                            .child(SharedString::from(hunk.header.clone()))
                            .into_any_element(),
                    );
                    for line in &hunk.lines {
                        if budget == 0 {
                            hidden += 1;
                            continue;
                        }
                        budget -= 1;
                        body.push(changes::diff_line_row(
                            line,
                            &[],
                            theme,
                            gutter,
                            changes::DiffCodeWidth::Wrapped,
                            None,
                        ));
                    }
                }
                if hidden > 0 {
                    body.push(note(
                        format!("{hidden} more lines not shown").into(),
                        theme.text_muted,
                    ));
                }
                if diff.truncated {
                    body.push(note(
                        "The diff is too large to show in full.".into(),
                        theme.text_muted,
                    ));
                }
            }
            _ if diff.binary => {
                body.push(note("Binary file, no text diff.".into(), theme.text_muted))
            }
            _ => body.push(note("No text changes.".into(), theme.text_muted)),
        },
    }
    div()
        .id("git-file-diff")
        .debug_selector(|| "git-file-diff".into())
        .ml(px(32.0))
        .mr(px(12.0))
        .mb(px(6.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(theme.hairline(0.09))
        .overflow_hidden()
        .flex()
        .flex_col()
        .children(body)
        .into_any_element()
}

impl Render for GitTab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.activation.is_none() {
            self.window_active = window.is_window_active();
            self.activation = Some(cx.observe_window_activation(window, |tab, window, cx| {
                let resumed = window.is_window_active() && !tab.window_active;
                tab.window_active = window.is_window_active();
                if resumed && tab.visible && !tab.loading {
                    tab.load(cx);
                }
            }));
        }
        let theme = Theme::of(cx).clone();
        if self.space.is_none() {
            return div()
                .id("pull-requests-git")
                .role(gpui::Role::Group)
                .aria_label("Git")
                .size_full()
                .child(self.render_empty(&theme));
        }
        div()
            .id("pull-requests-git")
            .role(gpui::Role::Group)
            .aria_label("Git")
            .size_full()
            .flex()
            .flex_row()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(self.history.clone()),
            )
            .child(self.render_panel(&theme, cx))
    }
}

pub(super) fn pr_badges(
    items: &[ChangeRequestListItem],
    repository: &str,
) -> HashMap<String, (SharedString, usize)> {
    let mut badges = HashMap::new();
    for (index, item) in items.iter().enumerate() {
        if item.state != ChangeRequestState::Open
            || !item.repository.eq_ignore_ascii_case(repository)
            || item.head_ref == item.base_ref
        {
            continue;
        }
        badges
            .entry(item.head_ref.clone())
            .or_insert_with(|| (SharedString::from(format!("#{}", item.number)), index));
    }
    badges
}

impl PullRequestsPage {
    pub(super) fn git_tab(&mut self, cx: &mut Context<Self>) -> Entity<GitTab> {
        if let Some(view) = &self.git_view {
            return view.clone();
        }
        let state = self.state.clone();
        let view = cx.new(|cx| GitTab::new(state, cx));
        self.git_events = vec![
            cx.subscribe(&view, |page: &mut Self, _, event, cx| match event {
                GitTabEvent::OpenPullRequest(branch) => page.open_branch_pull_request(branch, cx),
                GitTabEvent::OpenSourceControlSettings => {
                    cx.emit(super::PullRequestsEvent::OpenSettings(
                        crate::shell::SettingsSection::SourceControl,
                    ))
                }
            }),
            cx.observe(&view, |_, _, cx| cx.notify()),
        ];
        self.git_view = Some(view.clone());
        view
    }

    fn git_project_repository(&self, space_id: &str) -> Option<String> {
        self.listing.ready().and_then(|listing| {
            listing
                .projects
                .iter()
                .find(|project| project.space_id == space_id)
                .map(|project| project.repository.clone())
        })
    }

    pub(super) fn git_space(&self, cx: &App) -> Option<GitSpace> {
        let state = self.state.read(cx);
        let usable = |id: &str| {
            state
                .spaces
                .iter()
                .find(|space| space.id == id && space.git_detected)
        };
        let space = self
            .prefs
            .git_space_id
            .as_deref()
            .and_then(usable)
            .or_else(|| {
                let wanted = self.prefs.repository.as_deref()?;
                let listing = self.listing.ready()?;
                listing
                    .projects
                    .iter()
                    .filter(|project| project.repository.eq_ignore_ascii_case(wanted))
                    .find_map(|project| usable(&project.space_id))
            })?;
        Some(GitSpace {
            space_id: space.id.clone(),
            path: space.path.clone(),
            device: (state.local_device_id.as_deref() != Some(space.device_id.as_str()))
                .then(|| space.device_id.clone()),
        })
    }

    pub(super) fn follow_repository_into_git(&mut self) {
        let Some(wanted) = self.prefs.repository.clone() else {
            return;
        };
        let Some(listing) = self.listing.ready() else {
            return;
        };
        let current = self.prefs.git_space_id.as_deref().and_then(|id| {
            listing
                .projects
                .iter()
                .find(|project| project.space_id == id)
        });
        if current.is_some_and(|project| project.repository.eq_ignore_ascii_case(&wanted)) {
            return;
        }
        if let Some(project) = listing
            .projects
            .iter()
            .find(|project| project.repository.eq_ignore_ascii_case(&wanted))
        {
            self.prefs.git_space_id = Some(project.space_id.clone());
        }
    }

    pub(super) fn sync_git_tab(&mut self, cx: &mut Context<Self>) -> Entity<GitTab> {
        let view = self.git_tab(cx);
        let space = self.git_space(cx);
        let badges = match (&space, self.listing.ready()) {
            (Some(space), Some(listing)) => self
                .git_project_repository(&space.space_id)
                .map(|repository| {
                    pr_badges(&listing.items, &repository)
                        .into_iter()
                        .map(|(branch, (label, _))| (branch, label))
                        .collect()
                })
                .unwrap_or_default(),
            _ => HashMap::new(),
        };
        let visible = self.visible;
        view.update(cx, |tab, cx| {
            tab.set_space(space, cx);
            tab.set_visible(visible, cx);
            tab.set_ref_badges(badges, cx);
        });
        view
    }

    pub(super) fn open_branch_pull_request(&mut self, branch: &str, cx: &mut Context<Self>) {
        let Some(space) = self.git_space(cx) else {
            return;
        };
        let Some(repository) = self.git_project_repository(&space.space_id) else {
            return;
        };
        let Some(listing) = self.listing.ready() else {
            return;
        };
        let Some(item) = pr_badges(&listing.items, &repository)
            .remove(branch)
            .and_then(|(_, index)| listing.items.get(index).cloned())
        else {
            return;
        };
        self.set_tab(PageTab::PullRequests, cx);
        self.open_pull_request(&item, cx);
    }

    fn git_project_options(&self, cx: &App) -> Vec<(String, SharedString, SharedString)> {
        let state = self.state.read(cx);
        let mut options: Vec<_> = state
            .spaces
            .iter()
            .filter(|space| space.git_detected)
            .map(|space| {
                let subtitle = self
                    .git_project_repository(&space.id)
                    .unwrap_or_else(|| space.path.clone());
                (
                    space.id.clone(),
                    SharedString::from(space.display_name().to_string()),
                    SharedString::from(subtitle),
                )
            })
            .collect();
        options.sort_by_key(|(_, name, _)| name.to_lowercase());
        options
    }

    pub(super) fn render_git_project_control(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let space = self.git_space(cx);
        let label: SharedString = space
            .as_ref()
            .and_then(|space| {
                self.state
                    .read(cx)
                    .spaces
                    .iter()
                    .find(|candidate| candidate.id == space.space_id)
                    .map(|candidate| SharedString::from(candidate.display_name().to_string()))
            })
            .unwrap_or_else(|| "Pick a project".into());
        self.menu_trigger(
            filters::MenuKind::GitProject,
            top_chip("pull-requests-git-project", "Project", theme)
                .debug_selector(|| "pull-requests-git-project".into())
                .max_w(px(240.0))
                .child(
                    icon(icons::LUCIDE_FOLDER)
                        .size(px(14.0))
                        .flex_none()
                        .text_color(if space.is_some() {
                            theme.text
                        } else {
                            theme.text_muted
                        }),
                )
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(label),
                )
                .child(
                    icon(icons::LUCIDE_CHEVRON_DOWN)
                        .size(px(12.0))
                        .flex_none()
                        .text_color(theme.text_muted.opacity(0.7)),
                ),
            theme,
            cx,
        )
        .into_any_element()
    }

    pub(super) fn render_git_project_menu(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current = self.git_space(cx).map(|space| space.space_id);
        let options = self.git_project_options(cx);
        let body = div()
            .min_w(px(240.0))
            .flex()
            .flex_col()
            .gap(px(popover::MENU_GAP));
        if options.is_empty() {
            return body
                .child(
                    div()
                        .px(px(8.0))
                        .py(px(6.0))
                        .text_color(theme.text_muted)
                        .child("No Git projects yet"),
                )
                .into_any_element();
        }
        body.children(
            options
                .into_iter()
                .enumerate()
                .map(|(index, (space_id, name, subtitle))| {
                    let selected = current.as_deref() == Some(space_id.as_str());
                    let selector = format!("pull-requests-git-project-{index}");
                    let repository = self.git_project_repository(&space_id);
                    menu_option(selector, name, subtitle, selected, false, theme).on_click(
                        cx.listener(move |page, _, _, cx| {
                            let space_id = space_id.clone();
                            let repository = repository.clone();
                            let refetch = repository.is_some()
                                && repository.as_deref() != page.prefs.repository.as_deref();
                            page.choose(refetch, cx, |prefs| {
                                prefs.git_space_id = Some(space_id);
                                if repository.is_some() {
                                    prefs.repository = repository;
                                }
                            });
                        }),
                    )
                }),
        )
        .into_any_element()
    }

    pub(super) fn render_git_checkout_control(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let view = self.git_view.as_ref()?.read(cx);
        let space = view.space()?;
        let option = checkout_options(self.state.read(cx), space)
            .into_iter()
            .find(|option| option.path.as_deref() == view.checkout_path())?;
        let tree = view.tree();
        let label: SharedString = tree
            .and_then(|tree| tree.branch.clone())
            .map(SharedString::from)
            .unwrap_or_else(|| option.label.clone());
        let ahead = tree.map(|tree| (tree.ahead, tree.behind));
        let agent = option.path.is_some();
        Some(
            self.menu_trigger(
                filters::MenuKind::GitCheckout,
                top_chip("pull-requests-git-checkout", "Checkout", theme)
                    .debug_selector(|| "pull-requests-git-checkout".into())
                    .max_w(px(280.0))
                    .child(
                        icon(icons::LUCIDE_GIT_BRANCH)
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
                        let mut parts = Vec::new();
                        if a > 0 {
                            parts.push(format!("↑{a}"));
                        }
                        if b > 0 {
                            parts.push(format!("↓{b}"));
                        }
                        el.child(
                            div()
                                .text_size(px(12.0))
                                .text_color(theme.text_muted)
                                .child(SharedString::from(parts.join(" "))),
                        )
                    })
                    .child(
                        icon(icons::LUCIDE_CHEVRON_DOWN)
                            .size(px(12.0))
                            .flex_none()
                            .text_color(theme.text_muted.opacity(0.7)),
                    ),
                theme,
                cx,
            )
            .into_any_element(),
        )
    }

    pub(super) fn render_git_checkout_menu(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(view) = self.git_view.clone() else {
            return div().into_any_element();
        };
        let (options, current) = {
            let tab = view.read(cx);
            let Some(space) = tab.space() else {
                return div().into_any_element();
            };
            (
                checkout_options(self.state.read(cx), space),
                tab.checkout_path().map(str::to_owned),
            )
        };
        let group = |label: &'static str| {
            div()
                .px(px(8.0))
                .pt(px(6.0))
                .pb(px(2.0))
                .text_size(px(11.0))
                .text_color(theme.text_muted)
                .child(label)
        };
        let worktrees = options.len().saturating_sub(1);
        let rows: Vec<AnyElement> = options
            .into_iter()
            .enumerate()
            .map(|(index, option)| {
                let selected = option.path == current;
                let selector = format!("pull-requests-git-checkout-{index}");
                let path = option.path.clone();
                let view = view.clone();
                menu_option(
                    selector,
                    option.label,
                    option.detail,
                    selected,
                    option.live,
                    theme,
                )
                .on_click(cx.listener(move |page, _, _, cx| {
                    let path = path.clone();
                    view.update(cx, |tab, cx| tab.set_checkout(path, cx));
                    page.close_menu(cx);
                }))
                .into_any_element()
            })
            .collect();
        let mut rows = rows.into_iter();
        div()
            .min_w(px(280.0))
            .max_w(px(420.0))
            .flex()
            .flex_col()
            .gap(px(popover::MENU_GAP))
            .child(group("Main checkout"))
            .children(rows.next())
            .child(group("Agent worktrees"))
            .when(worktrees == 0, |el| {
                el.child(
                    div()
                        .px(px(8.0))
                        .py(px(6.0))
                        .text_color(theme.text_muted)
                        .child("No agent worktrees in this project"),
                )
            })
            .children(rows)
            .into_any_element()
    }
}

fn menu_option(
    selector: String,
    label: SharedString,
    detail: SharedString,
    selected: bool,
    live: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let id = SharedString::from(selector.clone());
    popover::menu_row(theme, false, id.clone())
        .id(id)
        .debug_selector(move || selector.clone())
        .role(gpui::Role::MenuItemRadio)
        .aria_label(label.clone())
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use zeron_proto::{ChangeRequestListing, Chat, Session, Space};
    use zeron_rpc::{RpcReply, RpcService};

    use super::*;
    use crate::popover::Loadable;

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
    }

    const WRITTEN: &str =
        "Panel: keep the scroll position\n\nThe list reset its offset on every poll.";

    const PATCH: &str = "diff --git a/src/a b.rs b/src/a b.rs\n--- a/src/a b.rs\n+++ b/src/a b.rs\n@@ -1,2 +1,3 @@\n one\n-two\n+TWO\n+three\n";

    fn listing() -> ChangeRequestListing {
        let item = |number: u64, head: &str, state: &str, repository: &str| {
            serde_json::json!({
                "provider": "github", "host": "github.com", "repository": repository,
                "number": number, "title": format!("PR {number}"), "url": "u",
                "headRef": head, "baseRef": "main", "state": state, "isDraft": false,
                "mergeability": "unknown", "additions": 0, "deletions": 0,
                "createdAt": "2026-10-01T00:00:00Z", "updatedAt": "2026-10-01T00:00:00Z",
                "viewerReviewRequested": false, "labels": []
            })
        };
        serde_json::from_value(serde_json::json!({
            "viewers": {"github.com": "alice"},
            "items": [
                item(7, "feature/7", "open", "acme/zeron"),
                item(8, "feature/8", "merged", "acme/zeron"),
                item(9, "main", "open", "acme/zeron"),
                item(10, "feature/7", "open", "acme/other"),
            ],
            "errors": [],
            "truncated": false,
            "repositories": ["acme/zeron"],
            "projects": [{
                "repository": "acme/zeron", "host": "github.com",
                "spaceId": "space-1", "path": "/lab/repo"
            }]
        }))
        .unwrap()
    }

    fn tree() -> serde_json::Value {
        serde_json::json!({
            "checkoutId": "checkout-1", "root": "/lab/repo", "branch": "feature/7",
            "headSha": "aaa", "upstream": "origin/feature/7", "ahead": 2, "behind": 0,
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
                    RpcReply::value(&self.tree.lock().unwrap().clone().unwrap_or_else(tree))
                }
                methods::RUN_WORKING_TREE_ACTION => {
                    if let Some(error) = self.action_error.lock().unwrap().clone() {
                        return Err(RpcError::Failed(error));
                    }
                    let mut next = self.tree.lock().unwrap().clone().unwrap_or_else(tree);
                    next["revision"] = "r2".into();
                    let reply = if params["action"]["kind"] == "commit" {
                        next["headSha"] = "bbb".into();
                        serde_json::json!({"commit": {
                            "sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                            "branch": "feature/7", "subject": "Fix the panel",
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
                methods::LIST_CHANGE_REQUESTS => RpcReply::value(&listing()),
                methods::LIST_CHANGE_REQUEST_STATS => {
                    RpcReply::value(&serde_json::json!({"stats": []}))
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
                "capabilities": [zeron_proto::capabilities::SOURCE_CONTROL_V1],
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
    fn badges_cover_open_pull_requests_of_the_project_by_head_branch() {
        let listing = listing();
        let badges = pr_badges(&listing.items, "ACME/zeron");
        assert_eq!(badges.len(), 1);
        assert_eq!(badges["feature/7"].0.as_ref(), "#7");
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

    fn open_page(
        cx: &mut gpui::TestAppContext,
        state: AppState,
        prefs: filters::PullRequestListPrefs,
    ) -> (
        Entity<PullRequestsPage>,
        &mut gpui::VisualTestContext,
        Calls,
        tempfile::TempDir,
    ) {
        open_lab(cx, state, prefs, Arc::default())
    }

    fn open_lab(
        cx: &mut gpui::TestAppContext,
        state: AppState,
        prefs: filters::PullRequestListPrefs,
        lab: Arc<GitLab>,
    ) -> (
        Entity<PullRequestsPage>,
        &mut gpui::VisualTestContext,
        Calls,
        tempfile::TempDir,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let calls = lab.calls.clone();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            let mut settings = crate::settings::UiSettings::default();
            settings.pull_requests_list = prefs;
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
        let (page, cx) = cx.add_window_view(|_, cx| PullRequestsPage::new(state, cx));
        (page, cx, calls, dir)
    }

    #[gpui::test]
    fn the_git_tab_pins_history_and_reads_the_working_tree_of_the_picked_project(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let prefs = filters::PullRequestListPrefs {
            tab: PageTab::Git,
            ..Default::default()
        };
        let mut state = lab_state();
        state.selected_chat = Some("c-wt".into());
        let (page, cx, calls, _dir) = open_page(cx, state, prefs);
        settle(cx, |cx| {
            cx.debug_bounds("pull-requests-git-project").is_some()
        });
        page.update(cx, |page, cx| {
            assert!(page.git_space(cx).is_none(), "no project is guessed");
        });
        assert!(
            !calls
                .lock()
                .unwrap()
                .iter()
                .any(|(method, _)| method == methods::GET_WORKING_TREE
                    || method == methods::LIST_GIT_HISTORY),
            "nothing is read before a project is picked, not even the selected chat's history"
        );

        let chip = cx.debug_bounds("pull-requests-git-project").unwrap();
        cx.simulate_click(chip.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("pull-requests-git-project-0").is_some());
        let repo = cx
            .debug_bounds("pull-requests-git-project-1")
            .expect("the picker lists Git spaces by name: plain, repo");
        cx.simulate_click(repo.center(), gpui::Modifiers::default());
        settle(cx, |cx| cx.debug_bounds("git-staged-row-0").is_some());
        page.update(cx, |page, cx| {
            assert_eq!(page.prefs.git_space_id.as_deref(), Some("space-1"));
            assert_eq!(
                crate::settings::current(cx)
                    .pull_requests_list
                    .git_space_id
                    .as_deref(),
                Some("space-1")
            );
        });
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
    fn the_checkout_picker_retargets_the_graph_and_the_working_tree(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let prefs = filters::PullRequestListPrefs {
            tab: PageTab::Git,
            git_space_id: Some("space-1".into()),
            ..Default::default()
        };
        let (_page, cx, calls, _dir) = open_page(cx, lab_state(), prefs);
        settle(cx, |cx| cx.debug_bounds("git-staged-row-0").is_some());
        let chip = cx
            .debug_bounds("pull-requests-git-checkout")
            .expect("checkout chip in the top bar");
        cx.simulate_click(chip.center(), gpui::Modifiers::default());
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

    #[gpui::test]
    fn a_pull_request_badge_on_a_ref_opens_that_pull_request(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let prefs = filters::PullRequestListPrefs {
            tab: PageTab::Git,
            git_space_id: Some("space-1".into()),
            ..Default::default()
        };
        let (page, cx, _calls, _dir) = open_page(cx, lab_state(), prefs);
        page.update(cx, |page, cx| {
            page.listing = Loadable::Ready(listing());
            cx.notify();
        });
        settle(cx, |cx| cx.debug_bounds("history-pr-badge-0").is_some());
        let badge = cx.debug_bounds("history-pr-badge-0").unwrap();
        cx.simulate_click(badge.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        page.update(cx, |page, _| {
            assert_eq!(page.prefs.tab, PageTab::PullRequests);
            let active = page.active.clone().expect("a pull request is open");
            assert_eq!(
                (active.repository.as_str(), active.number),
                ("acme/zeron", 7)
            );
        });
        let git = page.read_with(cx, |page, _| page.git_view.clone().unwrap());
        git.read_with(cx, |tab, cx| {
            assert_eq!(
                tab.history.read(cx).selected_sha(),
                None,
                "the row click did not fire"
            );
        });
    }

    #[gpui::test]
    fn picking_a_repository_on_the_pull_requests_tab_moves_the_git_project(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (page, cx, _calls, _dir) =
            open_page(cx, lab_state(), filters::PullRequestListPrefs::default());
        page.update(cx, |page, cx| {
            page.listing = Loadable::Ready(listing());
            page.update_prefs(false, cx, |prefs| {
                prefs.repository = Some("acme/zeron".into())
            });
            assert_eq!(page.prefs.git_space_id.as_deref(), Some("space-1"));
            page.update_prefs(false, cx, |prefs| prefs.repository = None);
            assert_eq!(
                page.prefs.git_space_id.as_deref(),
                Some("space-1"),
                "All projects keeps the Git project"
            );
        });
    }

    fn git_prefs() -> filters::PullRequestListPrefs {
        filters::PullRequestListPrefs {
            tab: PageTab::Git,
            git_space_id: Some("space-1".into()),
            ..Default::default()
        }
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

    fn git_tab(
        page: &Entity<PullRequestsPage>,
        cx: &mut gpui::VisualTestContext,
    ) -> Entity<GitTab> {
        page.read_with(cx, |page, _| page.git_view.clone().unwrap())
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
                && tab.read_with(cx, |tab, _| tab.pending.is_none() && !tab.loading)
        });
    }

    #[gpui::test]
    fn stage_and_unstage_send_the_shown_revision_and_the_policy_then_reread(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        let (page, cx, calls, _dir) = open_lab(cx, lab_state(), git_prefs(), lab.clone());
        settle(cx, |cx| cx.debug_bounds("git-changes-toggle-1").is_some());
        let tab = git_tab(&page, cx);

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
            assert_eq!(tab.tree().unwrap().revision, "r2", "the tree is read again");
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
    fn clicking_a_file_expands_its_diff_and_clicking_again_folds_it(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (page, cx, calls, _dir) = open_page(cx, lab_state(), git_prefs());
        settle(cx, |cx| cx.debug_bounds("git-staged-row-0").is_some());
        let tab = git_tab(&page, cx);
        click(cx, "git-staged-row-0");
        settle(cx, |cx| {
            tab.read_with(cx, |tab, _| {
                matches!(
                    tab.expanded.as_ref().map(|open| &open.view),
                    Some(DiffView::Ready { .. })
                )
            }) && cx.debug_bounds("git-file-diff").is_some()
        });
        assert_eq!(
            calls_of(&calls, methods::GET_WORKING_TREE_FILE_DIFF),
            [serde_json::json!({"spaceId": "space-1", "path": "src/a b.rs", "staged": true})]
        );
        tab.read_with(cx, |tab, _| {
            let Some(DiffView::Ready {
                parsed: Some(file), ..
            }) = tab.expanded.as_ref().map(|open| &open.view)
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
        let (page, cx, calls, _dir) = open_lab(cx, lab_state(), git_prefs(), lab);
        cx.update(|_, cx| {
            crate::settings::update(crate::settings::SavePolicy::Immediate, cx, |settings| {
                settings.git_agent_busy_policy = crate::settings::GitAgentBusyPolicy::BlockAll;
            });
        });
        settle(cx, |cx| cx.debug_bounds("git-agent-active").is_some());
        let tab = git_tab(&page, cx);
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
            assert!(!tab.allows(&WorkingTreeAction::StageAll, busy_policy(cx)));
            assert!(tab.allows(&WorkingTreeAction::StageAll, GitAgentBusyPolicy::SafeOnly));
            assert!(tab.allows(&WorkingTreeAction::StageAll, GitAgentBusyPolicy::AllowAll));
            assert!(!tab.allows(
                &WorkingTreeAction::Discard { paths: vec![] },
                GitAgentBusyPolicy::SafeOnly
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
        let (page, cx, calls, _dir) = open_lab(cx, lab_state(), git_prefs(), lab);
        settle(cx, |cx| cx.debug_bounds("git-agent-active").is_some());
        let tab = git_tab(&page, cx);
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
        let (_page, cx, calls, _dir) = open_page(cx, state, git_prefs());
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
        let (page, cx, calls, _dir) = open_lab(cx, lab_state(), git_prefs(), lab);
        settle(cx, |cx| cx.debug_bounds("git-changes-toggle-0").is_some());
        let tab = git_tab(&page, cx);
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
        let (page, cx, calls, _dir) = open_lab(cx, lab_state(), git_prefs(), lab);
        cx.update(|_, cx| crate::composer::init(cx, Default::default()));
        settle(cx, |cx| cx.debug_bounds("git-commit-button").is_some());
        let tab = git_tab(&page, cx);
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
            assert_eq!(tab.tree().unwrap().head_sha.as_deref(), Some("bbb"));
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
        let (page, cx, calls, _dir) = open_lab(cx, lab_state(), git_prefs(), lab);
        settle(cx, |cx| cx.debug_bounds("git-agent-active").is_some());
        let tab = git_tab(&page, cx);

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
        let (page, cx, calls, _dir) = open_lab(cx, lab_state(), git_prefs(), lab);
        settle(cx, |cx| cx.debug_bounds("git-commit-button").is_some());
        let tab = git_tab(&page, cx);
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
        let (page, cx, calls, _dir) = open_lab(cx, lab_state(), git_prefs(), lab);
        cx.update(|_, cx| crate::composer::init(cx, Default::default()));
        settle(cx, |cx| cx.debug_bounds("git-write-ai").is_some());
        let tab = git_tab(&page, cx);
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
            assert!(!tab.writing);
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
        settle(cx, |cx| tab.read_with(cx, |tab, _| !tab.writing));
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
        let (page, cx, calls, _dir) = open_lab(cx, lab_state(), git_prefs(), lab.clone());
        cx.update(|_, cx| crate::composer::init(cx, Default::default()));
        settle(cx, |cx| cx.debug_bounds("git-write-ai").is_some());
        let tab = git_tab(&page, cx);
        settle(cx, |cx| tab.read_with(cx, |tab, _| tab.tree.is_some()));
        set_message(&tab, cx, "My own words");

        click(cx, "git-write-ai");
        settle(cx, |_| writes(&calls).len() == 1);
        cx.run_until_parked();
        assert!(tab.read_with(cx, |tab, _| tab.writing));
        assert!(cx.debug_bounds("git-write-preview").is_some());
        cx.simulate_keystrokes("cmd-enter");
        cx.run_until_parked();
        assert!(
            calls_of(&calls, methods::RUN_WORKING_TREE_ACTION).is_empty(),
            "no commit while a message is being written"
        );

        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert!(!tab.read_with(cx, |tab, _| tab.writing));
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
            assert!(tab.written.is_none() && tab.write_error.is_none() && !tab.writing);
        });
        assert!(calls_of(&calls, methods::RUN_WORKING_TREE_ACTION).is_empty());
    }

    #[gpui::test]
    fn a_failed_write_keeps_the_message_and_says_why(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        *lab.write_error.lock().unwrap() = Some("Claude usage limit reached.".into());
        let (page, cx, calls, _dir) = open_lab(cx, lab_state(), git_prefs(), lab);
        settle(cx, |cx| cx.debug_bounds("git-write-ai").is_some());
        let tab = git_tab(&page, cx);
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
        let (page, cx, calls, _dir) = open_lab(cx, lab_state(), git_prefs(), lab);
        settle(cx, |cx| cx.debug_bounds("git-write-ai").is_some());
        let tab = git_tab(&page, cx);
        settle(cx, |cx| tab.read_with(cx, |tab, _| tab.tree.is_some()));
        click(cx, "git-write-ai");
        cx.run_until_parked();
        assert!(writes(&calls).is_empty());

        let opened = std::rc::Rc::new(std::cell::Cell::new(0));
        cx.update(|_, cx| {
            let opened = opened.clone();
            cx.subscribe(&page, move |_, event, _| {
                if matches!(
                    event,
                    super::super::PullRequestsEvent::OpenSettings(
                        crate::shell::SettingsSection::SourceControl
                    )
                ) {
                    opened.set(opened.get() + 1);
                }
            })
            .detach();
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
        let (page, cx, calls, _dir) = open_lab(cx, lab_state(), git_prefs(), lab);
        cx.update(|_, cx| crate::composer::init(cx, Default::default()));
        settle(cx, |cx| cx.debug_bounds("picker-model").is_some());
        let tab = git_tab(&page, cx);
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
        assert!(cx.debug_bounds("git-write-model-override").is_some());
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
        let (page, cx, calls, _dir) = open_lab(cx, lab_state(), git_prefs(), lab);
        settle(cx, |cx| cx.debug_bounds("git-changes-discard-1").is_some());
        let tab = git_tab(&page, cx);
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
        cx.simulate_keystrokes("escape");
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
        let (page, cx, calls, _dir) = open_lab(cx, lab_state(), git_prefs(), lab.clone());
        settle(cx, |cx| cx.debug_bounds("git-changes-discard-0").is_some());
        let tab = git_tab(&page, cx);

        click(cx, "git-changes-discard-0");
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-discard-confirm").is_some());
        let mut changed = tree();
        changed["revision"] = "r-edited".into();
        *lab.tree.lock().unwrap() = Some(changed);
        tab.update(cx, |tab, cx| tab.load(cx));
        settle(cx, |cx| tab.read_with(cx, |tab, _| !tab.loading));
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
        let (page, cx, calls, _dir) = open_lab(cx, lab_state(), git_prefs(), lab.clone());
        settle(cx, |cx| cx.debug_bounds("git-stash-button").is_some());
        let tab = git_tab(&page, cx);
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
        cx.simulate_keystrokes("escape");
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
        let (page, cx, calls, _dir) = open_lab(cx, lab_state(), git_prefs(), lab);
        settle(cx, |cx| cx.debug_bounds("git-agent-active").is_some());
        let tab = git_tab(&page, cx);
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
}
