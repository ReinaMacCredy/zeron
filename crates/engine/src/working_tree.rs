use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Weak};

use sha2::{Digest, Sha256};
use zeron_proto::{
    CommitSummary, GitFileState, LineDelta, StashEntry, SyncDirection, WorkingTree,
    WorkingTreeAction, WorkingTreeActionReply, WorkingTreeFile, WorkingTreeFileDiff,
};

use crate::EngineError;
use crate::diff_sync::Capture;

pub(crate) const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
const OUTPUT_MAX_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_FILES: usize = 2000;
const MAX_UNTRACKED_COUNTED: usize = 200;
const MAX_UNTRACKED_BYTES: u64 = 1024 * 1024;
const MAX_STASHES: usize = 50;

pub(crate) async fn git(root: &Path, args: &[&str]) -> Result<Capture, EngineError> {
    git_with(root, args, OUTPUT_MAX_BYTES, false).await
}

pub(crate) async fn git_with(
    root: &Path,
    args: &[&str],
    max_bytes: usize,
    exit_one_ok: bool,
) -> Result<Capture, EngineError> {
    run_git(root, args, max_bytes, exit_one_ok, true).await
}

async fn git_stash(root: &Path, args: &[&str]) -> Result<Capture, EngineError> {
    // `stash push -u` cleans with a `:/` pathspec; literal pathspecs leave untracked files behind.
    run_git(root, args, OUTPUT_MAX_BYTES, false, false).await
}

async fn run_git(
    root: &Path,
    args: &[&str],
    max_bytes: usize,
    exit_one_ok: bool,
    literal_pathspecs: bool,
) -> Result<Capture, EngineError> {
    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("--no-optional-locks")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env(
            "GIT_LITERAL_PATHSPECS",
            if literal_pathspecs { "1" } else { "0" },
        )
        .env("GIT_EDITOR", "true")
        .env("GIT_OPTIONAL_LOCKS", "0");
    crate::diff_sync::run_capped(cmd, max_bytes, exit_one_ok, |status| {
        format!("git {} exited {status}", args.first().unwrap_or(&"?"))
    })
    .await
}

#[derive(Debug, Default, PartialEq)]
struct Status {
    head_sha: Option<String>,
    branch: Option<String>,
    upstream: Option<String>,
    ahead: u32,
    behind: u32,
    files: Vec<WorkingTreeFile>,
    index_ids: BTreeMap<String, String>,
    complete: bool,
}

fn state(code: u8) -> Option<GitFileState> {
    Some(match code {
        b'.' | b' ' => GitFileState::Unchanged,
        b'A' => GitFileState::Added,
        b'M' => GitFileState::Modified,
        b'D' => GitFileState::Deleted,
        b'R' => GitFileState::Renamed,
        b'C' => GitFileState::Copied,
        b'U' => GitFileState::Unmerged,
        b'T' => GitFileState::TypeChanged,
        _ => return None,
    })
}

fn file(
    path: &str,
    old_path: Option<&str>,
    index: GitFileState,
    worktree: GitFileState,
) -> WorkingTreeFile {
    WorkingTreeFile {
        path: path.to_owned(),
        old_path: old_path.map(str::to_owned),
        index,
        worktree,
        staged: None,
        unstaged: None,
    }
}

