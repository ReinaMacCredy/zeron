use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use zeron_proto::{Chat, ChatWorkTree, ToolCall, WorkTreeState};

use crate::EngineError;
use crate::fs_watch::{FsWatch, FsWatchHub};
use crate::repos::Repos;
use crate::workspace_host::WorkspaceHost;

const BASE_WATCH_DEBOUNCE: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Worktree {
    pub path: PathBuf,
    pub head: Option<String>,
    pub branch: Option<String>,
}

pub(crate) fn parse_worktree_list(listing: &str) -> Vec<Worktree> {
    let mut entries: Vec<Worktree> = Vec::new();
    for line in listing.lines().map(str::trim) {
        if let Some(path) = line.strip_prefix("worktree ") {
            entries.push(Worktree {
                path: canonical(Path::new(path)),
                head: None,
                branch: None,
            });
        } else if let Some(entry) = entries.last_mut() {
            if let Some(head) = line.strip_prefix("HEAD ") {
                entry.head = Some(head.to_owned());
            } else if let Some(branch) = line.strip_prefix("branch refs/heads/") {
                entry.branch = Some(branch.to_owned());
            }
        }
    }
    entries
}

pub(crate) fn worktree_holding<'a>(path: &Path, listing: &'a [Worktree]) -> Option<&'a Worktree> {
    listing
        .iter()
        .skip(1)
        .filter(|worktree| path.starts_with(&worktree.path))
        .max_by_key(|worktree| worktree.path.components().count())
}

pub(crate) fn worktree_made_by<'a>(
    command: &str,
    before: &[Worktree],
    after: &'a [Worktree],
) -> Option<&'a Worktree> {
    after.iter().skip(1).find(|worktree| {
        !before.iter().any(|old| old.path == worktree.path)
            && worktree
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| !name.is_empty() && command.contains(name))
    })
}

fn in_unlisted_worktree(path: &Path, listing: &[Worktree]) -> bool {
    worktree_holding(path, listing).is_none()
        && path.ancestors().any(|dir| dir.join(".git").is_file())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BranchFacts {
    pub ahead: u32,
    pub moved_since_creation: Option<bool>,
}

pub(crate) fn classify(
    branch: Option<&str>,
    base: Option<&str>,
    facts: BranchFacts,
    previous: Option<WorkTreeState>,
) -> WorkTreeState {
    let Some(branch) = branch else {
        return WorkTreeState::Detached;
    };
    if base == Some(branch) {
        return WorkTreeState::Fresh;
    }
    if facts.ahead > 0 {
        return WorkTreeState::Ahead {
            commits: facts.ahead,
        };
    }
    let had_own_commits = facts.moved_since_creation.unwrap_or(matches!(
        previous,
        Some(WorkTreeState::Ahead { .. } | WorkTreeState::Merged)
    ));
    if had_own_commits {
        WorkTreeState::Merged
    } else {
        WorkTreeState::Fresh
    }
}

fn canonical(path: &Path) -> PathBuf {
    if let Ok(path) = std::fs::canonicalize(path) {
        return path;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) if parent != path => canonical(parent).join(name),
        _ => path.to_path_buf(),
    }
}

fn written_path(call: &ToolCall) -> Option<&str> {
    match call {
        ToolCall::WriteFile { path, .. } | ToolCall::EditFile { path, .. } => Some(path),
        ToolCall::ApplyPatch { path } => path.as_deref(),
        _ => None,
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Clone)]
pub struct WorkTrees {
    inner: Arc<Inner>,
}

struct Inner {
    repos: Repos,
    workspace: WorkspaceHost,
    device_id: String,
    listings: Mutex<HashMap<PathBuf, Vec<Worktree>>>,
    refreshing: Mutex<HashMap<PathBuf, bool>>,
    shell_starts: Mutex<HashMap<String, Vec<Worktree>>>,
    base_watches: Mutex<HashMap<PathBuf, Vec<FsWatch>>>,
}

impl WorkTrees {
    pub fn start(repos: Repos, workspace: WorkspaceHost, device_id: &str) -> Self {
        let work_trees = Self {
            inner: Arc::new(Inner {
                repos,
                workspace,
                device_id: device_id.to_string(),
                listings: Mutex::new(HashMap::new()),
                refreshing: Mutex::new(HashMap::new()),
                shell_starts: Mutex::new(HashMap::new()),
                base_watches: Mutex::new(HashMap::new()),
            }),
        };
        let boot = work_trees.clone();
        tokio::spawn(async move { boot.refresh_all().await });
        work_trees
    }

