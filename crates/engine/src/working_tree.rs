use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Weak};

use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use zeron_proto::{
    GitFileState, LineDelta, StashEntry, WorkingTree, WorkingTreeAction, WorkingTreeFile,
    WorkingTreeFileDiff,
};

use crate::EngineError;

pub(crate) const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
const OUTPUT_MAX_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_FILES: usize = 2000;
const MAX_UNTRACKED_COUNTED: usize = 200;
const MAX_UNTRACKED_BYTES: u64 = 1024 * 1024;
const MAX_STASHES: usize = 50;

pub(crate) struct GitOutput {
    pub stdout: Vec<u8>,
    pub truncated: bool,
}

pub(crate) async fn git(root: &Path, args: &[&str]) -> Result<GitOutput, EngineError> {
    git_with(root, args, OUTPUT_MAX_BYTES, false).await
}

async fn git_with(
    root: &Path,
    args: &[&str],
    max_bytes: usize,
    exit_one_ok: bool,
) -> Result<GitOutput, EngineError> {
    let mut cmd = tokio::process::Command::new("git");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.as_std_mut().creation_flags(0x08000000);
    }
    cmd.arg("--no-optional-locks")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_LITERAL_PATHSPECS", "1")
        .env("GIT_EDITOR", "true")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|e| EngineError::Other(format!("git spawn failed: {e}")))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| EngineError::Other("git stdout unavailable".into()))?;
    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut truncated = false;
    loop {
        let n = stdout
            .read(&mut buf)
            .await
            .map_err(|e| EngineError::Other(format!("git read failed: {e}")))?;
        if n == 0 {
            break;
        }
        let remaining = max_bytes.saturating_sub(out.len());
        if n > remaining {
            out.extend_from_slice(&buf[..remaining]);
            truncated = true;
            let _ = child.start_kill();
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    let output = child
        .wait_with_output()
        .await
        .map_err(|e| EngineError::Other(format!("git wait failed: {e}")))?;
    let accepted = output.status.success() || (exit_one_ok && output.status.code() == Some(1));
    if !accepted && !truncated {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = stderr.trim();
        return Err(EngineError::Other(if message.is_empty() {
            format!(
                "git {} exited {}",
                args.first().unwrap_or(&"?"),
                output.status
            )
        } else {
            format!("git: {message}")
        }));
    }
    Ok(GitOutput {
        stdout: out,
        truncated,
    })
}

#[derive(Debug, Default, PartialEq)]
struct Status {
    head_sha: Option<String>,
    branch: Option<String>,
    upstream: Option<String>,
    ahead: u32,
    behind: u32,
    files: Vec<WorkingTreeFile>,
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
    let mut deltas = HashMap::new();
    let mut tokens = bytes.split(|b| *b == 0);
    while let Some(token) = tokens.next() {
        let Ok(token) = std::str::from_utf8(token) else {
            continue;
        };
        let mut parts = token.splitn(3, '\t');
        let (Some(added), Some(deleted), Some(path)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let path = if path.is_empty() {
            let _old = tokens.next();
            match tokens.next().map(std::str::from_utf8) {
                Some(Ok(new)) => new,
                _ => continue,
            }
        } else {
            path
        };
        let binary = added == "-" || deleted == "-";
        deltas.insert(
            path.to_owned(),
            LineDelta {
                additions: added.parse().unwrap_or(0),
                deletions: deleted.parse().unwrap_or(0),
                binary,
            },
        );
    }
    deltas
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

fn revision(status: &Status) -> String {
    let mut hasher = Sha256::new();
    hasher.update(
        serde_json::to_vec(&(&status.head_sha, &status.branch, &status.files))
            .expect("working tree serialization"),
    );
    crate::repos::hex(&hasher.finalize())
}

pub(crate) async fn read(root: &Path, checkout_id: &str) -> Result<WorkingTree, EngineError> {
    let status_output = git(
        root,
        &[
            "status",
            "--porcelain=v2",
            "-z",
            "--branch",
            "--untracked-files=all",
        ],
    )
    .await?;
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
    let (unstaged, staged, stashes) =
        tokio::join!(git(root, &numstat), git(root, &cached), git(root, &stash));
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
    let counted_root = root.to_path_buf();
    let counts = tokio::task::spawn_blocking(move || {
        untracked
            .into_iter()
            .map(|(index, path)| (index, count_untracked(&counted_root, &path)))
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();
    for (index, delta) in counts {
        status.files[index].unstaged = delta;
    }
    Ok(WorkingTree {
        checkout_id: checkout_id.to_owned(),
        root: root.to_string_lossy().into_owned(),
        revision: revision(&status),
        branch: status.branch,
        head_sha: status.head_sha,
        upstream: status.upstream,
        ahead: status.ahead,
        behind: status.behind,
        truncated,
        agent_active: false,
        files: status.files,
        stashes: stashes
            .map(|out| parse_stashes(&out.stdout))
            .unwrap_or_default(),
    })
}

pub(crate) const WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(150);
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
) -> Result<(), EngineError> {
    match action {
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
        _ => Err(EngineError::Other(
            "this Git action is not available yet".into(),
        )),
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

    fn run(cwd: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(args)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
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
        assert!(error.to_string().contains("not available yet"), "{error}");
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
}