fn parse_status(bytes: &[u8], truncated: bool) -> Status {
    let mut status = Status {
        complete: !truncated && (bytes.is_empty() || bytes.ends_with(&[0])),
        ..Status::default()
    };
    let mut records = bytes.split(|b| *b == 0).peekable();
    while let Some(record) = records.next() {
        if record.is_empty() {
            if records.peek().is_some() {
                status.complete = false;
            }
            continue;
        }
        let Ok(line) = std::str::from_utf8(record) else {
            status.complete = false;
            continue;
        };
        if let Some(header) = line.strip_prefix("# ") {
            let (key, value) = header.split_once(' ').unwrap_or((header, ""));
            match key {
                "branch.oid" if value != "(initial)" => status.head_sha = Some(value.into()),
                "branch.head" if value != "(detached)" => status.branch = Some(value.into()),
                "branch.upstream" => status.upstream = Some(value.into()),
                "branch.ab" => {
                    for part in value.split(' ') {
                        if let Some(n) = part.strip_prefix('+') {
                            status.ahead = n.parse().unwrap_or(0);
                        } else if let Some(n) = part.strip_prefix('-') {
                            status.behind = n.parse().unwrap_or(0);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        let parsed = match record[0] {
            b'1' => line.splitn(9, ' ').nth(8).and_then(|path| {
                let xy = line.as_bytes().get(2..4)?;
                Some(file(path, None, state(xy[0])?, state(xy[1])?))
            }),
            b'2' => {
                let old = records.next().and_then(|old| std::str::from_utf8(old).ok());
                line.splitn(10, ' ').nth(9).and_then(|path| {
                    let xy = line.as_bytes().get(2..4)?;
                    Some(file(path, Some(old?), state(xy[0])?, state(xy[1])?))
                })
            }
            b'u' => line
                .splitn(11, ' ')
                .nth(10)
                .map(|path| file(path, None, GitFileState::Unmerged, GitFileState::Unmerged)),
            b'?' => line
                .strip_prefix("? ")
                .map(|path| file(path, None, GitFileState::Untracked, GitFileState::Untracked)),
            b'!' => continue,
            _ => None,
        };
        if let (Some(file), b'1' | b'2') = (&parsed, record[0]) {
            let mut fields = line.split(' ');
            if let (Some(mode), Some(oid)) = (fields.nth(4), fields.nth(2)) {
                status
                    .index_ids
                    .insert(file.path.clone(), format!("{mode} {oid}"));
            }
        }
        match parsed {
            Some(file) if !file.path.is_empty() => status.files.push(file),
            _ => status.complete = false,
        }
    }
    status.files.sort_by(|a, b| a.path.cmp(&b.path));
    let mut merged: Vec<WorkingTreeFile> = Vec::with_capacity(status.files.len());
    for file in std::mem::take(&mut status.files) {
        match merged
            .last_mut()
            .filter(|previous| previous.path == file.path)
        {
            Some(previous) => {
                if !matches!(
                    file.index,
                    GitFileState::Unchanged | GitFileState::Untracked
                ) {
                    previous.index = file.index;
                }
                if file.worktree != GitFileState::Unchanged {
                    previous.worktree = file.worktree;
                }
                previous.old_path = previous.old_path.take().or(file.old_path);
            }
            None => merged.push(file),
        }
    }
    status.files = merged;
    status
}

fn parse_numstat(bytes: &[u8]) -> HashMap<String, LineDelta> {
    crate::diff_sync::numstat_records(bytes)
        .filter_map(|(added, deleted, path)| {
            let delta = LineDelta {
                additions: added.parse().unwrap_or(0),
                deletions: deleted.parse().unwrap_or(0),
                binary: added == "-" || deleted == "-",
            };
            Some((std::str::from_utf8(path).ok()?.to_owned(), delta))
        })
        .collect()
}

fn parse_stashes(bytes: &[u8]) -> Vec<StashEntry> {
    bytes
        .split(|b| *b == 0 || *b == b'\n')
        .filter_map(|record| {
            let record = std::str::from_utf8(record).ok()?;
            let mut fields = record.splitn(3, '\u{1f}');
            let oid = fields.next()?.trim();
            if oid.is_empty() {
                return None;
            }
            let created_at = fields
                .next()
                .and_then(|date| chrono::DateTime::parse_from_rfc3339(date).ok())
                .map(|date| date.with_timezone(&chrono::Utc));
            Some(StashEntry {
                oid: oid.to_owned(),
                message: fields.next().unwrap_or_default().to_owned(),
                created_at,
            })
        })
        .take(MAX_STASHES)
        .collect()
}

fn count_untracked(root: &Path, path: &str) -> Option<LineDelta> {
    let full: PathBuf = root.join(path);
    let meta = std::fs::symlink_metadata(&full).ok()?;
    if !meta.is_file() || meta.len() > MAX_UNTRACKED_BYTES {
        return None;
    }
    let bytes = std::fs::read(&full).ok()?;
    if bytes.contains(&0) {
        return Some(LineDelta {
            binary: true,
            ..LineDelta::default()
        });
    }
    let mut lines = bytes.iter().filter(|b| **b == b'\n').count();
    if bytes.last().is_some_and(|last| *last != b'\n') {
        lines += 1;
    }
    Some(LineDelta {
        additions: u32::try_from(lines).unwrap_or(u32::MAX),
        deletions: 0,
        binary: false,
    })
}

fn stamp(root: &Path, path: &str) -> Option<(u64, u128)> {
    let meta = std::fs::symlink_metadata(root.join(path)).ok()?;
    let modified = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some((meta.len(), modified))
}

fn revision(status: &Status, stamps: &[Option<(u64, u128)>]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(
        serde_json::to_vec(&(
            &status.head_sha,
            &status.branch,
            &status.files,
            stamps,
            &status.index_ids,
        ))
        .expect("working tree serialization"),
    );
    crate::repos::hex(&hasher.finalize())
}

pub(crate) async fn read(root: &Path, checkout_id: &str) -> Result<WorkingTree, EngineError> {
    let status = [
        "status",
        "--porcelain=v2",
        "-z",
        "--branch",
        "--untracked-files=all",
    ];
    let numstat = ["diff", "--numstat", "-z", "--no-ext-diff", "--no-color"];
    let cached = [
        "diff",
        "--cached",
        "--numstat",
        "-z",
        "--no-ext-diff",
        "--no-color",
    ];
    let stash = ["stash", "list", "-z", "--format=%H%x1f%cI%x1f%gs"];
    let (status_output, unstaged, staged, stashes, picking, config) = tokio::join!(
        git(root, &status),
        git(root, &numstat),
        git(root, &cached),
        git(root, &stash),
        verify_rev(root, "CHERRY_PICK_HEAD"),
        read_branch_config(root)
    );
    let status_output = status_output?;
    let mut status = parse_status(&status_output.stdout, status_output.truncated);
    let mut truncated = !status.complete;
    let unstaged = unstaged
        .map(|out| parse_numstat(&out.stdout))
        .unwrap_or_default();
    let staged = staged
        .map(|out| parse_numstat(&out.stdout))
        .unwrap_or_default();
    if status.files.len() > MAX_FILES {
        status.files.truncate(MAX_FILES);
        truncated = true;
    }
    let mut untracked = Vec::new();
    for (index, file) in status.files.iter_mut().enumerate() {
        if file.index != GitFileState::Unchanged && file.index != GitFileState::Untracked {
            file.staged = staged.get(&file.path).copied();
        }
        if file.worktree == GitFileState::Untracked {
            if untracked.len() < MAX_UNTRACKED_COUNTED {
                untracked.push((index, file.path.clone()));
            }
        } else if file.worktree != GitFileState::Unchanged {
            file.unstaged = unstaged.get(&file.path).copied();
        }
    }
    let changed: Vec<String> = status
        .files
        .iter()
        .filter(|file| file.worktree != GitFileState::Unchanged)
        .map(|file| file.path.clone())
        .collect();
    let config = match (status.branch.as_deref(), config) {
        (Some(branch), Ok(out)) => BranchConfig::parse(&out.stdout, branch),
        _ => BranchConfig::default(),
    };
    let push_remote = config.push_remote(status.upstream.is_some());
    let counted_root = root.to_path_buf();
    let measured = tokio::task::spawn_blocking(move || {
        let counts = untracked
            .into_iter()
            .map(|(index, path)| (index, count_untracked(&counted_root, &path)))
            .collect::<Vec<_>>();
        let stamps = changed
            .iter()
            .map(|path| stamp(&counted_root, path))
            .collect::<Vec<_>>();
        (counts, stamps)
    });
    let unpushed = async {
        let (None, Some(remote), Some(_)) = (&status.upstream, &push_remote, &status.head_sha)
        else {
            return status.ahead;
        };
        let not_on_remote = format!("--remotes={remote}");
        git(
            root,
            &["rev-list", "--count", "HEAD", "--not", &not_on_remote],
        )
        .await
        .ok()
        .and_then(|out| String::from_utf8_lossy(&out.stdout).trim().parse().ok())
        .unwrap_or(0)
    };
    let remote_default = async {
        match &push_remote {
            Some(remote) => default_branch(root, remote).await,
            None => None,
        }
    };
    let (measured, ahead, default_branch) = tokio::join!(measured, unpushed, remote_default);
    let (counts, stamps) = measured.unwrap_or_default();
    for (index, delta) in counts {
        status.files[index].unstaged = delta;
    }
    Ok(WorkingTree {
        checkout_id: checkout_id.to_owned(),
        root: root.to_string_lossy().into_owned(),
        revision: revision(&status, &stamps),
        branch: status.branch,
        head_sha: status.head_sha,
        upstream: status.upstream,
        ahead,
        behind: status.behind,
        push_remote,
        default_branch,
        truncated,
        agent_active: false,
        cherry_pick_head: picking.ok().flatten(),
        files: status.files,
        stashes: stashes
            .map(|out| parse_stashes(&out.stdout))
            .unwrap_or_default(),
    })
}

#[derive(Debug, Default, PartialEq)]
struct BranchConfig {
    remotes: Vec<String>,
    push_default: Option<String>,
    push_mode: Option<String>,
    remote: Option<String>,
    branch_push_remote: Option<String>,
    merge: Option<String>,
}

impl BranchConfig {
    fn parse(bytes: &[u8], branch: &str) -> Self {
        let mut config = Self::default();
        let branch_key = |key: &str, field: &str| {
            key.strip_prefix("branch.")
                .and_then(|rest| rest.strip_suffix(field))
                .is_some_and(|name| name == branch)
        };
        for entry in bytes.split(|byte| *byte == 0) {
            let entry = String::from_utf8_lossy(entry);
            let Some((key, value)) = entry.split_once('\n') else {
                continue;
            };
            let value = value.to_owned();
            if let Some(name) = key
                .strip_prefix("remote.")
                .and_then(|rest| rest.strip_suffix(".url"))
            {
                if !config.remotes.iter().any(|known| known == name) {
                    config.remotes.push(name.to_owned());
                }
            } else if key == "remote.pushdefault" {
                config.push_default = Some(value);
            } else if key == "push.default" {
                config.push_mode = Some(value);
            } else if branch_key(key, ".remote") {
                config.remote = Some(value);
            } else if branch_key(key, ".pushremote") {
                config.branch_push_remote = Some(value);
            } else if branch_key(key, ".merge") {
                config.merge = Some(value);
            }
        }
        config
    }

    fn known(&self, remote: &str) -> bool {
        self.remotes.iter().any(|known| known == remote)
    }

    fn push_remote(&self, tracking: bool) -> Option<String> {
        if let Some(remote) = self
            .branch_push_remote
            .as_ref()
            .or(self.push_default.as_ref())
        {
            return self.known(remote).then(|| remote.clone());
        }
        if tracking {
            return self.remote.clone().filter(|remote| self.known(remote));
        }
        self.known("origin")
            .then(|| "origin".to_owned())
            .or_else(|| (self.remotes.len() == 1).then(|| self.remotes[0].clone()))
    }
}

async fn read_branch_config(root: &Path) -> Result<Capture, EngineError> {
    git_with(
        root,
        &[
            "config",
            "-z",
            "--get-regexp",
            r"^(remote\..*\.url|remote\.pushdefault|push\.default|branch\..*\.(remote|pushremote|merge))$",
        ],
        OUTPUT_MAX_BYTES,
        true,
    )
    .await
}

async fn branch_config(root: &Path, branch: &str) -> BranchConfig {
    read_branch_config(root)
        .await
        .map(|out| BranchConfig::parse(&out.stdout, branch))
        .unwrap_or_default()
}

async fn default_branch(root: &Path, remote: &str) -> Option<String> {
    let head = format!("refs/remotes/{remote}/HEAD");
    let out = git(root, &["symbolic-ref", "-q", &head]).await.ok()?;
    let target = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    target
        .strip_prefix(&format!("refs/remotes/{remote}/"))
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

pub(crate) const WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(150);
pub(crate) const COMMIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
const MAX_MESSAGE_BYTES: usize = 64 * 1024;
const PATCH_MAX_BYTES: usize = 1024 * 1024;
const MAX_PATH_BYTES: usize = 4096;

type CheckoutLocks = HashMap<String, Weak<tokio::sync::Mutex<()>>>;
static CHECKOUT_LOCKS: LazyLock<std::sync::Mutex<CheckoutLocks>> = LazyLock::new(Default::default);

pub(crate) async fn lock_checkout(checkout_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
    let lock = {
        let mut locks = CHECKOUT_LOCKS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        locks.retain(|_, slot| slot.strong_count() > 0);
        match locks.get(checkout_id).and_then(Weak::upgrade) {
            Some(lock) => lock,
            None => {
                let lock = Arc::new(tokio::sync::Mutex::new(()));
                locks.insert(checkout_id.to_owned(), Arc::downgrade(&lock));
                lock
            }
        }
    };
    lock.lock_owned().await
}

const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

pub(crate) async fn lock_checkout_briefly(
    checkout_id: &str,
) -> Result<tokio::sync::OwnedMutexGuard<()>, EngineError> {
    tokio::time::timeout(LOCK_WAIT, lock_checkout(checkout_id))
        .await
        .map_err(|_| {
            refused("Another Git action is running in this checkout. Try again when it finishes.")
        })
}

pub(crate) fn is_staged(file: &WorkingTreeFile) -> bool {
    !matches!(
        file.index,
        GitFileState::Unchanged | GitFileState::Untracked | GitFileState::Unmerged
    )
}

fn checked_path(path: &str) -> Result<(), EngineError> {
    let invalid = path.is_empty()
        || path.len() > MAX_PATH_BYTES
        || path.contains('\0')
        || path.starts_with('/')
        || path.split(['/', '\\']).any(|part| {
            part.is_empty() || part == "." || part == ".." || part.eq_ignore_ascii_case(".git")
        });
    if invalid {
        return Err(EngineError::Other(format!(
            "not a working tree path: {path:?}"
        )));
    }
    Ok(())
}

fn listed<'a>(
    tree: &'a WorkingTree,
    paths: &[String],
    wanted: impl Fn(&WorkingTreeFile) -> bool,
    missing: &str,
) -> Result<Vec<&'a WorkingTreeFile>, EngineError> {
    if paths.is_empty() {
        return Err(EngineError::Other("no files were named".into()));
    }
    if paths.len() > MAX_FILES {
        return Err(EngineError::Other("too many files in one action".into()));
    }
    let mut files: Vec<&WorkingTreeFile> = Vec::with_capacity(paths.len());
    for path in paths {
        checked_path(path)?;
        let file = tree
            .files
            .iter()
            .find(|file| &file.path == path)
            .filter(|file| wanted(file))
            .ok_or_else(|| EngineError::Other(format!("{path} {missing}")))?;
        if !files.iter().any(|known| known.path == file.path) {
            files.push(file);
        }
    }
    Ok(files)
}

async fn run_with_paths(root: &Path, command: &[&str], paths: &[&str]) -> Result<(), EngineError> {
    let mut args: Vec<&str> = command.to_vec();
    args.push("--");
    args.extend_from_slice(paths);
    git(root, &args).await.map(drop)
}

pub(crate) async fn apply(
    root: &Path,
    tree: &WorkingTree,
    action: &WorkingTreeAction,
) -> Result<WorkingTreeActionReply, EngineError> {
    let done = match action {
        WorkingTreeAction::Stage { paths } => {
            let files = listed(
                tree,
                paths,
                |file| file.worktree != GitFileState::Unchanged,
                "has no unstaged changes",
            )?;
            let paths: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();
            run_with_paths(root, &["add", "-A"], &paths).await
        }
        WorkingTreeAction::Unstage { paths } => {
            let files = listed(tree, paths, is_staged, "is not staged")?;
            let mut paths: Vec<&str> = Vec::with_capacity(files.len());
            for file in &files {
                paths.push(&file.path);
                if let Some(old) = file.old_path.as_deref() {
                    checked_path(old)?;
                    paths.push(old);
                }
            }
            run_with_paths(root, &["reset", "-q"], &paths).await
        }
        WorkingTreeAction::StageAll => git(root, &["add", "-A"]).await.map(drop),
        WorkingTreeAction::UnstageAll => run_with_paths(root, &["reset", "-q"], &["."]).await,
        WorkingTreeAction::Commit {
            message,
            amend,
            all,
        } => return commit(root, tree, message, *amend, *all, COMMIT_TIMEOUT).await,
        WorkingTreeAction::Discard { paths } => discard(root, tree, paths).await,
        WorkingTreeAction::StashPush { message } => {
            stash_push(root, tree, message.as_deref()).await
        }
        WorkingTreeAction::StashPop { oid } => return stash_pop(root, tree, oid).await,
        WorkingTreeAction::StashDrop { oid } => {
            let selector = stash_selector(root, oid).await?;
            git_stash(root, &["stash", "drop", "--quiet", &selector])
                .await
                .map(drop)
        }
        WorkingTreeAction::Checkout { target_ref, detach } => {
            checkout(root, tree, target_ref, *detach).await
        }
        WorkingTreeAction::CherryPick { sha } => return cherry_pick(root, tree, sha).await,
        WorkingTreeAction::AbortCherryPick => {
            if tree.cherry_pick_head.is_none() {
                return Err(refused("There is no cherry-pick to abort."));
            }
            git(root, &["cherry-pick", "--abort"]).await.map(drop)
        }
        WorkingTreeAction::CreateBranch {
            name,
            start_point,
            checkout,
        } => create_branch(root, tree, name, start_point.as_deref(), *checkout).await,
    };
    done.map(|()| WorkingTreeActionReply::default())
}

fn refused(message: &str) -> EngineError {
    EngineError::Other(message.into())
}

pub(crate) fn refuse_outside_folder(
    action: &WorkingTreeAction,
    folder: &str,
) -> Result<(), EngineError> {
    let inside = |path: &str| {
        path == folder
            || path
                .strip_prefix(folder)
                .is_some_and(|rest| rest.starts_with('/'))
    };
    let whole = match action {
        WorkingTreeAction::StashPush { .. } => "Stash all",
        WorkingTreeAction::Commit { all: true, .. } => "Commit all",
        WorkingTreeAction::Discard { paths } => {
            return match paths.iter().find(|path| !inside(path)) {
                Some(path) => Err(refused(&format!(
                    "{path} is outside this project's folder, {folder}/. Discard it from a terminal."
                ))),
                None => Ok(()),
            };
        }
        _ => return Ok(()),
    };
    Err(refused(&format!(
        "This project is the folder {folder}/ inside a larger repository, so {whole} would reach files outside it. Run it from a terminal."
    )))
}

fn is_directory(root: &Path, path: &str) -> bool {
    let path = path.trim_end_matches('/');
    checked_path(path).is_ok()
        && std::fs::symlink_metadata(root.join(path)).is_ok_and(|meta| meta.is_dir())
}

async fn discard(root: &Path, tree: &WorkingTree, paths: &[String]) -> Result<(), EngineError> {
    if let Some(path) = paths.iter().find(|path| is_directory(root, path)) {
        return Err(EngineError::Other(format!(
            "{} is a directory (a submodule or a nested repository). Discard it in a terminal.",
            path.trim_end_matches('/')
        )));
    }
    let files = listed(
        tree,
        paths,
        |file| file.worktree != GitFileState::Unchanged,
        "has no unstaged changes",
    )?;
    if let Some(file) = files
        .iter()
        .find(|file| file.worktree == GitFileState::Unmerged)
    {
        return Err(EngineError::Other(format!(
            "{} has a merge conflict. Resolve it before discarding.",
            file.path
        )));
    }
    let (untracked, tracked): (Vec<&WorkingTreeFile>, Vec<&WorkingTreeFile>) = files
        .into_iter()
        .partition(|file| file.worktree == GitFileState::Untracked);
    if !tracked.is_empty() {
        let paths: Vec<&str> = tracked.iter().map(|file| file.path.as_str()).collect();
        run_with_paths(root, &["restore", "--worktree"], &paths).await?;
    }
    if !untracked.is_empty() {
        let paths: Vec<&str> = untracked.iter().map(|file| file.path.as_str()).collect();
        run_with_paths(root, &["clean", "-f", "-q"], &paths).await?;
        for path in paths {
            crate::diff_sync::remove_empty_ancestors(root, &root.join(path));
        }
    }
    Ok(())
}

async fn stash_push(
    root: &Path,
    tree: &WorkingTree,
    message: Option<&str>,
) -> Result<(), EngineError> {
    if tree.files.is_empty() {
        return Err(refused("There is nothing to stash."));
    }
    if tree.head_sha.is_none() {
        return Err(refused("Stash needs a first commit on this branch."));
    }
    if has_conflicts(tree) {
        return Err(refused("Resolve the merge conflicts before stashing."));
    }
    if let Some(file) = tree
        .files
        .iter()
        .find(|file| file.worktree == GitFileState::Untracked && is_directory(root, &file.path))
    {
        return Err(EngineError::Other(format!(
            "{} is a nested repository, which a stash would leave behind. Stash in a terminal.",
            file.path.trim_end_matches('/')
        )));
    }
    let message = message.map(str::trim).filter(|message| !message.is_empty());
    if message.is_some_and(|message| message.len() > MAX_MESSAGE_BYTES) {
        return Err(refused("The stash message is too long."));
    }
    let message = message.map(|message| format!("--message={message}"));
    let mut args = vec!["stash", "push", "--include-untracked", "--quiet"];
    if let Some(message) = message.as_deref() {
        args.push(message);
    }
    git_stash(root, &args).await.map(drop)
}

async fn stash_selector(root: &Path, oid: &str) -> Result<String, EngineError> {
    if oid.is_empty() || oid.len() > 64 || !oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(EngineError::Other(format!("not a stash id: {oid:?}")));
    }
    let listed = git(root, &["stash", "list", "-z", "--format=%H"]).await?;
    let index = listed
        .stdout
        .split(|byte| *byte == 0 || *byte == b'\n')
        .filter(|record| !record.is_empty())
        .position(|record| record.eq_ignore_ascii_case(oid.as_bytes()))
        .ok_or_else(|| {
            refused("That stash is no longer in the list. It may have been popped or dropped elsewhere.")
        })?;
    let selector = format!("stash@{{{index}}}");
    let resolved = git(root, &["rev-parse", "--verify", "--quiet", &selector]).await?;
    if !String::from_utf8_lossy(&resolved.stdout)
        .trim()
        .eq_ignore_ascii_case(oid)
    {
        return Err(refused(
            "The stash list changed while it was read. Try again.",
        ));
    }
    Ok(selector)
}

async fn stash_pop(
    root: &Path,
    tree: &WorkingTree,
    oid: &str,
) -> Result<WorkingTreeActionReply, EngineError> {
    if has_conflicts(tree) {
        return Err(refused(
            "Resolve the merge conflicts before popping a stash.",
        ));
    }
    let selector = stash_selector(root, oid).await?;
    let Err(error) = git_stash(root, &["stash", "pop", "--quiet", &selector]).await else {
        return Ok(WorkingTreeActionReply::default());
    };
    let conflicts = unmerged_paths(root).await?;
    if conflicts.is_empty() {
        return Err(error);
    }
    Ok(WorkingTreeActionReply {
        commit: None,
        conflicts,
    })
}

fn commit_error(error: EngineError) -> EngineError {
    let text = error.to_string().to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|needle| text.contains(needle));
    if has(&[
        "please tell me who you are",
        "identity unknown",
        "empty ident name",
        "unable to auto-detect email address",
    ]) {
        refused(
            "Git does not know who you are. Set your name and email with `git config --global user.name \"Your Name\"` and `git config --global user.email you@example.com`, then commit again.",
        )
    } else if has(&["failed to sign", "couldn't sign", "signing failed"]) {
        refused(
            "Git could not sign the commit. Signing here cannot show a passphrase prompt; unlock your signing key in a terminal (or commit there once), then try again.",
        )
    } else {
        error
    }
}

async fn commit(
    root: &Path,
    tree: &WorkingTree,
    message: &str,
    amend: bool,
    all: bool,
    timeout: std::time::Duration,
) -> Result<WorkingTreeActionReply, EngineError> {
    if message.trim().is_empty() {
        return Err(refused("Write a commit message first."));
    }
    if message.len() > MAX_MESSAGE_BYTES {
        return Err(refused("The commit message is too long."));
    }
    if amend {
        let Some(head) = tree.head_sha.as_deref() else {
            return Err(refused("There is no commit to amend yet."));
        };
        let output = git(
            root,
            &[
                "for-each-ref",
                "--contains",
                head,
                "--format=%(refname)",
                "refs/remotes/",
            ],
        )
        .await?;
        let listed = String::from_utf8_lossy(&output.stdout).into_owned();
        let refs: Vec<&str> = listed.lines().filter(|line| !line.is_empty()).collect();
        if let Some(remote) = refs
            .iter()
            .find(|name| !name.ends_with("/HEAD"))
            .or(refs.first())
        {
            let remote = remote.strip_prefix("refs/remotes/").unwrap_or(remote);
            return Err(EngineError::Other(format!(
                "The last commit is already on {remote}. Amending it would rewrite pushed history."
            )));
        }
    } else if all {
        if tree.files.is_empty() {
            return Err(refused("There is nothing to commit."));
        }
    } else if !tree.files.iter().any(is_staged) {
        return Err(refused(
            "Nothing is staged. Stage files first, or use Commit all.",
        ));
    }
    if all {
        git(root, &["add", "-A"]).await?;
    }
    let mut file = tempfile::NamedTempFile::new()?;
    std::io::Write::write_all(&mut file, message.as_bytes())?;
    let path = file
        .path()
        .to_str()
        .ok_or_else(|| refused("the temporary directory path is not UTF-8"))?;
    let mut args = vec!["commit", "--quiet", "--cleanup=whitespace", "--file", path];
    if amend {
        args.push("--amend");
    }
    let timed_out = format!(
        "The commit did not finish within {} s. A commit hook or a signing prompt may be waiting for input; commit in a terminal instead.",
        timeout.as_secs()
    );
    git_within(root, &args, timeout, timed_out)
        .await
        .map_err(commit_error)?;
    drop(file);
    new_commit_reply(root, tree).await
}

async fn new_commit_reply(
    root: &Path,
    tree: &WorkingTree,
) -> Result<WorkingTreeActionReply, EngineError> {
    let log = git(root, &["log", "-1", "--format=%H%x1f%s"]).await?;
    let log = String::from_utf8_lossy(&log.stdout);
    let (sha, subject) = log
        .trim_end_matches('\n')
        .split_once('\u{1f}')
        .ok_or_else(|| refused("could not read the new commit"))?;
    Ok(WorkingTreeActionReply {
        commit: Some(CommitSummary {
            sha: sha.to_owned(),
            branch: tree.branch.clone(),
            subject: subject.to_owned(),
        }),
        conflicts: Vec::new(),
    })
}

fn has_conflicts(tree: &WorkingTree) -> bool {
    tree.files
        .iter()
        .any(|file| file.index == GitFileState::Unmerged || file.worktree == GitFileState::Unmerged)
}

fn no_cherry_pick_in_progress(tree: &WorkingTree) -> Result<(), EngineError> {
    if tree.cherry_pick_head.is_some() {
        return Err(refused(
            "A cherry-pick is in progress. Commit the resolved files or abort it first.",
        ));
    }
    Ok(())
}

fn has_tracked_changes(tree: &WorkingTree) -> bool {
    tree.files.iter().any(|file| {
        !matches!(
            file.index,
            GitFileState::Unchanged | GitFileState::Untracked
        ) || !matches!(
            file.worktree,
            GitFileState::Unchanged | GitFileState::Untracked
        )
    })
}

fn ready_to_move(tree: &WorkingTree, doing: &str) -> Result<(), EngineError> {
    no_cherry_pick_in_progress(tree)?;
    if has_tracked_changes(tree) {
        return Err(EngineError::Other(format!(
            "This checkout has uncommitted changes. Commit or stash them before {doing}."
        )));
    }
    Ok(())
}

async fn git_within(
    root: &Path,
    args: &[&str],
    timeout: std::time::Duration,
    timed_out: String,
) -> Result<Capture, EngineError> {
    tokio::time::timeout(timeout, git(root, args))
        .await
        .map_err(|_| EngineError::Other(timed_out))?
}

async fn hooked_git(
    root: &Path,
    args: &[&str],
    timeout: std::time::Duration,
) -> Result<Capture, EngineError> {
    let timed_out = format!(
        "Git did not finish within {} s. A hook or a signing prompt may be waiting for input; run it in a terminal instead.",
        timeout.as_secs()
    );
    git_within(root, args, timeout, timed_out).await
}

pub(crate) async fn verify_rev(root: &Path, spec: &str) -> Result<Option<String>, EngineError> {
    let out = git_with(
        root,
        &["rev-parse", "--verify", "--quiet", spec],
        1024,
        true,
    )
    .await?;
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    Ok((!sha.is_empty()).then_some(sha))
}

pub(crate) async fn resolve_commit(root: &Path, rev: &str) -> Result<String, EngineError> {
    if rev.len() < 4 || rev.len() > 64 || !rev.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(EngineError::Other(format!("not a commit id: {rev:?}")));
    }
    verify_rev(root, &format!("{rev}^{{commit}}"))
        .await?
        .ok_or_else(|| refused("That commit is not in this repository."))
}

async fn local_branch(root: &Path, name: &str) -> Result<Option<String>, EngineError> {
    verify_rev(root, &format!("refs/heads/{name}")).await
}

async fn checked_branch_name(root: &Path, name: &str) -> Result<(), EngineError> {
    let invalid = || EngineError::Other(format!("{name:?} is not a valid branch name."));
    if name.is_empty()
        || name.len() > 255
        || name.starts_with('-')
        || name.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(invalid());
    }
    let out = git(root, &["check-ref-format", "--branch", name])
        .await
        .map_err(|_| invalid())?;
    if String::from_utf8_lossy(&out.stdout).trim() != name {
        return Err(invalid());
    }
    Ok(())
}