    pub fn note_tool_call(&self, chat_id: &str, call: &ToolCall, finished: bool) {
        let shell = matches!(call, ToolCall::Exec { .. });
        if !finished && !shell {
            return;
        }
        let this = self.clone();
        let chat_id = chat_id.to_string();
        let call = call.clone();
        tokio::spawn(async move {
            if let Err(error) = this.on_tool_call(&chat_id, &call, finished).await {
                tracing::debug!(chat = %chat_id, %error, "work trees: tool call not attributed");
            }
        });
    }

    pub fn note_checkout_changed(&self, root: &Path) {
        let repos: Vec<PathBuf> = lock(&self.inner.listings)
            .iter()
            .filter(|(_, listing)| listing.iter().any(|worktree| worktree.path == root))
            .map(|(main, _)| main.clone())
            .collect();
        for main in repos {
            let this = self.clone();
            tokio::spawn(async move { this.refresh_repo(&main).await });
        }
    }

    pub async fn select(
        &self,
        chat_id: &str,
        path: Option<&str>,
    ) -> Result<Option<ChatWorkTree>, EngineError> {
        let chat = self
            .inner
            .workspace
            .chat(chat_id)?
            .ok_or_else(|| EngineError::Other(format!("no chat {chat_id}")))?;
        let Some(path) = path else {
            self.inner.workspace.set_chat_work_tree(chat_id, None)?;
            return Ok(None);
        };
        let main = self
            .main_folder(&chat)
            .ok_or_else(|| EngineError::Other("the chat has no folder".into()))?;
        let listing = self.list(&main).await?;
        let wanted = canonical(Path::new(path));
        let worktree = listing
            .iter()
            .skip(1)
            .find(|worktree| worktree.path == wanted)
            .ok_or_else(|| EngineError::Other(format!("{path} is not a worktree of this project")))?
            .clone();
        let work_tree = self
            .observe(&listing, &worktree, chat.work_tree.as_ref())
            .await;
        self.inner
            .workspace
            .set_chat_work_tree(chat_id, Some(&work_tree))?;
        Ok(Some(work_tree))
    }

    pub async fn annotate_refs(
        &self,
        repo_path: &Path,
        refs: &mut [zeron_proto::RepoRef],
    ) -> Result<(), EngineError> {
        if refs.iter().all(|row| row.worktree_path.is_none()) {
            return Ok(());
        }
        let listing = self.list(&canonical(repo_path)).await?;
        let observed = futures::future::join_all(
            listing
                .iter()
                .skip(1)
                .filter(|worktree| worktree.branch.is_some())
                .map(|worktree| self.observe(&listing, worktree, None)),
        )
        .await;
        for row in refs {
            let Some(path) = row.worktree_path.as_deref() else {
                continue;
            };
            let path = canonical(Path::new(path)).to_string_lossy().into_owned();
            if let Some(work_tree) = observed.iter().find(|work_tree| work_tree.path == path) {
                row.work_tree_state = Some(work_tree.state);
                row.dirty_files = work_tree.dirty_files;
            }
        }
        Ok(())
    }

