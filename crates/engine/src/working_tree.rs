use std::collections::HashMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use zeron_proto::{GitFileState, LineDelta, StashEntry, WorkingTree, WorkingTreeFile};

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
        let remaining = OUTPUT_MAX_BYTES.saturating_sub(out.len());
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
    if !output.status.success() && !truncated {
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