async fn checkout(
    root: &Path,
    tree: &WorkingTree,
    target_ref: &str,
    detach: bool,
) -> Result<(), EngineError> {
    ready_to_move(tree, "checking out another commit")?;
    if detach {
        let sha = resolve_commit(root, target_ref).await?;
        if tree.branch.is_none() && tree.head_sha.as_deref() == Some(sha.as_str()) {
            return Err(refused("HEAD is already detached at that commit."));
        }
        hooked_git(
            root,
            &["switch", "--quiet", "--detach", &sha],
            COMMIT_TIMEOUT,
        )
        .await
        .map(drop)
    } else {
        checked_branch_name(root, target_ref).await?;
        if tree.branch.as_deref() == Some(target_ref) {
            return Err(EngineError::Other(format!(
                "{target_ref} is already checked out here."
            )));
        }
        if local_branch(root, target_ref).await?.is_none() {
            return Err(EngineError::Other(format!(
                "There is no local branch named {target_ref}."
            )));
        }
        hooked_git(
            root,
            &["switch", "--quiet", "--no-guess", "--", target_ref],
            COMMIT_TIMEOUT,
        )
        .await
        .map(drop)
    }
}

async fn create_branch(
    root: &Path,
    tree: &WorkingTree,
    name: &str,
    start_point: Option<&str>,
    checkout: bool,
) -> Result<(), EngineError> {
    checked_branch_name(root, name).await?;
    let Some(head) = tree.head_sha.as_deref() else {
        return Err(refused("This branch has no commits yet."));
    };
    let start = match start_point {
        Some(rev) => resolve_commit(root, rev).await?,
        None => head.to_owned(),
    };
    if local_branch(root, name).await?.is_some() {
        return Err(EngineError::Other(format!(
            "A branch named {name} already exists."
        )));
    }
    if !checkout {
        return git(root, &["branch", "--no-track", "--", name, &start])
            .await
            .map(drop);
    }
    if start == head {
        no_cherry_pick_in_progress(tree)?;
    } else {
        ready_to_move(tree, "switching to a branch at another commit")?;
    }
    hooked_git(
        root,
        &["switch", "--quiet", "--no-track", "-c", name, &start],
        COMMIT_TIMEOUT,
    )
    .await
    .map(drop)
}

async fn unmerged_paths(root: &Path) -> Result<Vec<String>, EngineError> {
    let unmerged = git(
        root,
        &["diff", "--name-only", "--diff-filter=U", "-z", "--no-color"],
    )
    .await?;
    Ok(unmerged
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).into_owned())
        .collect())
}

async fn cherry_pick(
    root: &Path,
    tree: &WorkingTree,
    sha: &str,
) -> Result<WorkingTreeActionReply, EngineError> {
    if tree.head_sha.is_none() {
        return Err(refused("This branch has no commits yet."));
    }
    ready_to_move(tree, "cherry-picking")?;
    let sha = resolve_commit(root, sha).await?;
    let short: String = sha.chars().take(7).collect();
    let parents = git(root, &["rev-list", "--parents", "-n", "1", &sha]).await?;
    if String::from_utf8_lossy(&parents.stdout)
        .split_whitespace()
        .count()
        > 2
    {
        return Err(refused(
            "That is a merge commit. Cherry-picking it needs a parent choice; do it in a terminal.",
        ));
    }
    let range = format!("HEAD..{sha}");
    let missing = git(root, &["rev-list", "--count", &range]).await?;
    if String::from_utf8_lossy(&missing.stdout).trim() == "0" {
        return Err(EngineError::Other(format!(
            "{short} is already on this branch."
        )));
    }
    let Err(error) = hooked_git(root, &["cherry-pick", &sha], COMMIT_TIMEOUT).await else {
        return new_commit_reply(root, tree).await;
    };
    let conflicts = unmerged_paths(root).await?;
    if !conflicts.is_empty() {
        return Ok(WorkingTreeActionReply {
            commit: None,
            conflicts,
        });
    }
    if let Ok(Some(_)) = verify_rev(root, "CHERRY_PICK_HEAD").await {
        git(root, &["cherry-pick", "--abort"]).await?;
    }
    if error.to_string().contains("is now empty") {
        return Err(EngineError::Other(format!(
            "The changes in {short} are already on this branch, so there is nothing to pick."
        )));
    }
    Err(commit_error(error))
}

pub(crate) const SYNC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10 * 60);

pub(crate) fn push_args(remote: &str, branch: &str, dest: &str, set_upstream: bool) -> Vec<String> {
    let mut args = vec!["push".to_owned()];
    if set_upstream {
        args.push("--set-upstream".into());
    }
    args.extend([
        "--".to_owned(),
        remote.to_owned(),
        format!("refs/heads/{branch}:{dest}"),
    ]);
    args
}

pub(crate) const PULL_ARGS: [&str; 4] = ["pull", "--ff-only", "--no-rebase", "--no-autostash"];