    async fn on_tool_call(
        &self,
        chat_id: &str,
        call: &ToolCall,
        finished: bool,
    ) -> Result<(), EngineError> {
        let Some(chat) = self.inner.workspace.chat(chat_id)? else {
            return Ok(());
        };
        if chat.device_id != self.inner.device_id {
            return Ok(());
        }
        let Some(main) = self.main_folder(&chat) else {
            return Ok(());
        };
        if !finished {
            // Per-chat baseline: another chat's shell may re-list the shared cache meanwhile.
            let known = lock(&self.inner.listings).get(&main).cloned();
            let before = match known {
                Some(listing) => listing,
                None => self.list(&main).await?,
            };
            lock(&self.inner.shell_starts)
                .entry(chat_id.to_string())
                .or_insert(before);
            return Ok(());
        }
        let attributed = match call {
            ToolCall::Exec { command } => {
                let before = lock(&self.inner.shell_starts)
                    .remove(chat_id)
                    .or_else(|| lock(&self.inner.listings).get(&main).cloned());
                let after = self.list(&main).await?;
                before.and_then(|before| worktree_made_by(command, &before, &after).cloned())
            }
            call => match written_path(call) {
                Some(path) => {
                    let path = match chat.cwd.as_deref() {
                        Some(cwd) => Path::new(cwd).join(path),
                        None => PathBuf::from(path),
                    };
                    let path = canonical(&path);
                    let known = lock(&self.inner.listings).get(&main).cloned();
                    let listing = match known {
                        Some(listing) if !in_unlisted_worktree(&path, &listing) => listing,
                        _ => self.list(&main).await?,
                    };
                    worktree_holding(&path, &listing).cloned()
                }
                None => None,
            },
        };
        if let Some(worktree) = attributed {
            let own_folder = chat.cwd.as_deref().map(|cwd| canonical(Path::new(cwd)));
            let current = chat.work_tree.as_ref().filter(|current| !current.removed);
            let moved =
                current.is_none_or(|current| canonical(Path::new(&current.path)) != worktree.path);
            if own_folder.as_ref() != Some(&worktree.path) && moved {
                let listing = lock(&self.inner.listings)
                    .get(&main)
                    .cloned()
                    .unwrap_or_default();
                let work_tree = self.observe(&listing, &worktree, None).await;
                self.inner
                    .workspace
                    .set_chat_work_tree(chat_id, Some(&work_tree))?;
            }
        }
        let linked = lock(&self.inner.listings)
            .get(&main)
            .is_some_and(|listing| listing.len() > 1);
        let attributed_anywhere = self
            .inner
            .workspace
            .watch_chats()
            .borrow()
            .iter()
            .any(|chat| {
                chat.work_tree
                    .as_ref()
                    .is_some_and(|work_tree| !work_tree.removed)
            });
        if linked || attributed_anywhere {
            self.refresh_repo(&main).await;
        }
        Ok(())
    }

    async fn refresh_all(&self) {
        let Ok(chats) = self.inner.workspace.read_chats() else {
            return;
        };
        let mut mains: Vec<PathBuf> = chats
            .iter()
            .filter(|chat| chat.device_id == self.inner.device_id && chat.work_tree.is_some())
            .filter_map(|chat| self.main_folder(chat))
            .collect();
        mains.sort();
        mains.dedup();
        for main in mains {
            self.refresh_repo(&main).await;
        }
    }

    async fn refresh_repo(&self, main: &Path) {
        {
            let mut refreshing = lock(&self.inner.refreshing);
            if let Some(again) = refreshing.get_mut(main) {
                *again = true;
                return;
            }
            refreshing.insert(main.to_path_buf(), false);
        }
        loop {
            if let Err(error) = self.refresh_pass(main).await {
                tracing::debug!(repo = %main.display(), %error, "work trees: refresh failed");
            }
            let mut refreshing = lock(&self.inner.refreshing);
            if refreshing.get(main).copied().unwrap_or(false) {
                refreshing.insert(main.to_path_buf(), false);
                continue;
            }
            refreshing.remove(main);
            break;
        }
    }

    async fn refresh_pass(&self, main: &Path) -> Result<(), EngineError> {
        let chats: Vec<Chat> = self
            .inner
            .workspace
            .read_chats()?
            .into_iter()
            .filter(|chat| {
                chat.device_id == self.inner.device_id
                    && chat.work_tree.is_some()
                    && self.main_folder(chat).as_deref() == Some(main)
            })
            .collect();
        let live = chats.iter().any(|chat| {
            chat.work_tree
                .as_ref()
                .is_some_and(|work_tree| !work_tree.removed)
        });
        self.watch_base(main, live).await;
        if chats.is_empty() {
            return Ok(());
        }
        let listing = self.list(main).await?;
        let mut observed: HashMap<PathBuf, ChatWorkTree> = HashMap::new();
        for chat in chats {
            let Some(current) = chat.work_tree.as_ref() else {
                continue;
            };
            let path = canonical(Path::new(&current.path));
            let next = match listing
                .iter()
                .skip(1)
                .find(|worktree| worktree.path == path)
            {
                Some(worktree) => match observed.get(&path) {
                    Some(work_tree) => work_tree.clone(),
                    None => {
                        let work_tree = self.observe(&listing, worktree, Some(current)).await;
                        observed.insert(path, work_tree.clone());
                        work_tree
                    }
                },
                None => ChatWorkTree {
                    removed: true,
                    ..current.clone()
                },
            };
            if &next != current {
                self.inner
                    .workspace
                    .set_chat_work_tree(&chat.id, Some(&next))?;
            }
        }
        Ok(())
    }

