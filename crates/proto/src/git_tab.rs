//! The Git tab's wording and rules over a [`WorkingTree`]: file sections,
//! status letters, the agent banner, and when Pull and Push may run.
use crate::{
    GitAgentBusyPolicy, GitFileState, SyncBranchReply, SyncDirection, WorkingTree, WorkingTreeFile,
};

pub const SUBJECT_LIMIT: usize = 72;

/// The host's reply when a write names a revision or head that has moved.
pub const WORKING_TREE_CHANGED: &str =
    "The working tree changed since it was shown. Check the refreshed list and try again.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncKind {
    Push,
    Pull,
}

pub fn staged_files(tree: &WorkingTree) -> Vec<&WorkingTreeFile> {
    tree.files.iter().filter(|file| file.is_staged()).collect()
}

pub fn changed_files(tree: &WorkingTree) -> Vec<&WorkingTreeFile> {
    tree.files
        .iter()
        .filter(|file| file.worktree != GitFileState::Unchanged)
        .collect()
}

pub fn status_letter(state: GitFileState) -> &'static str {
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

pub fn agent_banner(policy: GitAgentBusyPolicy) -> &'static str {
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

pub fn commit_count(count: u32) -> String {
    if count == 1 {
        "1 commit".to_string()
    } else {
        format!("{count} commits")
    }
}

pub fn short_sha(sha: &str) -> String {
    sha.chars().take(7).collect()
}

pub fn subject_chars(message: &str) -> usize {
    message.lines().next().unwrap_or_default().chars().count()
}

pub fn clean_status(tree: &WorkingTree) -> String {
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

/// Where a push lands when it is not the branch's upstream, so the push
/// publishes the branch under its own name on the push remote.
pub fn publish_destination(tree: &WorkingTree) -> Option<String> {
    let (upstream, remote, branch) = (
        tree.upstream.as_deref()?,
        tree.push_remote.as_deref()?,
        tree.branch.as_deref()?,
    );
    let destination = format!("{remote}/{branch}");
    (upstream != destination).then_some(destination)
}

pub fn push_direction(tree: &WorkingTree) -> SyncDirection {
    SyncDirection::Push {
        set_upstream: tree.upstream.is_none() || publish_destination(tree).is_some(),
    }
}

/// The question the push confirmation asks.
pub fn push_prompt(tree: &WorkingTree) -> Option<String> {
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
        return Some(prompt);
    }
    match (&tree.upstream, &tree.push_remote) {
        (Some(upstream), _) => Some(format!("Push {commits} from {branch} to {upstream}?")),
        (None, Some(remote)) => Some(format!(
            "Push {commits} to {remote} as the new branch {branch}? It will track {remote}/{branch}."
        )),
        (None, None) => None,
    }
}

/// Why Pull or Push cannot run now; `None` when it can. `syncs_branches` is
/// whether the host supports syncing, `busy` whether another Git action is
/// still in flight.
pub fn sync_refusal(
    kind: SyncKind,
    tree: &WorkingTree,
    policy: GitAgentBusyPolicy,
    syncs_branches: bool,
    busy: bool,
) -> Option<String> {
    match kind {
        SyncKind::Push => {
            if !syncs_branches {
                return Some("Zeron on this device is too old to push. Update it first.".into());
            }
            if busy {
                return Some("Another Git action is still running.".into());
            }
            if tree.agent_active
                && !policy.allows_sync(SyncDirection::Push {
                    set_upstream: false,
                })
            {
                return Some(agent_banner(policy).into());
            }
            if tree.branch.is_none() {
                return Some("HEAD is detached. Check out a branch to push.".into());
            }
            if tree.head_sha.is_none() {
                return Some("This branch has no commits yet.".into());
            }
            if tree.push_remote.is_none() {
                return Some(
                    "There is no remote to push to. Add one named origin, or set remote.pushDefault."
                        .into(),
                );
            }
            match (&tree.upstream, &tree.branch) {
                (Some(upstream), Some(branch)) if tree.ahead == 0 => {
                    Some(format!("Nothing to push. {branch} matches {upstream}."))
                }
                _ => None,
            }
        }
        SyncKind::Pull => {
            if !syncs_branches {
                return Some("Zeron on this device is too old to pull. Update it first.".into());
            }
            if busy {
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

/// What a finished Pull or Push did, in words.
pub fn sync_note(
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