fn sync_error(error: EngineError) -> EngineError {
    let EngineError::Other(message) = error else {
        return error;
    };
    let text: String = message
        .lines()
        .filter(|line| !line.trim_start().starts_with("hint:"))
        .collect::<Vec<_>>()
        .join("\n");
    let lower = text.to_ascii_lowercase();
    let lead = if lower.contains("[rejected]") || lower.contains("non-fast-forward") {
        Some("The remote has commits this branch does not have. Pull first, then push again.")
    } else if lower.contains("not possible to fast-forward") {
        Some(
            "This branch and its upstream have diverged, and Pull only fast-forwards. Merge or rebase in a terminal.",
        )
    } else if lower.contains("terminal prompts disabled")
        || lower.contains("could not read username")
        || lower.contains("authentication failed")
        || lower.contains("permission denied (publickey")
        || lower.contains("host key verification failed")
    {
        Some(
            "Git could not sign in to the remote without a prompt. Set up a credential helper or an SSH key in a terminal, then try again.",
        )
    } else {
        None
    };
    EngineError::Other(match lead {
        Some(lead) => format!("{lead}\n{text}"),
        None => text,
    })
}

async fn timed_git(
    root: &Path,
    args: &[&str],
    timeout: std::time::Duration,
) -> Result<(), EngineError> {
    let timed_out = format!(
        "Git did not finish within {} min. The remote may be slow or waiting for a sign-in; try again from a terminal.",
        timeout.as_secs() / 60
    );
    git_within(root, args, timeout, timed_out)
        .await
        .map(drop)
        .map_err(sync_error)
}

pub(crate) async fn sync(
    root: &Path,
    tree: &WorkingTree,
    direction: SyncDirection,
    timeout: std::time::Duration,
) -> Result<u32, EngineError> {
    let Some(branch) = tree.branch.as_deref() else {
        return Err(refused(
            "HEAD is detached. Check out a branch to push or pull.",
        ));
    };
    let Some(head) = tree.head_sha.as_deref() else {
        return Err(refused("This branch has no commits yet."));
    };
    let config = branch_config(root, branch).await;
    match direction {
        SyncDirection::Push { set_upstream } => {
            let (remote, dest, track) = if let Some(upstream) = tree.upstream.as_deref() {
                let not_a_remote_branch = || {
                    refused(
                        "This branch tracks something other than a remote branch. Push it from a terminal.",
                    )
                };
                let remote = config.push_remote(true).ok_or_else(not_a_remote_branch)?;
                let same_name = format!("refs/heads/{branch}");
                let to_upstream =
                    matches!(config.push_mode.as_deref(), Some("upstream" | "tracking"));
                if config.remote.as_deref() != Some(remote.as_str()) {
                    if to_upstream {
                        return Err(refused(&format!(
                            "push.default is upstream, but this branch pushes to {remote}, which is not the remote it tracks. Push it from a terminal."
                        )));
                    }
                    (remote, same_name, false)
                } else {
                    let merge = config
                        .merge
                        .clone()
                        .filter(|merge| merge.starts_with("refs/heads/"))
                        .ok_or_else(not_a_remote_branch)?;
                    if merge == same_name {
                        (remote, same_name, false)
                    } else if set_upstream {
                        (remote, same_name, true)
                    } else if to_upstream || config.push_mode.as_deref() == Some("current") {
                        let dest = if to_upstream { merge } else { same_name };
                        (remote, dest, false)
                    } else {
                        return Err(refused(&format!(
                            "{branch} tracks {upstream}, a branch with another name, so Push will not send it there. To publish it as its own branch, run `git push -u {remote} {branch}` in a terminal."
                        )));
                    }
                }
            } else {
                if !set_upstream {
                    return Err(refused(
                        "This branch is not on a remote yet. Push it with tracking turned on.",
                    ));
                }
                let remote = config.push_remote(false).ok_or_else(|| {
                    refused(
                        "There is no remote to push to. Add one named origin, or set remote.pushDefault.",
                    )
                })?;
                (remote, format!("refs/heads/{branch}"), true)
            };
            let args = push_args(&remote, branch, &dest, track);
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            timed_git(root, &args, timeout).await?;
            Ok(tree.ahead)
        }
        SyncDirection::Pull => {
            if tree.upstream.is_none() {
                return Err(refused(
                    "This branch has no upstream to pull from. Push it first.",
                ));
            }
            timed_git(root, &PULL_ARGS, timeout).await?;
            let range = format!("{head}..HEAD");
            let count = git(root, &["rev-list", "--count", &range]).await?;
            Ok(String::from_utf8_lossy(&count.stdout)
                .trim()
                .parse()
                .unwrap_or(0))
        }
    }
}