    // A merge into base from a shell outside Zeron touches no worktree that diff-sync watches.
    async fn watch_base(&self, main: &Path, wanted: bool) {
        if !wanted {
            lock(&self.inner.base_watches).remove(main);
            return;
        }
        if lock(&self.inner.base_watches).contains_key(main) {
            return;
        }
        let git_dir = main.join(".git");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let Ok(watches) = tokio::task::spawn_blocking(move || {
            if !git_dir.is_dir() {
                return Vec::new();
            }
            let hub = FsWatchHub::global();
            [
                (git_dir.clone(), notify::RecursiveMode::NonRecursive),
                (git_dir.join("refs/heads"), notify::RecursiveMode::Recursive),
            ]
            .into_iter()
            .map(|(path, mode)| {
                let tx = tx.clone();
                hub.watch(&path, mode, move |event| {
                    if !matches!(event.kind, notify::EventKind::Access(_)) {
                        let _ = tx.send(());
                    }
                })
            })
            .collect()
        })
        .await
        else {
            return;
        };
        lock(&self.inner.base_watches).insert(main.to_path_buf(), watches);
        let inner = Arc::downgrade(&self.inner);
        let main = main.to_path_buf();
        tokio::spawn(async move {
            while rx.recv().await.is_some() {
                while let Ok(Some(())) = tokio::time::timeout(BASE_WATCH_DEBOUNCE, rx.recv()).await
                {
                }
                let Some(inner) = inner.upgrade() else {
                    return;
                };
                WorkTrees { inner }.refresh_boxed(main.clone()).await;
            }
        });
    }