pub(crate) async fn file_diff(
    root: &Path,
    checkout_id: &str,
    path: &str,
    staged: bool,
) -> Result<WorkingTreeFileDiff, EngineError> {
    checked_path(path)?;
    let status = git(
        root,
        &["status", "--porcelain=v2", "-z", "--untracked-files=all"],
    )
    .await?;
    let file = parse_status(&status.stdout, status.truncated)
        .files
        .into_iter()
        .find(|file| file.path == path)
        .filter(|file| {
            if staged {
                is_staged(file)
            } else {
                file.worktree != GitFileState::Unchanged
            }
        })
        .ok_or_else(|| {
            EngineError::Other(format!(
                "{path} has no {} changes",
                if staged { "staged" } else { "unstaged" }
            ))
        })?;
    let common = ["--no-ext-diff", "--no-color", "--no-textconv"];
    let output = if staged {
        let mut args = vec!["diff", "--cached", "-M"];
        args.extend(common);
        args.extend(["--", path]);
        if let Some(old) = file.old_path.as_deref() {
            args.push(old);
        }
        git_with(root, &args, PATCH_MAX_BYTES, false).await?
    } else if file.worktree == GitFileState::Untracked {
        let mut args = vec!["diff", "--no-index"];
        args.extend(common);
        args.extend(["--", "/dev/null", path]);
        git_with(root, &args, PATCH_MAX_BYTES, true).await?
    } else {
        let mut args = vec!["diff"];
        args.extend(common);
        args.extend(["--", path]);
        git_with(root, &args, PATCH_MAX_BYTES, false).await?
    };
    let mut bytes = output.stdout;
    if output.truncated
        && let Some(end) = bytes.iter().rposition(|byte| *byte == b'\n')
    {
        bytes.truncate(end + 1);
    }
    let patch = String::from_utf8_lossy(&bytes).into_owned();
    let binary = patch
        .lines()
        .any(|line| line.starts_with("Binary files ") || line == "GIT binary patch");
    Ok(WorkingTreeFileDiff {
        checkout_id: checkout_id.to_owned(),
        path: path.to_owned(),
        staged,
        patch,
        binary,
        truncated: output.truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commit_messages::tests::git as run;

    #[test]
    fn porcelain_v2_reads_headers_renames_conflicts_and_untracked() {
        let bytes = b"# branch.oid 1111111111111111111111111111111111111111\0# branch.head feat/x\0# branch.upstream origin/feat/x\0# branch.ab +2 -3\01 M. N... 100644 100644 100644 aaa bbb src/a b.rs\01 .M N... 100644 100644 100644 aaa aaa -lead\02 R. N... 100644 100644 100644 aaa aaa R100 new name.rs\0old name.rs\0u UU N... 100644 100644 100644 100644 a b c conflict.rs\0? notes/new file.txt\0! ignored\0";
        let status = parse_status(bytes, false);
        assert!(status.complete);
        assert_eq!(
            status.head_sha.as_deref(),
            Some("1111111111111111111111111111111111111111")
        );
        assert_eq!(status.branch.as_deref(), Some("feat/x"));
        assert_eq!(status.upstream.as_deref(), Some("origin/feat/x"));
        assert_eq!((status.ahead, status.behind), (2, 3));
        let rows: Vec<_> = status
            .files
            .iter()
            .map(|f| (f.path.as_str(), f.old_path.as_deref(), f.index, f.worktree))
            .collect();
        use GitFileState::*;
        assert_eq!(
            rows,
            [
                ("-lead", None, Unchanged, Modified),
                ("conflict.rs", None, Unmerged, Unmerged),
                ("new name.rs", Some("old name.rs"), Renamed, Unchanged),
                ("notes/new file.txt", None, Untracked, Untracked),
                ("src/a b.rs", None, Modified, Unchanged),
            ]
        );
    }

    #[test]
    fn a_staged_deletion_and_its_untracked_replacement_are_one_row() {
        let status = parse_status(
            b"1 D. N... 100644 000000 000000 aaa 000 notes.md\0? notes.md\0",
            false,
        );
        assert!(status.complete);
        assert_eq!(status.files.len(), 1);
        assert_eq!(
            (status.files[0].index, status.files[0].worktree),
            (GitFileState::Deleted, GitFileState::Untracked)
        );
    }

    #[test]
    fn initial_and_detached_heads_have_no_sha_or_branch() {
        let status = parse_status(b"# branch.oid (initial)\0# branch.head main\0", false);
        assert_eq!(
            (status.head_sha, status.branch.as_deref()),
            (None, Some("main"))
        );
        let detached = parse_status(b"# branch.oid abc\0# branch.head (detached)\0", false);
        assert_eq!(detached.branch, None);
        assert_eq!(detached.head_sha.as_deref(), Some("abc"));
    }

    #[test]
    fn a_cut_record_is_never_complete() {
        assert!(!parse_status(b"1 M. N... 100644", false).complete);
        assert!(!parse_status(b"? a\0", true).complete);
        assert!(!parse_status(b"1 M. broken\0", false).complete);
    }

    #[test]
    fn numstat_keys_renames_by_destination_and_flags_binary() {
        let deltas = parse_numstat(b"3\t1\tsrc/a b.rs\0-\t-\timage.png\02\t0\t\0old.rs\0new.rs\0");
        assert_eq!(
            deltas["src/a b.rs"],
            LineDelta {
                additions: 3,
                deletions: 1,
                binary: false
            }
        );
        assert!(deltas["image.png"].binary);
        assert_eq!(deltas["new.rs"].additions, 2);
        assert!(!deltas.contains_key("old.rs"));
    }

    #[tokio::test]
    async fn a_real_repository_reports_sections_counts_tracking_and_stashes() {
        let dir = tempfile::tempdir().unwrap();
        let remote = dir.path().join("remote.git");
        let repo = dir.path().join("repo");
        run(
            dir.path(),
            &["init", "--bare", "-b", "main", remote.to_str().unwrap()],
        );
        run(dir.path(), &["init", "-b", "main", repo.to_str().unwrap()]);
        run(
            &repo,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        std::fs::write(repo.join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(repo.join("b.txt"), "b\n").unwrap();
        run(&repo, &["add", "."]);
        run(&repo, &["commit", "-m", "first"]);
        run(&repo, &["push", "-u", "origin", "main"]);
        std::fs::write(repo.join("c.txt"), "c\n").unwrap();
        run(&repo, &["add", "."]);
        run(&repo, &["commit", "-m", "second"]);
        std::fs::write(repo.join("stash.txt"), "s\n").unwrap();
        run(&repo, &["add", "stash.txt"]);
        run(&repo, &["stash", "push", "-m", "parked work"]);

        std::fs::write(repo.join("a.txt"), "one\nTWO\nthree\n").unwrap();
        run(&repo, &["add", "a.txt"]);
        std::fs::write(repo.join("a.txt"), "one\nTWO\nthree\nfour\n").unwrap();
        std::fs::remove_file(repo.join("b.txt")).unwrap();
        std::fs::create_dir(repo.join("new dir")).unwrap();
        std::fs::write(repo.join("new dir/with space.md"), "x\ny\nz").unwrap();

        let tree = read(&repo, "checkout-1").await.unwrap();
        assert_eq!(tree.checkout_id, "checkout-1");
        assert_eq!(tree.branch.as_deref(), Some("main"));
        assert_eq!(tree.upstream.as_deref(), Some("origin/main"));
        assert_eq!((tree.ahead, tree.behind), (1, 0));
        assert!(!tree.truncated);
        let a = tree.files.iter().find(|f| f.path == "a.txt").unwrap();
        assert_eq!(
            (a.index, a.worktree),
            (GitFileState::Modified, GitFileState::Modified)
        );
        assert_eq!(
            a.staged,
            Some(LineDelta {
                additions: 2,
                deletions: 1,
                binary: false
            })
        );
        assert_eq!(
            a.unstaged,
            Some(LineDelta {
                additions: 1,
                deletions: 0,
                binary: false
            })
        );
        let b = tree.files.iter().find(|f| f.path == "b.txt").unwrap();
        assert_eq!(b.worktree, GitFileState::Deleted);
        assert_eq!(b.unstaged.map(|d| d.deletions), Some(1));
        let new = tree
            .files
            .iter()
            .find(|f| f.path == "new dir/with space.md")
            .unwrap();
        assert_eq!(new.worktree, GitFileState::Untracked);
        assert_eq!(new.unstaged.map(|d| d.additions), Some(3));
        assert_eq!(tree.stashes.len(), 1);
        assert!(
            tree.stashes[0].message.contains("parked work"),
            "{:?}",
            tree.stashes
        );
        assert_eq!(tree.stashes[0].oid.len(), 40);

        let again = read(&repo, "checkout-1").await.unwrap();
        assert_eq!(again.revision, tree.revision);
        std::fs::write(repo.join("a.txt"), "changed\n").unwrap();
        assert_ne!(
            read(&repo, "checkout-1").await.unwrap().revision,
            tree.revision
        );
    }

    fn porcelain(cwd: &Path) -> String {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(["-c", "core.quotePath=false", "status", "--porcelain"])
            .output()
            .unwrap();
        String::from_utf8(output.stdout).unwrap().replace('"', "")
    }

    fn committed_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), &["init", "-b", "main", "."]);
        std::fs::write(dir.path().join("tracked.txt"), "one\n").unwrap();
        std::fs::write(dir.path().join("old name.txt"), "rename me\n").unwrap();
        run(dir.path(), &["add", "."]);
        run(dir.path(), &["commit", "-m", "first"]);
        dir
    }

    fn stage(paths: &[&str]) -> WorkingTreeAction {
        WorkingTreeAction::Stage {
            paths: paths.iter().map(|path| (*path).to_owned()).collect(),
        }
    }

    fn unstage(paths: &[&str]) -> WorkingTreeAction {
        WorkingTreeAction::Unstage {
            paths: paths.iter().map(|path| (*path).to_owned()).collect(),
        }
    }

    #[tokio::test]
    async fn staging_treats_every_name_as_a_literal_path() {
        let dir = committed_repo();
        let root = dir.path();
        for name in [
            "with space.txt",
            "ünïcødé 文件.txt",
            "-leading-dash",
            ":(glob)*",
            "glob-bait.txt",
        ] {
            std::fs::write(root.join(name), "x\n").unwrap();
        }
        let tree = read(root, "c").await.unwrap();
        apply(
            root,
            &tree,
            &stage(&[
                "with space.txt",
                "ünïcødé 文件.txt",
                "-leading-dash",
                ":(glob)*",
            ]),
        )
        .await
        .unwrap();
        assert_eq!(
            porcelain(root),
            "A  -leading-dash\nA  :(glob)*\nA  with space.txt\nA  ünïcødé 文件.txt\n?? glob-bait.txt\n",
            "`:(glob)*` is a file name, not a pattern"
        );

        let tree = read(root, "c").await.unwrap();
        apply(root, &tree, &unstage(&[":(glob)*", "-leading-dash"]))
            .await
            .unwrap();
        assert_eq!(
            porcelain(root),
            "A  with space.txt\nA  ünïcødé 文件.txt\n?? -leading-dash\n?? :(glob)*\n?? glob-bait.txt\n"
        );
    }

    #[tokio::test]
    async fn paths_outside_the_listed_tree_are_refused() {
        let dir = committed_repo();
        let root = dir.path();
        std::fs::write(root.join("tracked.txt"), "two\n").unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "s\n").unwrap();
        let tree = read(root, "c").await.unwrap();
        let escape = format!(
            "../{}/secret.txt",
            outside.path().file_name().unwrap().to_string_lossy()
        );
        let absolute = outside.path().join("secret.txt");
        for path in [
            escape.as_str(),
            absolute.to_str().unwrap(),
            "sub/../tracked.txt",
            "./tracked.txt",
            ".git/config",
            ".GIT/HEAD",
            "",
            "tracked.txt/",
            "nul\0byte",
        ] {
            let error = apply(root, &tree, &stage(&[path])).await.unwrap_err();
            assert!(
                error.to_string().contains("not a working tree path"),
                "{path:?}: {error}"
            );
        }
        let error = apply(root, &tree, &stage(&["old name.txt"]))
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("has no unstaged changes"),
            "{error}"
        );
        let error = apply(root, &tree, &unstage(&["tracked.txt"]))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("is not staged"), "{error}");
        let error = apply(root, &tree, &stage(&[])).await.unwrap_err();
        assert!(error.to_string().contains("no files"), "{error}");
        let error = apply(root, &tree, &WorkingTreeAction::AbortCherryPick)
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("no cherry-pick to abort"),
            "{error}"
        );
        assert_eq!(porcelain(root), " M tracked.txt\n");
        assert!(file_diff(root, "c", &escape, false).await.is_err());
        assert!(
            file_diff(root, "c", absolute.to_str().unwrap(), false)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn unstage_takes_a_rename_back_whole_and_the_all_actions_cover_the_tree() {
        let dir = committed_repo();
        let root = dir.path();
        run(root, &["mv", "old name.txt", "new name.txt"]);
        std::fs::remove_file(root.join("tracked.txt")).unwrap();
        std::fs::write(root.join("fresh.txt"), "f\n").unwrap();
        let tree = read(root, "c").await.unwrap();
        let renamed = tree
            .files
            .iter()
            .find(|file| file.path == "new name.txt")
            .unwrap();
        assert_eq!(renamed.old_path.as_deref(), Some("old name.txt"));
        apply(root, &tree, &unstage(&["new name.txt"]))
            .await
            .unwrap();
        assert_eq!(
            porcelain(root),
            " D old name.txt\n D tracked.txt\n?? fresh.txt\n?? new name.txt\n"
        );

        let tree = read(root, "c").await.unwrap();
        apply(root, &tree, &WorkingTreeAction::StageAll)
            .await
            .unwrap();
        assert_eq!(
            porcelain(root),
            "A  fresh.txt\nR  old name.txt -> new name.txt\nD  tracked.txt\n"
        );
        let tree = read(root, "c").await.unwrap();
        apply(root, &tree, &WorkingTreeAction::UnstageAll)
            .await
            .unwrap();
        assert_eq!(
            porcelain(root),
            " D old name.txt\n D tracked.txt\n?? fresh.txt\n?? new name.txt\n"
        );
    }

    #[tokio::test]
    async fn unstaging_works_before_the_first_commit() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        run(root, &["init", "-b", "main", "."]);
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        std::fs::write(root.join("b.txt"), "b\n").unwrap();
        run(root, &["add", "."]);
        std::fs::write(root.join("a.txt"), "a\nmore\n").unwrap();
        let tree = read(root, "c").await.unwrap();
        apply(root, &tree, &unstage(&["a.txt"])).await.unwrap();
        assert_eq!(porcelain(root), "A  b.txt\n?? a.txt\n");
        let tree = read(root, "c").await.unwrap();
        apply(root, &tree, &WorkingTreeAction::UnstageAll)
            .await
            .unwrap();
        assert_eq!(porcelain(root), "?? a.txt\n?? b.txt\n");
    }

    #[tokio::test]
    async fn file_diffs_cover_staged_unstaged_untracked_and_binary_files() {
        let dir = committed_repo();
        let root = dir.path();
        std::fs::write(root.join("tracked.txt"), "one\ntwo\n").unwrap();
        run(root, &["add", "tracked.txt"]);
        std::fs::write(root.join("tracked.txt"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(root.join("-new file.txt"), "x\ny\n").unwrap();
        std::fs::write(root.join("blob.bin"), [0u8, 1, 2, 0, 255]).unwrap();

        let staged = file_diff(root, "c", "tracked.txt", true).await.unwrap();
        assert!(staged.patch.contains("+two"), "{}", staged.patch);
        assert!(!staged.patch.contains("+three"));
        assert!(staged.staged && !staged.binary && !staged.truncated);
        let unstaged = file_diff(root, "c", "tracked.txt", false).await.unwrap();
        assert!(unstaged.patch.contains("+three") && !unstaged.patch.contains("+two"));
        let untracked = file_diff(root, "c", "-new file.txt", false).await.unwrap();
        assert!(
            untracked.patch.contains("new file mode"),
            "{}",
            untracked.patch
        );
        assert!(untracked.patch.contains("+x\n+y"), "{}", untracked.patch);
        let parsed = untracked
            .patch
            .lines()
            .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
            .count();
        assert_eq!(parsed, 2);
        let binary = file_diff(root, "c", "blob.bin", false).await.unwrap();
        assert!(binary.binary, "{}", binary.patch);

        let error = file_diff(root, "c", "old name.txt", false)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("no unstaged changes"), "{error}");
        let error = file_diff(root, "c", "-new file.txt", true)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("no staged changes"), "{error}");
    }

    #[tokio::test]
    async fn checkout_locks_serialize_one_checkout_only() {
        let first = lock_checkout("lock-a").await;
        let other = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            lock_checkout("lock-b"),
        )
        .await;
        assert!(other.is_ok(), "another checkout is not blocked");
        let same = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            lock_checkout("lock-a"),
        )
        .await;
        assert!(same.is_err(), "the same checkout waits");
        drop(first);
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(200),
                lock_checkout("lock-a")
            )
            .await
            .is_ok()
        );
    }

    fn local_identity(root: &Path) {
        for (key, value) in [
            ("user.name", "Lab"),
            ("user.email", "lab@example.com"),
            ("commit.gpgsign", "false"),
        ] {
            run(root, &["config", key, value]);
        }
    }

    fn output(root: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        String::from_utf8(output.stdout).unwrap()
    }

    fn head(root: &Path) -> String {
        output(root, &["rev-parse", "HEAD"]).trim().to_owned()
    }

    fn commit_action(message: &str, amend: bool, all: bool) -> WorkingTreeAction {
        WorkingTreeAction::Commit {
            message: message.into(),
            amend,
            all,
        }
    }

    #[tokio::test]
    async fn commit_takes_the_staged_files_only_and_reports_the_new_commit() {
        let dir = committed_repo();
        let root = dir.path();
        local_identity(root);
        std::fs::write(root.join("tracked.txt"), "two\n").unwrap();
        run(root, &["add", "tracked.txt"]);
        std::fs::write(root.join("tracked.txt"), "two\nthree\n").unwrap();
        std::fs::write(root.join("untracked.txt"), "u\n").unwrap();
        let before = head(root);

        let tree = read(root, "c").await.unwrap();
        let reply = apply(
            root,
            &tree,
            &commit_action("Subject line\n\nWhy it changed.\n\n\n", false, false),
        )
        .await
        .unwrap();
        let commit = reply.commit.expect("a commit summary");
        assert_eq!(commit.sha, head(root));
        assert_eq!(commit.sha.len(), 40);
        assert_eq!(commit.subject, "Subject line");
        assert_eq!(commit.branch.as_deref(), Some("main"));
        assert!(reply.conflicts.is_empty());
        assert_eq!(output(root, &["rev-parse", "HEAD^"]).trim(), before);
        assert_eq!(output(root, &["show", "HEAD:tracked.txt"]), "two\n");
        assert_eq!(
            output(root, &["log", "-1", "--format=%B"]),
            "Subject line\n\nWhy it changed.\n\n"
        );
        assert_eq!(porcelain(root), " M tracked.txt\n?? untracked.txt\n");
    }

    #[tokio::test]
    async fn a_commit_without_a_message_or_staged_files_is_refused_untouched() {
        let dir = committed_repo();
        let root = dir.path();
        local_identity(root);
        std::fs::write(root.join("tracked.txt"), "two\n").unwrap();
        let before = head(root);
        let tree = read(root, "c").await.unwrap();
        for message in ["", "  \n\t\n"] {
            let error = apply(root, &tree, &commit_action(message, false, false))
                .await
                .unwrap_err();
            assert!(
                error.to_string().contains("Write a commit message"),
                "{error}"
            );
        }
        let error = apply(root, &tree, &commit_action("x", false, false))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("Nothing is staged"), "{error}");
        let error = apply(
            root,
            &tree,
            &commit_action(&"x".repeat(MAX_MESSAGE_BYTES + 1), false, true),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("too long"), "{error}");
        assert_eq!(head(root), before);
        assert_eq!(porcelain(root), " M tracked.txt\n");

        run(root, &["checkout", "--", "tracked.txt"]);
        let tree = read(root, "c").await.unwrap();
        let error = apply(root, &tree, &commit_action("x", false, true))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("nothing to commit"), "{error}");
        assert_eq!(head(root), before);
    }

    #[tokio::test]
    async fn commit_all_stages_every_change_first() {
        let dir = committed_repo();
        let root = dir.path();
        local_identity(root);
        std::fs::write(root.join("tracked.txt"), "two\n").unwrap();
        std::fs::remove_file(root.join("old name.txt")).unwrap();
        std::fs::write(root.join("-new file.txt"), "n\n").unwrap();
        let tree = read(root, "c").await.unwrap();
        let reply = apply(root, &tree, &commit_action("Everything", false, true))
            .await
            .unwrap();
        assert_eq!(reply.commit.unwrap().subject, "Everything");
        assert_eq!(porcelain(root), "");
        assert_eq!(
            output(root, &["show", "--name-status", "--format=", "HEAD"]),
            "A\t-new file.txt\nD\told name.txt\nM\ttracked.txt\n"
        );
    }

    #[tokio::test]
    async fn amend_folds_into_an_unpushed_commit_and_refuses_pushed_or_missing_heads() {
        let dir = tempfile::tempdir().unwrap();
        let remote = dir.path().join("remote.git");
        let repo = dir.path().join("repo");
        run(
            dir.path(),
            &["init", "--bare", "-b", "main", remote.to_str().unwrap()],
        );
        run(dir.path(), &["init", "-b", "main", repo.to_str().unwrap()]);
        local_identity(&repo);
        run(
            &repo,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        run(&repo, &["add", "."]);
        run(&repo, &["commit", "-m", "first"]);
        run(&repo, &["push", "-u", "origin", "main"]);
        let pushed = head(&repo);

        let tree = read(&repo, "c").await.unwrap();
        let error = apply(&repo, &tree, &commit_action("Reworded", true, false))
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("already on origin/main"),
            "{error}"
        );
        assert_eq!(head(&repo), pushed);

        std::fs::write(repo.join("b.txt"), "b\n").unwrap();
        run(&repo, &["add", "."]);
        run(&repo, &["commit", "-m", "second"]);
        std::fs::write(repo.join("forgotten.txt"), "f\n").unwrap();
        run(&repo, &["add", "forgotten.txt"]);
        let tree = read(&repo, "c").await.unwrap();
        let reply = apply(&repo, &tree, &commit_action("Second, amended", true, false))
            .await
            .unwrap();
        assert_eq!(reply.commit.unwrap().subject, "Second, amended");
        assert_eq!(output(&repo, &["rev-parse", "HEAD^"]).trim(), pushed);
        assert_eq!(output(&repo, &["rev-list", "--count", "HEAD"]).trim(), "2");
        assert_eq!(output(&repo, &["show", "HEAD:forgotten.txt"]), "f\n");

        run(&repo, &["push", "origin", "HEAD:refs/heads/backup"]);
        let amended = head(&repo);
        let tree = read(&repo, "c").await.unwrap();
        let error = apply(&repo, &tree, &commit_action("Again", true, false))
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("already on origin/backup"),
            "any remote-tracking ref counts, not only the upstream: {error}"
        );
        assert_eq!(head(&repo), amended);

        let unborn = tempfile::tempdir().unwrap();
        run(unborn.path(), &["init", "-b", "main", "."]);
        local_identity(unborn.path());
        std::fs::write(unborn.path().join("x.txt"), "x\n").unwrap();
        run(unborn.path(), &["add", "."]);
        let tree = read(unborn.path(), "c").await.unwrap();
        let error = apply(unborn.path(), &tree, &commit_action("x", true, false))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("no commit to amend"), "{error}");
    }

    #[tokio::test]
    async fn the_first_commit_of_an_unborn_branch_works() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        run(root, &["init", "-b", "trunk", "."]);
        local_identity(root);
        std::fs::write(root.join("first.rs"), "fn main() {}\n").unwrap();
        run(root, &["add", "first.rs"]);
        let tree = read(root, "c").await.unwrap();
        let commit = apply(root, &tree, &commit_action("First", false, false))
            .await
            .unwrap()
            .commit
            .unwrap();
        assert_eq!(commit.branch.as_deref(), Some("trunk"));
        assert_eq!(commit.sha, head(root));
    }

    #[tokio::test]
    async fn missing_identity_and_failed_signing_read_as_plain_errors() {
        let dir = committed_repo();
        let root = dir.path();
        local_identity(root);
        run(root, &["config", "user.name", ""]);
        run(root, &["config", "user.email", ""]);
        std::fs::write(root.join("tracked.txt"), "two\n").unwrap();
        run(root, &["add", "tracked.txt"]);
        let before = head(root);
        let tree = read(root, "c").await.unwrap();
        let error = apply(root, &tree, &commit_action("x", false, false))
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("Git does not know who you are"),
            "{error}"
        );

        local_identity(root);
        run(root, &["config", "commit.gpgsign", "true"]);
        run(root, &["config", "gpg.program", "false"]);
        let error = apply(root, &tree, &commit_action("x", false, false))
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("could not sign the commit"),
            "{error}"
        );
        assert_eq!(head(root), before);
        assert_eq!(porcelain(root), "M  tracked.txt\n");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn commit_hooks_run_and_a_hook_that_hangs_times_out() {
        use std::os::unix::fs::PermissionsExt;
        let dir = committed_repo();
        let root = dir.path();
        local_identity(root);
        let hooks = tempfile::tempdir().unwrap();
        run(
            root,
            &["config", "core.hooksPath", hooks.path().to_str().unwrap()],
        );
        let hook = hooks.path().join("pre-commit");
        let install = |script: &str| {
            std::fs::write(&hook, script).unwrap();
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        std::fs::write(root.join("tracked.txt"), "two\n").unwrap();
        run(root, &["add", "tracked.txt"]);
        let before = head(root);
        let tree = read(root, "c").await.unwrap();

        install("#!/bin/sh\necho 'lint found 3 problems' >&2\nexit 1\n");
        let error = apply(root, &tree, &commit_action("x", false, false))
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("lint found 3 problems"),
            "{error}"
        );

        install("#!/bin/sh\nsleep 5\n");
        let started = std::time::Instant::now();
        let error = commit(
            root,
            &tree,
            "x",
            false,
            false,
            std::time::Duration::from_millis(300),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("did not finish"), "{error}");
        assert!(started.elapsed() < std::time::Duration::from_secs(4));
        assert_eq!(head(root), before);
        assert!(COMMIT_TIMEOUT < WRITE_TIMEOUT);
    }

    #[tokio::test]
    async fn an_unborn_branch_lists_staged_files_without_a_head() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), &["init", "-b", "trunk", "."]);
        std::fs::write(dir.path().join("first.rs"), "fn main() {}\n").unwrap();
        run(dir.path(), &["add", "first.rs"]);
        let tree = read(dir.path(), "c").await.unwrap();
        assert_eq!(tree.head_sha, None);
        assert_eq!(tree.branch.as_deref(), Some("trunk"));
        assert_eq!(tree.files[0].index, GitFileState::Added);
        assert_eq!(tree.files[0].staged.map(|d| d.additions), Some(1));
    }

    fn discard_paths(paths: &[&str]) -> WorkingTreeAction {
        WorkingTreeAction::Discard {
            paths: paths.iter().map(|path| (*path).to_owned()).collect(),
        }
    }

    fn identified_repo() -> tempfile::TempDir {
        let dir = committed_repo();
        local_identity(dir.path());
        dir
    }

    fn stash_oids(root: &Path) -> Vec<String> {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["stash", "list", "--format=%H"])
            .output()
            .unwrap();
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[tokio::test]
    async fn discard_restores_the_unstaged_side_and_removes_exact_untracked_files() {
        let dir = committed_repo();
        let root = dir.path();
        std::fs::write(root.join("tracked.txt"), "staged\n").unwrap();
        run(root, &["add", "tracked.txt"]);
        std::fs::write(root.join("tracked.txt"), "staged\nand more\n").unwrap();
        std::fs::remove_file(root.join("old name.txt")).unwrap();
        std::fs::create_dir_all(root.join("new dir/deeper")).unwrap();
        std::fs::write(root.join("new dir/deeper/-a b.txt"), "x\n").unwrap();
        std::fs::write(root.join("keep.txt"), "keep\n").unwrap();
        std::fs::write(root.join(":(glob)*"), "bait\n").unwrap();

        let tree = read(root, "c").await.unwrap();
        apply(
            root,
            &tree,
            &discard_paths(&[
                "tracked.txt",
                "old name.txt",
                "new dir/deeper/-a b.txt",
                ":(glob)*",
            ]),
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("tracked.txt")).unwrap(),
            "staged\n",
            "the staged version stays"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("old name.txt")).unwrap(),
            "rename me\n"
        );
        assert!(!root.join("new dir").exists(), "emptied parents are pruned");
        assert!(root.join("keep.txt").exists());
        assert_eq!(porcelain(root), "M  tracked.txt\n?? keep.txt\n");
    }

    #[tokio::test]
    async fn discard_refuses_directories_conflicts_and_rows_without_unstaged_changes() {
        let dir = committed_repo();
        let root = dir.path();
        run(root, &["init", "-b", "main", "nested"]);
        std::fs::write(root.join("nested/inner.txt"), "inner\n").unwrap();
        std::fs::create_dir(root.join("plain dir")).unwrap();
        std::fs::write(root.join("plain dir/file.txt"), "p\n").unwrap();
        std::fs::remove_file(root.join("old name.txt")).unwrap();
        std::fs::create_dir(root.join("old name.txt")).unwrap();
        std::fs::write(root.join("old name.txt/x"), "x\n").unwrap();
        std::fs::write(root.join("tracked.txt"), "staged\n").unwrap();
        run(root, &["add", "tracked.txt"]);
        let tree = read(root, "c").await.unwrap();
        assert!(tree.files.iter().any(|file| file.path == "nested/"));
        let before = porcelain(root);

        for path in ["nested/", "nested", "plain dir", "old name.txt"] {
            let error = apply(root, &tree, &discard_paths(&[path]))
                .await
                .unwrap_err();
            assert!(
                error.to_string().contains("is a directory"),
                "{path}: {error}"
            );
        }
        let error = apply(root, &tree, &discard_paths(&["tracked.txt"]))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("no unstaged changes"), "{error}");
        for path in ["../x", "nested/inner.txt", "missing.txt"] {
            assert!(
                apply(root, &tree, &discard_paths(&[path])).await.is_err(),
                "{path}"
            );
        }
        assert_eq!(porcelain(root), before);
        assert!(root.join("nested/inner.txt").exists());
        assert!(root.join("plain dir/file.txt").exists());
        assert!(root.join("old name.txt/x").exists());

        let mut conflicted = tree.clone();
        conflicted.files.push(file(
            "conflict.txt",
            None,
            GitFileState::Unmerged,
            GitFileState::Unmerged,
        ));
        let error = apply(root, &conflicted, &discard_paths(&["conflict.txt"]))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("merge conflict"), "{error}");
    }

    #[tokio::test]
    async fn a_rewrite_with_the_same_line_counts_still_changes_the_revision() {
        let dir = committed_repo();
        let root = dir.path();
        std::fs::write(root.join("tracked.txt"), "two\n").unwrap();
        let shown = read(root, "c").await.unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(root.join("tracked.txt"), "TWO\n").unwrap();
        let now = read(root, "c").await.unwrap();
        assert_eq!(
            shown.files, now.files,
            "status and line counts are identical"
        );
        assert_ne!(shown.revision, now.revision);
        assert_eq!(read(root, "c").await.unwrap().revision, now.revision);
    }

    #[tokio::test]
    async fn restaging_the_same_line_counts_still_changes_the_revision() {
        let dir = committed_repo();
        let root = dir.path();
        std::fs::write(root.join("tracked.txt"), "two\n").unwrap();
        run(root, &["add", "tracked.txt"]);
        let shown = read(root, "c").await.unwrap();
        std::fs::write(root.join("tracked.txt"), "three\n").unwrap();
        run(root, &["add", "tracked.txt"]);
        let now = read(root, "c").await.unwrap();
        assert_eq!(
            shown.files, now.files,
            "status and staged line counts are identical"
        );
        assert_ne!(shown.revision, now.revision);
        assert_eq!(read(root, "c").await.unwrap().revision, now.revision);
    }

    #[tokio::test]
    async fn stash_push_takes_untracked_files_and_refuses_what_it_cannot_carry() {
        let dir = identified_repo();
        let root = dir.path();
        let tree = read(root, "c").await.unwrap();
        let push = |message: Option<&str>| WorkingTreeAction::StashPush {
            message: message.map(str::to_owned),
        };
        let error = apply(root, &tree, &push(None)).await.unwrap_err();
        assert!(error.to_string().contains("nothing to stash"), "{error}");

        std::fs::write(root.join("tracked.txt"), "changed\n").unwrap();
        std::fs::write(root.join("-new file.txt"), "n\n").unwrap();
        let tree = read(root, "c").await.unwrap();
        apply(root, &tree, &push(Some("  -m sneaky message  ")))
            .await
            .unwrap();
        assert_eq!(porcelain(root), "");
        let tree = read(root, "c").await.unwrap();
        assert_eq!(tree.stashes.len(), 1);
        assert!(
            tree.stashes[0].message.ends_with(": -m sneaky message"),
            "{:?}",
            tree.stashes
        );

        run(root, &["init", "-b", "main", "nested"]);
        std::fs::write(root.join("nested/inner.txt"), "inner\n").unwrap();
        let tree = read(root, "c").await.unwrap();
        let error = apply(root, &tree, &push(None)).await.unwrap_err();
        assert!(error.to_string().contains("nested repository"), "{error}");
        assert_eq!(stash_oids(root).len(), 1);
    }

    #[tokio::test]
    async fn drop_and_pop_follow_the_oid_after_another_push_shifts_the_indexes() {
        let dir = identified_repo();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "first stash\n").unwrap();
        run(
            root,
            &["stash", "push", "--include-untracked", "-m", "first"],
        );
        let first = stash_oids(root)[0].clone();
        let shown = read(root, "c").await.unwrap();
        std::fs::write(root.join("b.txt"), "second stash\n").unwrap();
        run(
            root,
            &["stash", "push", "--include-untracked", "-m", "second"],
        );
        let second = stash_oids(root)[0].clone();
        assert_eq!(stash_oids(root), [second.clone(), first.clone()]);

        apply(
            root,
            &shown,
            &WorkingTreeAction::StashDrop { oid: first.clone() },
        )
        .await
        .unwrap();
        assert_eq!(
            stash_oids(root),
            [second.clone()],
            "the newer stash survived"
        );
        let error = apply(root, &shown, &WorkingTreeAction::StashDrop { oid: first })
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("no longer in the list"),
            "{error}"
        );
        for oid in ["", "stash@{0}", "HEAD", "--all"] {
            let error = apply(
                root,
                &shown,
                &WorkingTreeAction::StashDrop { oid: oid.into() },
            )
            .await
            .unwrap_err();
            assert!(
                error.to_string().contains("not a stash id"),
                "{oid}: {error}"
            );
        }

        let tree = read(root, "c").await.unwrap();
        let reply = apply(root, &tree, &WorkingTreeAction::StashPop { oid: second })
            .await
            .unwrap();
        assert_eq!(reply, WorkingTreeActionReply::default());
        assert!(stash_oids(root).is_empty());
        assert_eq!(porcelain(root), "?? b.txt\n");
    }

    #[tokio::test]
    async fn a_pop_that_conflicts_keeps_the_stash_and_names_the_files() {
        let dir = identified_repo();
        let root = dir.path();
        std::fs::write(root.join("tracked.txt"), "stashed side\n").unwrap();
        run(root, &["stash", "push", "-m", "parked"]);
        let oid = stash_oids(root)[0].clone();

        std::fs::write(root.join("tracked.txt"), "local edit\n").unwrap();
        let tree = read(root, "c").await.unwrap();
        let error = apply(
            root,
            &tree,
            &WorkingTreeAction::StashPop { oid: oid.clone() },
        )
        .await
        .unwrap_err();
        assert!(error.to_string().starts_with("git:"), "{error}");
        assert_eq!(stash_oids(root), [oid.clone()]);
        assert_eq!(
            std::fs::read_to_string(root.join("tracked.txt")).unwrap(),
            "local edit\n"
        );

        run(root, &["commit", "-am", "committed side"]);
        let tree = read(root, "c").await.unwrap();
        let reply = apply(
            root,
            &tree,
            &WorkingTreeAction::StashPop { oid: oid.clone() },
        )
        .await
        .unwrap();
        assert_eq!(reply.conflicts, ["tracked.txt"]);
        assert_eq!(stash_oids(root), [oid.clone()], "the stash is kept");

        let tree = read(root, "c").await.unwrap();
        let error = apply(root, &tree, &WorkingTreeAction::StashPop { oid })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("merge conflicts"), "{error}");
    }

    fn rev(cwd: &Path, name: &str) -> String {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(["rev-parse", "--verify", "-q", name])
            .output()
            .unwrap();
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn commit_file(cwd: &Path, name: &str, text: &str) {
        std::fs::write(cwd.join(name), text).unwrap();
        run(cwd, &["add", "--", name]);
        run(cwd, &["commit", "-q", "-m", name]);
    }

    fn lab() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let remote = dir.path().join("remote.git");
        let repo = dir.path().join("repo");
        run(
            dir.path(),
            &[
                "init",
                "-q",
                "--bare",
                "-b",
                "main",
                remote.to_str().unwrap(),
            ],
        );
        run(
            dir.path(),
            &["init", "-q", "-b", "main", repo.to_str().unwrap()],
        );
        run(
            &repo,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        commit_file(&repo, "a.txt", "a\n");
        run(&repo, &["push", "-q", "-u", "origin", "main"]);
        (dir, remote, repo)
    }

    const PUSH: SyncDirection = SyncDirection::Push {
        set_upstream: false,
    };
    const PUSH_TRACKING: SyncDirection = SyncDirection::Push { set_upstream: true };

    #[test]
    fn push_args_name_one_branch_ref_and_never_force() {
        let tracked = push_args("origin", "feature/x", "refs/heads/feature/x", false);
        assert_eq!(
            tracked,
            [
                "push",
                "--",
                "origin",
                "refs/heads/feature/x:refs/heads/feature/x"
            ]
        );
        let new = push_args("origin", "-odd", "refs/heads/-odd", true);
        assert_eq!(
            new,
            [
                "push",
                "--set-upstream",
                "--",
                "origin",
                "refs/heads/-odd:refs/heads/-odd"
            ]
        );
        for args in [tracked, new] {
            assert!(args.iter().all(|arg| {
                !matches!(
                    arg.as_str(),
                    "--force" | "-f" | "--no-verify" | "--mirror" | "--delete" | "--prune"
                ) && !arg.starts_with("--force")
                    && !arg.starts_with('+')
            }));
        }
        assert_eq!(
            PULL_ARGS,
            ["pull", "--ff-only", "--no-rebase", "--no-autostash"]
        );
    }

    #[test]
    fn the_push_remote_follows_tracking_then_push_default_then_origin_then_a_sole_remote() {
        let config = |text: &str, branch: &str| {
            BranchConfig::parse(text.replace('|', "\0").as_bytes(), branch)
        };
        let tracked = config(
            "remote.origin.url\n/r|remote.fork.url\n/f|branch.feature/x.remote\nfork|branch.feature/x.merge\nrefs/heads/x|branch.other.remote\norigin|",
            "feature/x",
        );
        assert_eq!(tracked.push_remote(true).as_deref(), Some("fork"));
        assert_eq!(tracked.merge.as_deref(), Some("refs/heads/x"));
        assert_eq!(tracked.push_remote(false).as_deref(), Some("origin"));
        let local = config("remote.origin.url\n/r|branch.b.remote\n.|", "b");
        assert_eq!(
            local.push_remote(true),
            None,
            "a local upstream is not a remote"
        );
        let pinned = config(
            "remote.origin.url\n/r|remote.mine.url\n/m|remote.pushdefault\nmine|",
            "b",
        );
        assert_eq!(pinned.push_remote(false).as_deref(), Some("mine"));
        let sole = config("remote.upstream.url\n/u|", "b");
        assert_eq!(sole.push_remote(false).as_deref(), Some("upstream"));
        let many = config("remote.a.url\n/a|remote.b.url\n/b|", "b");
        assert_eq!(many.push_remote(false), None, "never a guess among remotes");

        let triangular = config(
            "remote.origin.url\n/r|remote.fork.url\n/f|remote.pushdefault\norigin|push.default\nupstream|branch.Feat.remote\norigin|branch.Feat.pushremote\nfork|branch.Feat.merge\nrefs/heads/main|",
            "Feat",
        );
        assert_eq!(triangular.push_remote(true).as_deref(), Some("fork"));
        assert_eq!(triangular.push_remote(false).as_deref(), Some("fork"));
        assert_eq!(triangular.push_mode.as_deref(), Some("upstream"));
        let pinned_tracked = config(
            "remote.origin.url\n/r|remote.mine.url\n/m|remote.pushdefault\nmine|branch.b.remote\norigin|",
            "b",
        );
        assert_eq!(pinned_tracked.push_remote(true).as_deref(), Some("mine"));
        let unknown = config(
            "remote.origin.url\n/r|branch.b.remote\norigin|branch.b.pushremote\n/some/url|",
            "b",
        );
        assert_eq!(
            unknown.push_remote(true),
            None,
            "never fall back past a pushRemote"
        );
    }

    fn branch_from_origin_main(repo: &Path, name: &str) {
        run(repo, &["fetch", "-q", "origin"]);
        run(repo, &["switch", "-q", "-c", name, "origin/main"]);
        commit_file(repo, &format!("{name}.txt"), "x\n");
    }

    #[tokio::test]
    async fn push_never_sends_a_branch_to_an_upstream_with_another_name() {
        let (_dir, remote, repo) = lab();
        let main = rev(&remote, "refs/heads/main");
        branch_from_origin_main(&repo, "feat");
        let tree = read(&repo, "c").await.unwrap();
        assert_eq!(tree.upstream.as_deref(), Some("origin/main"));
        assert_eq!(tree.push_remote.as_deref(), Some("origin"));

        let error = sync(&repo, &tree, PUSH, SYNC_TIMEOUT).await.unwrap_err();
        let text = error.to_string();
        assert!(text.contains("feat tracks origin/main"), "{text}");
        assert!(text.contains("git push -u origin feat"), "{text}");
        assert_eq!(rev(&remote, "refs/heads/main"), main);
        assert_eq!(rev(&remote, "refs/heads/feat"), "");

        assert_eq!(
            sync(&repo, &tree, PUSH_TRACKING, SYNC_TIMEOUT)
                .await
                .unwrap(),
            1
        );
        assert_eq!(rev(&remote, "refs/heads/feat"), rev(&repo, "HEAD"));
        assert_eq!(rev(&remote, "refs/heads/main"), main);
        let tree = read(&repo, "c").await.unwrap();
        assert_eq!(tree.upstream.as_deref(), Some("origin/feat"));

        branch_from_origin_main(&repo, "current-mode");
        run(&repo, &["config", "push.default", "current"]);
        let tree = read(&repo, "c").await.unwrap();
        sync(&repo, &tree, PUSH, SYNC_TIMEOUT).await.unwrap();
        assert_eq!(rev(&remote, "refs/heads/current-mode"), rev(&repo, "HEAD"));
        assert_eq!(rev(&remote, "refs/heads/main"), main);

        branch_from_origin_main(&repo, "upstream-publish");
        run(&repo, &["config", "push.default", "upstream"]);
        let tree = read(&repo, "c").await.unwrap();
        sync(&repo, &tree, PUSH_TRACKING, SYNC_TIMEOUT)
            .await
            .unwrap();
        assert_eq!(
            rev(&remote, "refs/heads/upstream-publish"),
            rev(&repo, "HEAD")
        );
        assert_eq!(
            rev(&remote, "refs/heads/main"),
            main,
            "publishing under its own name never lands on the upstream"
        );
        assert_eq!(
            read(&repo, "c").await.unwrap().upstream.as_deref(),
            Some("origin/upstream-publish")
        );

        branch_from_origin_main(&repo, "upstream-mode");
        let tree = read(&repo, "c").await.unwrap();
        sync(&repo, &tree, PUSH, SYNC_TIMEOUT).await.unwrap();
        assert_eq!(
            rev(&remote, "refs/heads/main"),
            rev(&repo, "HEAD"),
            "push.default=upstream asked for the merge ref"
        );
    }

    #[tokio::test]
    async fn push_goes_to_the_branch_push_remote_under_its_own_name() {
        let (dir, origin, repo) = lab();
        let fork = dir.path().join("fork.git");
        run(
            dir.path(),
            &["init", "-q", "--bare", "-b", "main", fork.to_str().unwrap()],
        );
        run(&repo, &["remote", "add", "fork", fork.to_str().unwrap()]);
        let main = rev(&origin, "refs/heads/main");
        branch_from_origin_main(&repo, "feat");
        run(&repo, &["config", "branch.feat.pushRemote", "fork"]);
        let tree = read(&repo, "c").await.unwrap();
        assert_eq!(tree.push_remote.as_deref(), Some("fork"));

        sync(&repo, &tree, PUSH, SYNC_TIMEOUT).await.unwrap();
        assert_eq!(rev(&fork, "refs/heads/feat"), rev(&repo, "HEAD"));
        assert_eq!(rev(&fork, "refs/heads/main"), "");
        assert_eq!(rev(&origin, "refs/heads/main"), main);
        assert_eq!(rev(&origin, "refs/heads/feat"), "");
        assert_eq!(
            read(&repo, "c").await.unwrap().upstream.as_deref(),
            Some("origin/main"),
            "pull still comes from the tracked branch"
        );

        commit_file(&repo, "more.txt", "more\n");
        run(&repo, &["config", "push.default", "upstream"]);
        let tree = read(&repo, "c").await.unwrap();
        let error = sync(&repo, &tree, PUSH, SYNC_TIMEOUT).await.unwrap_err();
        assert!(
            error.to_string().contains("not the remote it tracks"),
            "{error}"
        );
        assert_eq!(rev(&origin, "refs/heads/main"), main);
    }

    #[tokio::test]
    async fn push_sends_a_new_branch_with_tracking_then_follow_up_commits() {
        let (_dir, remote, repo) = lab();
        run(&repo, &["checkout", "-q", "-b", "feature/x"]);
        commit_file(&repo, "b.txt", "b\n");
        commit_file(&repo, "c.txt", "c\n");
        let tree = read(&repo, "c").await.unwrap();
        assert_eq!(tree.upstream, None);
        assert_eq!(tree.push_remote.as_deref(), Some("origin"));
        assert_eq!(tree.ahead, 2, "commits not on origin yet");
        assert_eq!(tree.default_branch, None);

        let error = sync(&repo, &tree, PUSH, SYNC_TIMEOUT).await.unwrap_err();
        assert!(error.to_string().contains("not on a remote yet"), "{error}");
        assert_eq!(rev(&remote, "refs/heads/feature/x"), "");

        assert_eq!(
            sync(&repo, &tree, PUSH_TRACKING, SYNC_TIMEOUT)
                .await
                .unwrap(),
            2
        );
        assert_eq!(rev(&remote, "refs/heads/feature/x"), rev(&repo, "HEAD"));
        let tree = read(&repo, "c").await.unwrap();
        assert_eq!(tree.upstream.as_deref(), Some("origin/feature/x"));
        assert_eq!((tree.ahead, tree.behind), (0, 0));

        commit_file(&repo, "d.txt", "d\n");
        let tree = read(&repo, "c").await.unwrap();
        assert_eq!(tree.ahead, 1);
        assert_eq!(sync(&repo, &tree, PUSH, SYNC_TIMEOUT).await.unwrap(), 1);
        assert_eq!(rev(&remote, "refs/heads/feature/x"), rev(&repo, "HEAD"));
        assert_eq!(rev(&remote, "refs/heads/main"), rev(&repo, "main"));

        run(
            &repo,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        );
        assert_eq!(
            read(&repo, "c").await.unwrap().default_branch.as_deref(),
            Some("main")
        );
    }

    #[tokio::test]
    async fn a_rejected_push_and_a_diverged_pull_leave_both_sides_alone() {
        let (dir, remote, repo) = lab();
        let other = dir.path().join("other");
        run(
            dir.path(),
            &[
                "clone",
                "-q",
                remote.to_str().unwrap(),
                other.to_str().unwrap(),
            ],
        );
        commit_file(&other, "theirs.txt", "theirs\n");
        run(&other, &["push", "-q", "origin", "main"]);
        let theirs = rev(&other, "HEAD");
        commit_file(&repo, "mine.txt", "mine\n");
        let mine = rev(&repo, "HEAD");
        run(&repo, &["fetch", "-q", "origin"]);
        let tree = read(&repo, "c").await.unwrap();
        assert_eq!((tree.ahead, tree.behind), (1, 1));

        let error = sync(&repo, &tree, PUSH, SYNC_TIMEOUT).await.unwrap_err();
        let text = error.to_string();
        assert!(text.contains("Pull first"), "{text}");
        assert!(
            text.contains("[rejected]"),
            "git's own line is kept: {text}"
        );
        assert!(!text.contains("hint:"), "{text}");
        assert_eq!(rev(&remote, "refs/heads/main"), theirs);

        let error = sync(&repo, &tree, SyncDirection::Pull, SYNC_TIMEOUT)
            .await
            .unwrap_err();
        let text = error.to_string();
        assert!(text.contains("diverged"), "{text}");
        assert!(text.contains("Not possible to fast-forward"), "{text}");
        assert_eq!(rev(&repo, "HEAD"), mine);
        assert_eq!(rev(&remote, "refs/heads/main"), theirs);
    }

    #[tokio::test]
    async fn pull_fast_forwards_and_refuses_without_an_upstream_or_a_branch() {
        let (dir, remote, repo) = lab();
        let other = dir.path().join("other");
        run(
            dir.path(),
            &[
                "clone",
                "-q",
                remote.to_str().unwrap(),
                other.to_str().unwrap(),
            ],
        );
        commit_file(&other, "one.txt", "1\n");
        commit_file(&other, "two.txt", "2\n");
        run(&other, &["push", "-q", "origin", "main"]);
        std::fs::write(repo.join("local note.txt"), "keep\n").unwrap();
        let tree = read(&repo, "c").await.unwrap();
        assert_eq!(
            sync(&repo, &tree, SyncDirection::Pull, SYNC_TIMEOUT)
                .await
                .unwrap(),
            2
        );
        assert_eq!(rev(&repo, "HEAD"), rev(&other, "HEAD"));
        assert!(repo.join("local note.txt").exists());
        let tree = read(&repo, "c").await.unwrap();
        assert_eq!(
            sync(&repo, &tree, SyncDirection::Pull, SYNC_TIMEOUT)
                .await
                .unwrap(),
            0
        );

        run(&repo, &["checkout", "-q", "-b", "local-only"]);
        let tree = read(&repo, "c").await.unwrap();
        let error = sync(&repo, &tree, SyncDirection::Pull, SYNC_TIMEOUT)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("no upstream"), "{error}");

        let head = rev(&repo, "HEAD");
        run(&repo, &["checkout", "-q", "--detach", &head]);
        let tree = read(&repo, "c").await.unwrap();
        for direction in [PUSH_TRACKING, SyncDirection::Pull] {
            let error = sync(&repo, &tree, direction, SYNC_TIMEOUT)
                .await
                .unwrap_err();
            assert!(error.to_string().contains("detached"), "{error}");
        }
    }

    fn checkout_action(target: &str, detach: bool) -> WorkingTreeAction {
        WorkingTreeAction::Checkout {
            target_ref: target.into(),
            detach,
        }
    }

    fn branch_action(name: &str, at: Option<&str>, checkout: bool) -> WorkingTreeAction {
        WorkingTreeAction::CreateBranch {
            name: name.into(),
            start_point: at.map(str::to_owned),
            checkout,
        }
    }

    fn current_branch(root: &Path) -> String {
        output(root, &["branch", "--show-current"])
            .trim()
            .to_owned()
    }

    #[tokio::test]
    async fn checkout_needs_a_clean_tracked_tree_and_names_a_branch_or_a_commit() {
        let dir = identified_repo();
        let root = dir.path();
        let first = head(root);
        run(root, &["switch", "-q", "-c", "side"]);
        commit_file(root, "side.txt", "side\n");
        let side = head(root);
        run(root, &["switch", "-q", "main"]);

        std::fs::write(root.join("tracked.txt"), "edited\n").unwrap();
        let tree = read(root, "c").await.unwrap();
        for action in [checkout_action("side", false), checkout_action(&side, true)] {
            let error = apply(root, &tree, &action).await.unwrap_err();
            assert!(
                error.to_string().contains("uncommitted changes"),
                "{action:?}: {error}"
            );
        }
        run(root, &["add", "tracked.txt"]);
        let tree = read(root, "c").await.unwrap();
        let error = apply(root, &tree, &checkout_action("side", false))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("uncommitted changes"), "{error}");
        assert_eq!(
            (current_branch(root), head(root)),
            ("main".into(), first.clone())
        );
        run(root, &["reset", "-q", "--hard"]);

        std::fs::write(root.join("note.txt"), "untracked\n").unwrap();
        let tree = read(root, "c").await.unwrap();
        apply(root, &tree, &checkout_action("side", false))
            .await
            .unwrap();
        assert_eq!(
            (current_branch(root), head(root)),
            ("side".into(), side.clone())
        );
        assert!(root.join("note.txt").exists(), "untracked files ride along");

        let tree = read(root, "c").await.unwrap();
        for (action, expected) in [
            (checkout_action("side", false), "already checked out"),
            (checkout_action("nope", false), "no local branch named nope"),
            (checkout_action("-f", false), "not a valid branch name"),
            (checkout_action("a..b", false), "not a valid branch name"),
            (checkout_action("main", true), "not a commit id"),
            (checkout_action("--orphan", true), "not a commit id"),
            (checkout_action("deadbeef", true), "not in this repository"),
        ] {
            let error = apply(root, &tree, &action).await.unwrap_err();
            assert!(error.to_string().contains(expected), "{action:?}: {error}");
        }
        assert_eq!(current_branch(root), "side");

        apply(root, &tree, &checkout_action(&first, true))
            .await
            .unwrap();
        let tree = read(root, "c").await.unwrap();
        assert_eq!(
            (tree.branch.as_deref(), tree.head_sha.as_deref()),
            (None, Some(first.as_str()))
        );
        let error = apply(root, &tree, &checkout_action(&first, true))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("already detached"), "{error}");
        apply(root, &tree, &checkout_action("main", false))
            .await
            .unwrap();
        assert_eq!(current_branch(root), "main");
    }

    #[tokio::test]
    async fn create_branch_checks_the_name_and_moves_head_only_when_asked() {
        let dir = identified_repo();
        let root = dir.path();
        let first = head(root);
        commit_file(root, "second.txt", "2\n");
        let second = head(root);
        std::fs::write(root.join("tracked.txt"), "edited\n").unwrap();
        let tree = read(root, "c").await.unwrap();
        for name in [
            "",
            "-x",
            "a..b",
            "with space",
            "x.lock",
            "HEAD",
            "@{-1}",
            "tab\tname",
            "ends/",
        ] {
            let error = apply(root, &tree, &branch_action(name, None, false))
                .await
                .unwrap_err();
            assert!(
                error.to_string().contains("not a valid branch name"),
                "{name:?}: {error}"
            );
        }
        let error = apply(root, &tree, &branch_action("main", None, false))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("already exists"), "{error}");
        let error = apply(root, &tree, &branch_action("x", Some("HEAD~1"), false))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("not a commit id"), "{error}");

        apply(
            root,
            &tree,
            &branch_action("feature/old", Some(&first), false),
        )
        .await
        .unwrap();
        assert_eq!(rev(root, "refs/heads/feature/old"), first);
        assert_eq!(
            (current_branch(root), head(root)),
            ("main".into(), second.clone())
        );
        assert_eq!(porcelain(root), " M tracked.txt\n");
        assert_eq!(
            output(root, &["config", "--get", "branch.feature/old.merge"]),
            ""
        );

        let tree = read(root, "c").await.unwrap();
        let error = apply(root, &tree, &branch_action("moved", Some(&first), true))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("uncommitted changes"), "{error}");
        assert_eq!(rev(root, "refs/heads/moved"), "");

        apply(
            root,
            &tree,
            &branch_action("feature/here", Some(&second), true),
        )
        .await
        .unwrap();
        assert_eq!(current_branch(root), "feature/here");
        assert_eq!(porcelain(root), " M tracked.txt\n", "the edit stays");

        run(root, &["checkout", "-q", "--", "tracked.txt"]);
        let tree = read(root, "c").await.unwrap();
        apply(
            root,
            &tree,
            &branch_action("feature/back", Some(&first), true),
        )
        .await
        .unwrap();
        assert_eq!(
            (current_branch(root), head(root)),
            ("feature/back".into(), first)
        );
    }

    #[tokio::test]
    async fn cherry_pick_adds_the_commit_or_stops_on_conflicts_and_abort_restores() {
        let dir = identified_repo();
        let root = dir.path();
        run(root, &["switch", "-q", "-c", "side"]);
        commit_file(root, "side.txt", "side\n");
        let clean = head(root);
        commit_file(root, "tracked.txt", "side version\n");
        let clashing = head(root);
        run(root, &["switch", "-q", "-c", "merged", "main"]);
        commit_file(root, "m.txt", "m\n");
        run(
            root,
            &["merge", "-q", "--no-ff", "-m", "Merge side", "side"],
        );
        let merge = head(root);
        run(root, &["switch", "-q", "main"]);
        commit_file(root, "tracked.txt", "main version\n");
        let base = head(root);

        std::fs::write(root.join("tracked.txt"), "dirty\n").unwrap();
        let tree = read(root, "c").await.unwrap();
        let error = apply(
            root,
            &tree,
            &WorkingTreeAction::CherryPick { sha: clean.clone() },
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("uncommitted changes"), "{error}");
        run(root, &["checkout", "-q", "--", "tracked.txt"]);

        let tree = read(root, "c").await.unwrap();
        for (sha, expected) in [
            (merge.clone(), "merge commit"),
            (base.clone(), "already on this branch"),
            ("side".to_owned(), "not a commit id"),
        ] {
            let error = apply(
                root,
                &tree,
                &WorkingTreeAction::CherryPick { sha: sha.clone() },
            )
            .await
            .unwrap_err();
            assert!(error.to_string().contains(expected), "{sha}: {error}");
        }
        assert_eq!(head(root), base);

        let reply = apply(
            root,
            &tree,
            &WorkingTreeAction::CherryPick { sha: clean.clone() },
        )
        .await
        .unwrap();
        let picked = reply.commit.expect("the new commit is reported");
        assert_eq!(picked.sha, head(root));
        assert_eq!(
            (picked.branch.as_deref(), picked.subject.as_str()),
            (Some("main"), "side.txt")
        );
        assert_eq!(rev(root, "HEAD~1"), base);
        assert!(root.join("side.txt").exists());
        assert!(reply.conflicts.is_empty());
        assert_eq!(porcelain(root), "");

        let tree = read(root, "c").await.unwrap();
        let error = apply(root, &tree, &WorkingTreeAction::CherryPick { sha: clean })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("nothing to pick"), "{error}");
        assert_eq!(
            rev(root, "CHERRY_PICK_HEAD"),
            "",
            "an empty pick is not left open"
        );
        assert_eq!(porcelain(root), "");

        let before = head(root);
        let tree = read(root, "c").await.unwrap();
        let reply = apply(
            root,
            &tree,
            &WorkingTreeAction::CherryPick {
                sha: clashing.clone(),
            },
        )
        .await
        .unwrap();
        assert_eq!(reply.conflicts, ["tracked.txt"]);
        assert!(reply.commit.is_none());
        let tree = read(root, "c").await.unwrap();
        assert_eq!(tree.cherry_pick_head.as_deref(), Some(clashing.as_str()));
        for action in [
            checkout_action("side", false),
            WorkingTreeAction::CherryPick { sha: merge.clone() },
            branch_action("elsewhere", Some(&base), true),
        ] {
            let error = apply(root, &tree, &action).await.unwrap_err();
            assert!(
                error.to_string().contains("cherry-pick is in progress"),
                "{action:?}: {error}"
            );
        }

        apply(root, &tree, &WorkingTreeAction::AbortCherryPick)
            .await
            .unwrap();
        assert_eq!(head(root), before);
        assert_eq!(porcelain(root), "");
        assert_eq!(
            std::fs::read_to_string(root.join("tracked.txt")).unwrap(),
            "main version\n"
        );
        let tree = read(root, "c").await.unwrap();
        assert_eq!(tree.cherry_pick_head, None);
        let error = apply(root, &tree, &WorkingTreeAction::AbortCherryPick)
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("no cherry-pick to abort"),
            "{error}"
        );
    }
}