    fn refresh_boxed(self, main: PathBuf) -> futures::future::BoxFuture<'static, ()> {
        Box::pin(async move { self.refresh_repo(&main).await })
    }

    async fn observe(
        &self,
        listing: &[Worktree],
        worktree: &Worktree,
        previous: Option<&ChatWorkTree>,
    ) -> ChatWorkTree {
        let main = listing.first();
        let base = main.and_then(|main| main.branch.clone().or_else(|| main.head.clone()));
        let facts = match (&worktree.branch, &base, main) {
            (Some(branch), Some(base), Some(main)) if branch != base => {
                self.branch_facts(&main.path, base, branch).await
            }
            _ => BranchFacts {
                ahead: 0,
                moved_since_creation: Some(false),
            },
        };
        let path = worktree.path.to_string_lossy().into_owned();
        let previous =
            previous.filter(|previous| canonical(Path::new(&previous.path)) == worktree.path);
        ChatWorkTree {
            state: classify(
                worktree.branch.as_deref(),
                base.as_deref(),
                facts,
                previous.map(|previous| previous.state),
            ),
            dirty_files: self.dirty_files(&worktree.path).await,
            branch: worktree.branch.clone(),
            path,
            removed: false,
        }
    }

    async fn branch_facts(&self, main: &Path, base: &str, branch: &str) -> BranchFacts {
        let range = format!("{base}..refs/heads/{branch}");
        let ahead = self
            .inner
            .repos
            .git(&["rev-list", "--count", &range], Some(main))
            .await
            .ok()
            .and_then(|count| count.parse().ok())
            .unwrap_or(0);
        let moved_since_creation = if ahead > 0 {
            Some(true)
        } else {
            let reflog = self
                .inner
                .repos
                .git(
                    &[
                        "reflog",
                        "show",
                        "--format=%H %gs",
                        &format!("refs/heads/{branch}"),
                    ],
                    Some(main),
                )
                .await
                .unwrap_or_default();
            let tip = reflog
                .lines()
                .next()
                .and_then(|line| line.split(' ').next());
            let created = reflog
                .lines()
                .last()
                .filter(|line| line.contains("branch: Created from"))
                .and_then(|line| line.split(' ').next());
            match (tip, created) {
                (Some(tip), Some(created)) => Some(tip != created),
                _ => None,
            }
        };
        BranchFacts {
            ahead,
            moved_since_creation,
        }
    }

    async fn dirty_files(&self, path: &Path) -> u32 {
        // No index write, or diff-sync's watch would loop on this probe.
        self.inner
            .repos
            .git(
                &["--no-optional-locks", "status", "--porcelain"],
                Some(path),
            )
            .await
            .map(|out| out.lines().filter(|line| !line.is_empty()).count() as u32)
            .unwrap_or(0)
    }

    async fn list(&self, main: &Path) -> Result<Vec<Worktree>, EngineError> {
        let out = self
            .inner
            .repos
            .git(&["worktree", "list", "--porcelain"], Some(main))
            .await?;
        let listing = parse_worktree_list(&out);
        lock(&self.inner.listings).insert(main.to_path_buf(), listing.clone());
        Ok(listing)
    }

    fn main_folder(&self, chat: &Chat) -> Option<PathBuf> {
        let space = chat
            .space_id
            .as_deref()
            .and_then(|id| self.inner.workspace.space(id).ok().flatten())
            .map(|space| space.path);
        space
            .or_else(|| chat.cwd.clone())
            .map(|path| canonical(Path::new(&path)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(ahead: u32, moved: Option<bool>) -> BranchFacts {
        BranchFacts {
            ahead,
            moved_since_creation: moved,
        }
    }

    #[test]
    fn classify_tells_fresh_from_merged_by_the_branch_history() {
        let ahead = WorkTreeState::Ahead { commits: 2 };
        assert_eq!(
            classify(None, Some("main"), facts(3, None), None),
            WorkTreeState::Detached
        );
        assert_eq!(
            classify(Some("main"), Some("main"), facts(0, Some(true)), None),
            WorkTreeState::Fresh
        );
        assert_eq!(
            classify(Some("feat/x"), Some("main"), facts(2, Some(true)), None),
            ahead
        );
        assert_eq!(
            classify(Some("feat/x"), Some("main"), facts(0, Some(true)), None),
            WorkTreeState::Merged
        );
        assert_eq!(
            classify(
                Some("feat/x"),
                Some("main"),
                facts(0, Some(false)),
                Some(ahead)
            ),
            WorkTreeState::Fresh,
            "a reflog that says the branch never moved beats the memory"
        );
        assert_eq!(
            classify(Some("feat/x"), Some("main"), facts(0, None), Some(ahead)),
            WorkTreeState::Merged
        );
        assert_eq!(
            classify(Some("feat/x"), Some("main"), facts(0, None), None),
            WorkTreeState::Fresh
        );
    }

    fn git(cwd: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@test")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@test")
            .output()
            .expect("git spawns");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    struct Lab {
        _dir: tempfile::TempDir,
        main: PathBuf,
        work_trees: WorkTrees,
        workspace: WorkspaceHost,
    }

    impl Lab {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let main = canonical(&dir.path().join("repo"));
            std::fs::create_dir_all(&main).unwrap();
            git(&main, &["init", "-q", "-b", "main"]);
            std::fs::write(main.join("a.txt"), "a\n").unwrap();
            git(&main, &["add", "."]);
            git(&main, &["commit", "-q", "-m", "first"]);
            let workspace = WorkspaceHost::open(
                Arc::new(zeron_sync::DocsStore::open(dir.path().join("store")).unwrap()),
                crate::workspace_host::WorkspaceHostConfig {
                    device_id: "dev".into(),
                    device_name: "Dev".into(),
                    platform: "macos".into(),
                    org_id: "org".into(),
                    user_id: "user".into(),
                    edge: None,
                    local_only: true,
                    settle_after_days: zeron_proto::SETTLE_AFTER_DAYS_DEFAULT,
                },
            )
            .unwrap();
            let path = main.to_string_lossy().into_owned();
            workspace
                .create_space("space", "dev", &path, None, true)
                .unwrap();
            for chat in ["one", "two"] {
                workspace
                    .create_chat(chat, Some("space"), Some("dev"), None, Some(path.clone()))
                    .unwrap();
            }
            let repos =
                Repos::with_worktrees_root(&dir.path().join("data"), "dev", dir.path().join("wt"));
            let work_trees = WorkTrees::start(repos, workspace.clone(), "dev");
            Lab {
                _dir: dir,
                main,
                work_trees,
                workspace,
            }
        }

        fn add_worktree(&self, name: &str) -> PathBuf {
            let path = self.main.join(".maestro/worktree").join(name);
            git(
                &self.main,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    &format!("feat/{name}"),
                    path.to_str().unwrap(),
                    "main",
                ],
            );
            canonical(&path)
        }

        async fn shell(&self, chat: &str, command: &str, run: impl FnOnce()) {
            let call = ToolCall::Exec {
                command: command.into(),
            };
            self.work_trees
                .on_tool_call(chat, &call, false)
                .await
                .unwrap();
            run();
            self.work_trees
                .on_tool_call(chat, &call, true)
                .await
                .unwrap();
        }

        async fn tool(&self, chat: &str, call: ToolCall) {
            self.work_trees
                .on_tool_call(chat, &call, true)
                .await
                .unwrap();
        }

        fn work_tree(&self, chat: &str) -> Option<ChatWorkTree> {
            self.workspace.chat(chat).unwrap().unwrap().work_tree
        }
    }

    fn write(path: &Path) -> ToolCall {
        ToolCall::WriteFile {
            path: path.to_string_lossy().into_owned(),
            content: None,
        }
    }

    #[tokio::test]
    async fn writes_and_named_new_worktrees_attribute_the_chat() {
        let lab = Lab::new();
        let x = lab.main.join(".maestro/worktree/x");

        let call_one = ToolCall::Exec {
            command: "git worktree add -b feat/x .maestro/worktree/x main".into(),
        };
        lab.work_trees
            .on_tool_call("one", &call_one, false)
            .await
            .unwrap();
        lab.shell("two", "cargo check", || {
            lab.add_worktree("x");
        })
        .await;
        lab.work_trees
            .on_tool_call("one", &call_one, true)
            .await
            .unwrap();
        let one = lab.work_tree("one").expect("the shell that named x");
        assert_eq!(Path::new(&one.path), canonical(&x));
        assert_eq!(one.branch.as_deref(), Some("feat/x"));
        assert_eq!(one.state, WorkTreeState::Fresh);
        assert_eq!(lab.work_tree("two"), None);

        lab.tool(
            "two",
            ToolCall::ReadFile {
                path: x.join("a.txt").to_string_lossy().into_owned(),
            },
        )
        .await;
        assert_eq!(lab.work_tree("two"), None, "reads never attribute");

        lab.tool("one", write(&lab.main.join("notes.txt"))).await;
        assert_eq!(
            lab.work_tree("one").map(|w| w.path),
            Some(one.path.clone()),
            "a write under the main folder keeps the worktree"
        );

        lab.tool("two", write(&x.join("b.txt"))).await;
        assert_eq!(lab.work_tree("two").map(|w| w.path), Some(one.path.clone()));

        let y = lab.add_worktree("y");
        lab.tool("one", write(&y.join("c.txt"))).await;
        assert_eq!(
            lab.work_tree("one").map(|w| PathBuf::from(w.path)),
            Some(y.clone()),
            "a write in another worktree moves the chat there"
        );

        lab.shell("two", "ls", || {
            git(
                &lab.main,
                &["worktree", "remove", "--force", y.to_str().unwrap()],
            );
        })
        .await;
        let gone = lab.work_tree("one").unwrap();
        assert!(gone.removed);
        assert_eq!(gone.branch.as_deref(), Some("feat/y"));
    }

    #[tokio::test]
    async fn state_follows_commits_merges_and_dirty_files() {
        let lab = Lab::new();
        let x = lab.add_worktree("x");
        lab.tool("one", write(&x.join("a.txt"))).await;
        assert_eq!(lab.work_tree("one").unwrap().state, WorkTreeState::Fresh);

        std::fs::write(x.join("a.txt"), "changed\n").unwrap();
        git(&x, &["commit", "-q", "-am", "change"]);
        lab.work_trees.refresh_repo(&lab.main).await;
        assert_eq!(
            lab.work_tree("one").unwrap().state,
            WorkTreeState::Ahead { commits: 1 }
        );

        std::fs::write(x.join("left.txt"), "dirty\n").unwrap();
        git(&lab.main, &["merge", "-q", "--no-edit", "feat/x"]);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while lab.work_tree("one").unwrap().state != WorkTreeState::Merged {
            assert!(
                tokio::time::Instant::now() < deadline,
                "a merge in the main folder refreshes the badge without a tool call"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let merged = lab.work_tree("one").unwrap();
        assert_eq!(merged.state, WorkTreeState::Merged);
        assert_eq!(merged.dirty_files, 1);

        let mut refs = vec![zeron_proto::RepoRef {
            name: "feat/x".into(),
            worktree_path: Some(x.to_string_lossy().into_owned()),
            ..Default::default()
        }];
        lab.work_trees
            .annotate_refs(&lab.main, &mut refs)
            .await
            .unwrap();
        assert_eq!(refs[0].work_tree_state, Some(WorkTreeState::Merged));
        assert_eq!(refs[0].dirty_files, 1);

        let picked = lab
            .work_trees
            .select("two", Some(x.to_str().unwrap()))
            .await
            .unwrap();
        assert_eq!(picked.map(|w| w.state), Some(WorkTreeState::Merged));
        lab.work_trees.select("two", None).await.unwrap();
        assert_eq!(lab.work_tree("two"), None);
    }
}
