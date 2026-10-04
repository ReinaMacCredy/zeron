use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use gpui::{
    AnyElement, Context, Entity, ListAlignment, ListOffset, ListState, SharedString, Task, Window,
    div, list, prelude::*, px,
};
use zeron_proto::{
    ChangeRequestCommit, ChangeRequestDiffSlice, ChangeRequestFailure, GetChangeRequestDiffRequest,
};
use zeron_rpc::methods;

use super::{PullRequestKey, Tones, diff_stat, failure_message};
use crate::changes::{
    self, DiffCodeScrollContext, DiffCodeWidth, DiffHighlights, DiffMode, DiffRow, FileDiff,
    FileHorizontalState, FileStatus, LineKind,
};
use crate::icons::{self, icon};
use crate::popover::{self, Popup};
use crate::settings::widgets::text_tooltip;
use crate::state::AppState;
use crate::theme::Theme;

const COMMIT_PAGE_SIZE: usize = 10;
const TOOLBAR_HEIGHT: f32 = 40.0;
const FILE_HEADER_HEIGHT: f32 = 32.0;
const SEPARATOR_HEIGHT: f32 = 24.0;
const TREE_ROW_HEIGHT: f32 = 24.0;
const WITHHELD: &str =
    "The host withheld part of this diff — a binary file, or a change too large to inline.";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tier {
    Source,
    Test,
    Generated,
}

const GENERATED_FILE_NAMES: [&str; 8] = [
    "pnpm-lock.yaml",
    "yarn.lock",
    "package-lock.json",
    "bun.lockb",
    "Cargo.lock",
    "go.sum",
    "composer.lock",
    "Gemfile.lock",
];
const GENERATED_DIRECTORIES: [&str; 5] =
    ["__snapshots__", "__generated__", "dist", "build", "vendor"];
const TEST_DIRECTORIES: [&str; 3] = ["__tests__", "tests", "test"];
const MODULE_EXTENSIONS: [&str; 6] = [".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"];

fn tier(path: &str) -> Tier {
    let segments: Vec<&str> = path.split('/').collect();
    let (name, dirs) = segments.split_last().unwrap_or((&"", &[]));
    if GENERATED_FILE_NAMES.contains(name)
        || name.ends_with(".snap")
        || name.ends_with(".min.js")
        || name.ends_with(".min.css")
        || name.contains(".generated.")
        || dirs.iter().any(|dir| GENERATED_DIRECTORIES.contains(dir))
    {
        return Tier::Generated;
    }
    if name.contains(".test.")
        || name.contains(".spec.")
        || dirs.iter().any(|dir| TEST_DIRECTORIES.contains(dir))
    {
        return Tier::Test;
    }
    Tier::Source
}

fn strip_extension(path: &str) -> &str {
    match (path.rfind('.'), path.rfind('/')) {
        (Some(dot), Some(slash)) if dot > slash => &path[..dot],
        (Some(dot), None) => &path[..dot],
        _ => path,
    }
}

fn base_name(path: &str) -> &str {
    strip_extension(path.rsplit('/').next().unwrap_or(""))
}

fn resolve_relative(from: &str, specifier: &str) -> String {
    let mut segments: Vec<&str> = from.split('/').collect();
    segments.pop();
    for part in specifier.split('/') {
        match part {
            "." | "" => {}
            ".." => {
                segments.pop();
            }
            part => segments.push(part),
        }
    }
    segments.join("/")
}

/// `from "x"`, `import "x"` and `require("x")` (t3code's `IMPORT_SPECIFIER`), without a regex.
fn import_specifiers(line: &str) -> Vec<&str> {
    let bytes = line.as_bytes();
    let is_word = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    let skip_space = |mut at: usize| {
        while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        at
    };
    let mut found = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let word = ["from", "import", "require"]
            .into_iter()
            .find(|word| bytes[at..].starts_with(word.as_bytes()));
        let Some(word) = word.filter(|_| at == 0 || !is_word(bytes[at - 1])) else {
            at += 1;
            continue;
        };
        let mut next = skip_space(at + word.len());
        if word == "require" {
            if bytes.get(next) != Some(&b'(') {
                at += 1;
                continue;
            }
            next += 1;
        }
        next = skip_space(next);
        if bytes.get(next) == Some(&b'(') {
            next = skip_space(next + 1);
        }
        if matches!(bytes.get(next), Some(b'"' | b'\'')) {
            let start = next + 1;
            if let Some(len) = bytes[start..]
                .iter()
                .position(|b| matches!(b, b'"' | b'\''))
                && len > 0
            {
                found.push(&line[start..start + len]);
                at = start + len + 1;
                continue;
            }
        }
        at += 1;
    }
    found
}

/// Files in reading order (t3code `orderDiffFiles`): source with what it imports first, then the
/// tests in the order of the source they cover, then whatever a tool wrote.
fn order_files(files: Vec<FileDiff>) -> Vec<FileDiff> {
    let mut by_path: HashMap<String, FileDiff> = HashMap::new();
    for file in files {
        by_path.insert(file.path.clone(), file);
    }
    let mut paths: Vec<String> = by_path.keys().cloned().collect();
    paths.sort();
    let source: Vec<&String> = paths
        .iter()
        .filter(|path| tier(path) == Tier::Source)
        .collect();
    let mut by_module: HashMap<&str, &str> = HashMap::new();
    let mut by_base: HashMap<&str, &str> = HashMap::new();
    let mut ambiguous: HashSet<&str> = HashSet::new();
    for path in &source {
        by_module.insert(strip_extension(path), path);
        let base = base_name(path);
        if by_base.insert(base, path).is_some() {
            ambiguous.insert(base);
        }
    }
    for base in ambiguous {
        by_base.remove(base);
    }
    let imports: HashMap<&str, HashSet<&str>> = source
        .iter()
        .map(|path| {
            let file = &by_path[path.as_str()];
            let mut imported = HashSet::new();
            for line in file.hunks.iter().flat_map(|hunk| &hunk.lines) {
                if !matches!(line.kind, LineKind::Add | LineKind::Del) {
                    continue;
                }
                for specifier in import_specifiers(&line.text) {
                    let resolved = if specifier.starts_with('.') {
                        resolve_relative(path, specifier)
                    } else {
                        specifier.to_owned()
                    };
                    let resolved = MODULE_EXTENSIONS
                        .iter()
                        .find_map(|extension| resolved.strip_suffix(extension))
                        .unwrap_or(&resolved);
                    let target = by_module
                        .get(resolved)
                        .or_else(|| by_module.get(format!("{resolved}/index").as_str()))
                        .or_else(|| by_base.get(base_name(resolved)));
                    if let Some(target) = target.filter(|target| **target != path.as_str()) {
                        imported.insert(*target);
                    }
                }
            }
            (path.as_str(), imported)
        })
        .collect();
    let mut remaining: Vec<&str> = source.iter().map(|path| path.as_str()).collect();
    let mut ordered: Vec<String> = Vec::new();
    while !remaining.is_empty() {
        let ready = remaining
            .iter()
            .position(|path| {
                imports[path]
                    .iter()
                    .all(|dependency| !remaining.contains(dependency))
            })
            .unwrap_or(0);
        ordered.push(remaining.remove(ready).to_owned());
    }
    let mut positions: HashMap<&str, usize> = HashMap::new();
    for (index, path) in ordered.iter().enumerate() {
        positions.entry(base_name(path)).or_insert(index);
    }
    let tested = |path: &str| -> usize {
        let name = path.rsplit('/').next().unwrap_or("");
        let name = [".test.", ".spec."]
            .iter()
            .find_map(|marker| name.find(marker).map(|at| &name[..at]))
            .unwrap_or(name);
        positions
            .get(strip_extension(name))
            .copied()
            .unwrap_or(usize::MAX)
    };
    let mut tests: Vec<&String> = paths
        .iter()
        .filter(|path| tier(path) == Tier::Test)
        .collect();
    tests.sort_by(|left, right| tested(left).cmp(&tested(right)).then(left.cmp(right)));
    let generated = paths.iter().filter(|path| tier(path) == Tier::Generated);
    ordered
        .iter()
        .chain(tests)
        .chain(generated)
        .filter_map(|path| by_path.remove(path.as_str()))
        .collect()
}

/// `@@ -a,b +c,d @@` -> old start and count (count defaults to 1).
fn hunk_old_range(header: &str) -> Option<(u32, u32)> {
    let old = header.strip_prefix("@@ -")?.split(' ').next()?;
    let (start, count) = old.split_once(',').unwrap_or((old, "1"));
    Some((start.parse().ok()?, count.parse().ok()?))
}

/// Unchanged lines hidden above hunk `index` ("N unmodified lines" in the separator).
fn unmodified_before(file: &FileDiff, index: usize) -> u32 {
    let end = |range: Option<(u32, u32)>| match range {
        Some((start, 0)) => start,
        Some((start, count)) => start + count - 1,
        None => 0,
    };
    let Some((start, count)) = file
        .hunks
        .get(index)
        .and_then(|hunk| hunk_old_range(&hunk.header))
    else {
        return 0;
    };
    let first = if count == 0 { start + 1 } else { start };
    let previous = index
        .checked_sub(1)
        .map(|previous| end(hunk_old_range(&file.hunks[previous].header)))
        .unwrap_or(0);
    first.saturating_sub(previous + 1)
}

fn flatten(
    files: &[FileDiff],
    mode: DiffMode,
    collapsed: impl Fn(usize) -> bool,
) -> (Vec<DiffRow>, Vec<Range<usize>>) {
    let mut rows = Vec::new();
    let mut ranges = Vec::with_capacity(files.len());
    for (index, file) in files.iter().enumerate() {
        let start = rows.len();
        rows.push(DiffRow::FileHeader { file: index as u32 });
        if !collapsed(index) {
            rows.extend(
                changes::body_rows(index as u32, file, &[], None, mode)
                    .into_iter()
                    .filter(|row| match row {
                        DiffRow::Notice { .. } => false,
                        DiffRow::HunkHeader { hunk, .. } => {
                            unmodified_before(file, *hunk as usize) > 0
                        }
                        _ => true,
                    }),
            );
        }
        ranges.push(start..rows.len());
    }
    (rows, ranges)
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TreeEntry {
    Folder { path: String, label: String },
    File { index: usize, label: String },
}

#[derive(Default)]
struct TreeNode {
    folders: Vec<(String, TreeNode)>,
    files: Vec<(String, usize)>,
}

/// Rows of the file tree in diff order, empty directory chains flattened (`a/b`), and the
/// descendants of folded folders left out.
fn tree_rows(paths: &[&str], folded: &HashSet<String>) -> Vec<(usize, TreeEntry)> {
    let mut root = TreeNode::default();
    for (index, path) in paths.iter().enumerate() {
        let mut node = &mut root;
        let mut parts: Vec<&str> = path.split('/').collect();
        let name = parts.pop().unwrap_or_default();
        for part in parts {
            let at = match node.folders.iter().position(|(label, _)| label == part) {
                Some(at) => at,
                None => {
                    node.folders.push((part.to_owned(), TreeNode::default()));
                    node.folders.len() - 1
                }
            };
            node = &mut node.folders[at].1;
        }
        node.files.push((name.to_owned(), index));
    }
    fn walk(
        node: &TreeNode,
        prefix: &str,
        depth: usize,
        folded: &HashSet<String>,
        rows: &mut Vec<(usize, TreeEntry)>,
    ) {
        for (label, child) in &node.folders {
            let mut label = label.clone();
            let mut child = child;
            while child.files.is_empty() && child.folders.len() == 1 {
                let (next, grandchild) = &child.folders[0];
                label = format!("{label}/{next}");
                child = grandchild;
            }
            let path = if prefix.is_empty() {
                label.clone()
            } else {
                format!("{prefix}/{label}")
            };
            let open = !folded.contains(&path);
            rows.push((
                depth,
                TreeEntry::Folder {
                    path: path.clone(),
                    label,
                },
            ));
            if open {
                walk(child, &path, depth + 1, folded, rows);
            }
        }
        for (label, index) in &node.files {
            rows.push((
                depth,
                TreeEntry::File {
                    index: *index,
                    label: label.clone(),
                },
            ));
        }
    }
    let mut rows = Vec::new();
    walk(&root, "", 0, folded, &mut rows);
    rows
}

enum Highlight {
    Pending(#[allow(dead_code)] Task<()>),
    Ready(Option<Arc<DiffHighlights>>),
}

struct Slice {
    truncated: bool,
    raw: Option<String>,
}

pub(crate) struct PullRequestCode {
    state: Entity<AppState>,
    key: PullRequestKey,
    commits: Vec<ChangeRequestCommit>,
    commit: Option<ChangeRequestCommit>,
    commits_shown: usize,
    commit_menu: Popup<()>,
    slices: Vec<Slice>,
    next_cursor: Option<String>,
    started: bool,
    pending: bool,
    error: Option<String>,
    files: Vec<FileDiff>,
    omitted: HashMap<String, (u64, u64)>,
    horizontal: Vec<FileHorizontalState>,
    toggled: HashSet<String>,
    fold_all: bool,
    rows: Vec<DiffRow>,
    row_ranges: Vec<Range<usize>>,
    list: ListState,
    highlights: HashMap<String, Highlight>,
    mode: DiffMode,
    wrap: bool,
    tree_folded: HashSet<String>,
    selected: Option<String>,
    generation: u64,
    _fetch: Option<Task<()>>,
}

impl PullRequestCode {
    pub(super) fn new(
        state: Entity<AppState>,
        key: PullRequestKey,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = crate::settings::current(cx);
        Self {
            state,
            key,
            commits: Vec::new(),
            commit: None,
            commits_shown: COMMIT_PAGE_SIZE,
            commit_menu: Popup::default(),
            slices: Vec::new(),
            next_cursor: None,
            started: false,
            pending: false,
            error: None,
            files: Vec::new(),
            omitted: HashMap::new(),
            horizontal: Vec::new(),
            toggled: HashSet::new(),
            fold_all: false,
            rows: Vec::new(),
            row_ranges: Vec::new(),
            list: ListState::new(0, ListAlignment::Top, px(1024.0)),
            highlights: HashMap::new(),
            mode: DiffMode::from_split(settings.diff_split),
            wrap: settings.diff_wrap,
            tree_folded: HashSet::new(),
            selected: None,
            generation: 0,
            _fetch: None,
        }
    }

    /// Newest first; a scoped commit that left the pull request (force-push) drops the scope.
    pub(super) fn set_commits(&mut self, commits: &[ChangeRequestCommit], cx: &mut Context<Self>) {
        let mut commits = commits.to_vec();
        commits.sort_by(|left, right| right.committed_at.cmp(&left.committed_at));
        self.commits = commits;
        if let Some(commit) = &self.commit
            && !self.commits.iter().any(|known| known.oid == commit.oid)
        {
            self.choose_commit(None, cx);
        }
    }

    pub(super) fn show_commit(&mut self, oid: &str, cx: &mut Context<Self>) {
        self.started = true;
        let commit = self
            .commits
            .iter()
            .find(|commit| commit.oid == oid)
            .cloned();
        if commit.is_some() {
            self.choose_commit(commit, cx);
        } else if self.slices.is_empty() && !self.pending {
            self.fetch(None, cx);
        }
    }

    pub(super) fn ensure_loaded(&mut self, cx: &mut Context<Self>) {
        if !self.started {
            self.started = true;
            self.fetch(None, cx);
        }
    }

    pub(super) fn dismiss_overlay(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.commit_menu.is_open() || self.commit_menu.is_closing() {
            return false;
        }
        self.close_commit_menu(cx);
        true
    }

    fn reset(&mut self) {
        self.generation += 1;
        self.slices.clear();
        self.next_cursor = None;
        self.pending = false;
        self.error = None;
        self.files.clear();
        self.omitted.clear();
        self.horizontal.clear();
        self.toggled.clear();
        self.fold_all = false;
        self.highlights.clear();
        self.selected = None;
        self.commits_shown = COMMIT_PAGE_SIZE;
        self.rows.clear();
        self.row_ranges.clear();
        self.list.reset(0);
    }

    fn choose_commit(&mut self, commit: Option<ChangeRequestCommit>, cx: &mut Context<Self>) {
        self.close_commit_menu(cx);
        if self.commit.as_ref().map(|c| &c.oid) == commit.as_ref().map(|c| &c.oid) {
            return;
        }
        self.commit = commit;
        self.reset();
        self.fetch(None, cx);
    }

    fn fetch(&mut self, cursor: Option<String>, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.pending = true;
        self.error = None;
        let generation = self.generation;
        let params = serde_json::to_value(GetChangeRequestDiffRequest {
            host: self.key.host.clone(),
            repository: self.key.repository.clone(),
            number: self.key.number,
            cursor,
            commit: self.commit.as_ref().map(|commit| commit.oid.clone()),
        })
        .unwrap_or(serde_json::Value::Null);
        self._fetch = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::GET_CHANGE_REQUEST_DIFF, params)
                .await
                .map_err(|error| {
                    let message = error.to_string();
                    serde_json::from_str::<ChangeRequestFailure>(&message)
                        .map(|failure| failure_message(failure).to_string())
                        .unwrap_or(message)
                })
                .and_then(|value| {
                    serde_json::from_value::<ChangeRequestDiffSlice>(value)
                        .map_err(|error| error.to_string())
                });
            let parsed = match result {
                Ok(slice) => Ok(cx
                    .background_executor()
                    .spawn(async move {
                        let files = order_files(changes::parse_patch(&slice.patch));
                        (files, slice)
                    })
                    .await),
                Err(error) => Err(error),
            };
            this.update(cx, |code, cx| {
                if code.generation != generation {
                    return;
                }
                code.pending = false;
                match parsed {
                    Ok((files, slice)) => code.append(files, slice, cx),
                    Err(error) => code.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn append(
        &mut self,
        files: Vec<FileDiff>,
        slice: ChangeRequestDiffSlice,
        cx: &mut Context<Self>,
    ) {
        let raw = (files.is_empty() && !slice.patch.trim().is_empty()).then_some(slice.patch);
        self.slices.push(Slice {
            truncated: slice.truncated,
            raw,
        });
        self.next_cursor = slice.next_cursor;
        for stat in slice.omitted_file_stats {
            self.omitted
                .insert(stat.path, (stat.additions, stat.deletions));
        }
        let before = self.rows.len();
        for file in &files {
            if self.fold_all {
                self.toggled.remove(&file.path);
            }
            self.horizontal.push(FileHorizontalState::new(file));
        }
        self.files.extend(files);
        self.flatten_rows();
        self.list.splice(before..before, self.rows.len() - before);
        if self.list.item_count() != self.rows.len() + 1 {
            self.list.reset(self.rows.len() + 1);
        }
        cx.notify();
    }

    fn collapsed(&self, index: usize) -> bool {
        self.files
            .get(index)
            .is_some_and(|file| self.fold_all != self.toggled.contains(&file.path))
    }

    fn flatten_rows(&mut self) {
        let collapsed: Vec<bool> = (0..self.files.len()).map(|ix| self.collapsed(ix)).collect();
        let (rows, ranges) = flatten(&self.files, self.mode, |ix| collapsed[ix]);
        self.rows = rows;
        self.row_ranges = ranges;
    }

    fn reflatten_all(&mut self, cx: &mut Context<Self>) {
        self.flatten_rows();
        self.list.reset(self.rows.len() + 1);
        cx.notify();
    }

    fn toggle_file(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(path) = self.files.get(index).map(|file| file.path.clone()) else {
            return;
        };
        if !self.toggled.remove(&path) {
            self.toggled.insert(path);
        }
        let old = self.row_ranges[index].clone();
        self.flatten_rows();
        let new = self.row_ranges[index].clone();
        self.list.splice(old, new.len());
        cx.notify();
    }

    fn all_collapsed(&self) -> bool {
        !self.files.is_empty() && (0..self.files.len()).all(|ix| self.collapsed(ix))
    }

    fn toggle_all(&mut self, cx: &mut Context<Self>) {
        self.fold_all = !self.all_collapsed();
        self.toggled.clear();
        self.reflatten_all(cx);
    }

    fn set_mode(&mut self, mode: DiffMode, cx: &mut Context<Self>) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        crate::settings::update(crate::settings::SavePolicy::Immediate, cx, |settings| {
            settings.diff_split = mode.is_split();
        });
        self.reflatten_all(cx);
    }

    fn toggle_wrap(&mut self, cx: &mut Context<Self>) {
        self.wrap = !self.wrap;
        self.reflatten_all(cx);
    }

    fn toggle_tree(&mut self, cx: &mut Context<Self>) {
        let open = !crate::settings::current(cx).pull_requests_file_tree;
        crate::settings::update(crate::settings::SavePolicy::Immediate, cx, |settings| {
            settings.pull_requests_file_tree = open;
        });
        cx.notify();
    }

    fn reveal(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(path) = self.files.get(index).map(|file| file.path.clone()) else {
            return;
        };
        if self.collapsed(index) {
            self.toggle_file(index, cx);
        }
        self.selected = Some(path);
        if let Some(range) = self.row_ranges.get(index) {
            self.list.scroll_to(ListOffset {
                item_ix: range.start,
                offset_in_item: px(0.),
            });
        }
        cx.notify();
    }

    fn close_commit_menu(&mut self, cx: &mut Context<Self>) {
        if self.commit_menu.begin_close() {
            popover::reap_popup(cx, |code: &mut Self| &mut code.commit_menu);
            cx.notify();
        }
    }

    fn highlight(&mut self, index: usize, cx: &mut Context<Self>) -> Option<Arc<DiffHighlights>> {
        let file = self.files.get(index)?;
        if let Some(Highlight::Ready(highlights)) = self.highlights.get(&file.path) {
            return highlights.clone();
        }
        if self.highlights.contains_key(&file.path) {
            return None;
        }
        let path = file.path.clone();
        let Some(language) = zeron_syntax::language_for_path(&path) else {
            self.highlights.insert(path, Highlight::Ready(None));
            return None;
        };
        let file = file.clone();
        let generation = self.generation;
        let key = path.clone();
        let task = cx.spawn(async move |this, cx| {
            let highlights = cx
                .background_executor()
                .spawn(async move { changes::excerpt_highlights(&file, language).map(Arc::new) })
                .await;
            this.update(cx, |code, cx| {
                if code.generation == generation {
                    code.highlights.insert(key, Highlight::Ready(highlights));
                    cx.notify();
                }
            })
            .ok();
        });
        self.highlights.insert(path, Highlight::Pending(task));
        None
    }

    fn file_stat(&self, file: &FileDiff) -> (u64, u64) {
        match (file.additions, file.deletions, self.omitted.get(&file.path)) {
            (0, 0, Some(stat)) => *stat,
            (additions, deletions, _) => (additions as u64, deletions as u64),
        }
    }

    fn withheld(&self) -> bool {
        self.slices
            .iter()
            .any(|slice| slice.truncated || slice.raw.is_some())
    }

    fn render_row(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let Some(row) = self.rows.get(ix).copied() else {
            return self.render_footer(&theme, cx);
        };
        if let DiffRow::FileHeader { file } = row {
            return self.render_file_header(file as usize, false, &theme, cx);
        }
        let file_ix = row.file();
        let highlight = self.highlight(file_ix, cx);
        let Some(file) = self.files.get(file_ix) else {
            return gpui::Empty.into_any_element();
        };
        let code_width = if self.wrap {
            DiffCodeWidth::Wrapped
        } else {
            DiffCodeWidth::Scrollable(self.horizontal[file_ix].metrics(
                file,
                highlight.as_ref(),
                &theme,
                window.text_system(),
                crate::typography::generation(cx),
            ))
        };
        let code_scroll = DiffCodeScrollContext {
            handle: self.horizontal[file_ix].scroll.clone(),
            prefix: SharedString::from(format!("pr-code-row-{ix}")),
        };
        match row {
            DiffRow::HunkHeader { hunk, .. } => {
                separator_row(unmodified_before(file, hunk as usize), &theme)
            }
            DiffRow::Line { hunk, line, .. } => {
                let Some(line) = file
                    .hunks
                    .get(hunk as usize)
                    .and_then(|hunk| hunk.lines.get(line as usize))
                else {
                    return gpui::Empty.into_any_element();
                };
                let spans = highlight
                    .as_deref()
                    .map(|highlights| highlights.spans(line))
                    .unwrap_or(&[]);
                changes::diff_line_row(
                    line,
                    spans,
                    &theme,
                    changes::gutter_width(file),
                    code_width,
                    Some(code_scroll.slot("unified")),
                )
            }
            DiffRow::SplitLine {
                hunk, left, right, ..
            } => {
                let Some(lines) = file.hunks.get(hunk as usize).map(|hunk| &hunk.lines) else {
                    return gpui::Empty.into_any_element();
                };
                let gutter_px = changes::gutter_width(file);
                let left = left.and_then(|slot| lines.get(slot as usize));
                let right = right.and_then(|slot| lines.get(slot as usize));
                if let Some(line) = [left, right]
                    .into_iter()
                    .flatten()
                    .find(|line| line.kind == LineKind::Meta)
                {
                    return changes::meta_line_row(
                        &line.text,
                        &theme,
                        2.0 * (changes::ACCENT_BAR_WIDTH + gutter_px),
                    );
                }
                let cell = |line: Option<&changes::DiffLine>, old: bool| {
                    line.map(|line| {
                        changes::split_line_cell(
                            line,
                            if old { line.old_no } else { line.new_no },
                            changes::line_runs(line, highlight.as_deref(), &theme),
                            &theme,
                            gutter_px,
                            code_width,
                            Some(code_scroll.slot(if old { "old" } else { "new" })),
                        )
                        .into_any_element()
                    })
                    .unwrap_or_else(|| changes::split_filler().into_any_element())
                };
                changes::split_row(cell(left, true), cell(right, false), self.wrap, &theme)
                    .into_any_element()
            }
            DiffRow::BodyPad { .. } => div()
                .w_full()
                .h(px(changes::BODY_BOTTOM_PAD))
                .into_any_element(),
            _ => gpui::Empty.into_any_element(),
        }
    }

    fn render_footer(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let raw = self
            .next_cursor
            .is_none()
            .then(|| {
                self.slices
                    .iter()
                    .filter_map(|slice| slice.raw.clone())
                    .collect::<Vec<_>>()
            })
            .filter(|raw| !raw.is_empty());
        if let Some(raw) = raw {
            return div()
                .flex()
                .flex_col()
                .gap(px(16.0))
                .border_t_1()
                .border_color(theme.hairline(0.06))
                .px(px(16.0))
                .py(px(20.0))
                .children(raw.into_iter().map(|text| {
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(theme.text_muted)
                                .child("Unsupported diff format. Showing raw patch."),
                        )
                        .child(
                            div()
                                .font_family(theme.font_mono.clone())
                                .text_size(px(12.0))
                                .text_color(theme.text)
                                .child(text),
                        )
                }))
                .into_any_element();
        }
        if self.next_cursor.is_none() {
            return div().h(px(1.0)).into_any_element();
        }
        if self.error.is_none() && !self.pending {
            let cursor = self.next_cursor.clone();
            self.fetch(cursor, cx);
        }
        let footer = div()
            .py(px(8.0))
            .flex()
            .items_center()
            .justify_center()
            .gap(px(8.0))
            .text_size(px(12.0))
            .text_color(theme.text_muted);
        if self.error.is_some() {
            footer
                .child("The rest of this diff could not be loaded.")
                .child(
                    outline_button("pr-code-retry", "Retry", theme).on_click(cx.listener(
                        |code, _, _, cx| {
                            let cursor = code.next_cursor.clone();
                            code.fetch(cursor, cx);
                        },
                    )),
                )
                .into_any_element()
        } else {
            footer.child("Loading more files...").into_any_element()
        }
    }

    fn render_file_header(
        &self,
        index: usize,
        sticky: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(file) = self.files.get(index) else {
            return gpui::Empty.into_any_element();
        };
        let collapsed = self.collapsed(index);
        let tones = Tones::of(theme);
        let (glyph, tone) = match file.status {
            FileStatus::Added => (icons::LUCIDE_SQUARE_PLUS, tones.open),
            FileStatus::Deleted => (icons::LUCIDE_SQUARE_MINUS, theme.danger),
            FileStatus::Renamed => (icons::LUCIDE_SQUARE_ARROW_RIGHT, theme.accent),
            FileStatus::Modified => (icons::LUCIDE_SQUARE_DOT, theme.text_muted),
        };
        let (additions, deletions) = self.file_stat(file);
        let id = SharedString::from(format!(
            "pr-code-file-{}{index}",
            if sticky { "sticky-" } else { "" }
        ));
        let paint = changes::sticky_file_header_paint(theme);
        let bar = theme.text.opacity(0.24);
        div()
            .id(id.clone())
            .group(id.clone())
            .relative()
            .w_full()
            .h(px(FILE_HEADER_HEIGHT))
            .flex()
            .items_center()
            .gap(px(4.0))
            .pl(px(8.0))
            .pr(px(12.0))
            .bg(if sticky { paint.rest_bg } else { theme.bg })
            .when(sticky, |el| el.border_b_1().border_color(paint.border))
            .text_size(px(12.0))
            .cursor_pointer()
            .on_click(cx.listener(move |code, _, _, cx| code.toggle_file(index, cx)))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .w(px(3.0))
                    .invisible()
                    .group_hover(id.clone(), |style| style.visible())
                    .bg(bar),
            )
            .child(
                div()
                    .size(px(20.0))
                    .mr(px(4.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.0))
                    .child(
                        icon(if collapsed {
                            icons::LUCIDE_CHEVRON_RIGHT
                        } else {
                            icons::LUCIDE_CHEVRON_DOWN
                        })
                        .size(px(16.0))
                        .text_color(theme.text_muted),
                    ),
            )
            .child(icon(glyph).size(px(14.0)).flex_none().text_color(tone))
            .when_some(
                file.old_path
                    .clone()
                    .filter(|_| file.status == FileStatus::Renamed),
                |el, old| {
                    el.child(
                        div()
                            .ml(px(4.0))
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_color(theme.text_muted)
                            .child(old),
                    )
                    .child(
                        icon(icons::LUCIDE_ARROW_RIGHT)
                            .size(px(12.0))
                            .flex_none()
                            .text_color(theme.text_muted),
                    )
                },
            )
            .child(
                div()
                    .ml(px(4.0))
                    .min_w_0()
                    .flex_1()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_color(theme.text)
                    .group_hover(id, |style| style.underline())
                    .child(file.path.clone()),
            )
            .when(additions + deletions > 0, |el| {
                el.child(
                    div()
                        .flex_none()
                        .font_family(theme.font_mono.clone())
                        .text_size(px(11.0))
                        .child(diff_stat(additions, deletions, tones, theme)),
                )
            })
            .into_any_element()
    }

    fn render_sticky_header(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let top = self.list.logical_scroll_top();
        let sticky = changes::sticky_file_header(
            &self.row_ranges,
            top.item_ix,
            top.offset_in_item.as_f32(),
        )?;
        let next_header_y = sticky.next_header_row.and_then(|row| {
            let bounds = self.list.bounds_for_item(row)?;
            Some((bounds.origin.y - self.list.viewport_bounds().origin.y).as_f32())
        });
        let offset = next_header_y
            .map(|y| (y - FILE_HEADER_HEIGHT).min(0.0))
            .unwrap_or(0.0);
        let header = self.render_file_header(sticky.file_ix, true, theme, cx);
        let paint = changes::sticky_file_header_paint(theme);
        let header = match paint.frost_tint {
            Some(tint) => div().w_full().bg(tint).child(header).into_any_element(),
            None => header,
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
                    header,
                ))
                .into_any_element(),
        )
    }

    fn render_toolbar(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let tones = Tones::of(theme);
        let muted = theme.text_muted;
        let (additions, deletions) = self.files.iter().fold((0, 0), |(a, d), file| {
            (a + file.additions as u64, d + file.deletions as u64)
        });
        let count = self.files.len();
        let files = format!(
            "{count} {}{}",
            if count == 1 { "file" } else { "files" },
            if self.next_cursor.is_some() { "+" } else { "" }
        );
        let tree_open = crate::settings::current(cx).pull_requests_file_tree;
        let all_collapsed = self.all_collapsed();
        let split = self.mode.is_split();
        let left = div()
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(12.0))
            .when(!self.commits.is_empty(), |el| {
                el.child(self.render_commit_trigger(theme, cx))
            })
            .when(!self.slices.is_empty(), |el| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .min_w_0()
                        .child(div().flex_none().child(files))
                        .when(self.withheld(), |el| {
                            el.child(div().flex_none().text_color(muted.opacity(0.5)).child("·"))
                                .child(
                                    div()
                                        .id("pr-code-withheld")
                                        .aria_label("Some of this diff was not shown")
                                        .tooltip(text_tooltip(WITHHELD))
                                        .child(
                                            icon(icons::LUCIDE_TRIANGLE_ALERT)
                                                .size(px(14.0))
                                                .text_color(theme.warning),
                                        ),
                                )
                        }),
                )
            });
        let toggle = |id: &'static str, glyph: &'static str, label: &'static str, on: bool| {
            div()
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(label)
                .tooltip(text_tooltip(label))
                .size(px(28.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .cursor_pointer()
                .when(on, |el| el.bg(theme.ink(0.06)))
                .hover(|style| style.bg(theme.ink(0.06)))
                .child(
                    icon(glyph)
                        .size(px(14.0))
                        .text_color(if on { theme.text } else { muted }),
                )
        };
        let segment = |id: &'static str, glyph: &'static str, label: &'static str, on: bool| {
            div()
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(label)
                .h(px(24.0))
                .px(px(10.0))
                .flex()
                .items_center()
                .rounded(px(6.0))
                .cursor_pointer()
                .when(on, |el| el.bg(theme.ink(0.08)))
                .child(
                    icon(glyph)
                        .size(px(14.0))
                        .text_color(if on { theme.text } else { muted }),
                )
        };
        let right = div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.0))
            .when(additions + deletions > 0, |el| {
                el.child(
                    div()
                        .mr(px(4.0))
                        .child(diff_stat(additions, deletions, tones, theme)),
                )
            })
            .when(count > 0, |el| {
                let label = if all_collapsed {
                    "Expand all files"
                } else {
                    "Collapse all files"
                };
                el.child(
                    toggle(
                        "pr-code-fold-all",
                        if all_collapsed {
                            icons::LUCIDE_CHEVRONS_UP_DOWN
                        } else {
                            icons::LUCIDE_CHEVRONS_DOWN_UP
                        },
                        label,
                        false,
                    )
                    .on_click(cx.listener(|code, _, _, cx| code.toggle_all(cx))),
                )
            })
            .child(
                div()
                    .id("pr-code-layout")
                    .role(gpui::Role::Group)
                    .aria_label("Diff layout")
                    .flex()
                    .gap(px(2.0))
                    .p(px(2.0))
                    .rounded(px(8.0))
                    .bg(theme.ink(0.036))
                    .child(
                        segment(
                            "pr-code-stacked",
                            icons::LUCIDE_ROWS_3,
                            "Stacked diff view",
                            !split,
                        )
                        .on_click(
                            cx.listener(|code, _, _, cx| code.set_mode(DiffMode::Unified, cx)),
                        ),
                    )
                    .child(
                        segment(
                            "pr-code-split",
                            icons::LUCIDE_COLUMNS_2,
                            "Split diff view",
                            split,
                        )
                        .on_click(cx.listener(|code, _, _, cx| code.set_mode(DiffMode::Split, cx))),
                    ),
            )
            .child(
                toggle(
                    "pr-code-wrap",
                    icons::LUCIDE_TEXT_WRAP,
                    if self.wrap {
                        "Disable line wrapping"
                    } else {
                        "Enable line wrapping"
                    },
                    self.wrap,
                )
                .on_click(cx.listener(|code, _, _, cx| code.toggle_wrap(cx))),
            )
            .when(count > 0, |el| {
                el.child(
                    toggle(
                        "pr-code-tree",
                        icons::LUCIDE_FOLDER_TREE,
                        if tree_open {
                            "Hide file tree"
                        } else {
                            "Show file tree"
                        },
                        tree_open,
                    )
                    .on_click(cx.listener(|code, _, _, cx| code.toggle_tree(cx))),
                )
            });
        div()
            .h(px(TOOLBAR_HEIGHT))
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.0))
            .px(px(16.0))
            .border_b_1()
            .border_color(theme.hairline(0.06))
            .text_size(px(12.0))
            .text_color(muted)
            .child(left)
            .child(right)
            .into_any_element()
    }

    fn render_commit_trigger(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let label = self
            .commit
            .as_ref()
            .map(|commit| commit.headline.clone())
            .unwrap_or_else(|| "All commits".to_string());
        let trigger = div()
            .id("pr-code-scope")
            .role(gpui::Role::Button)
            .aria_label(SharedString::from(format!("Diff scope: {label}")))
            .relative()
            .h(px(24.0))
            .max_w(px(256.0))
            .flex()
            .items_center()
            .gap(px(4.0))
            .px(px(8.0))
            .rounded(px(6.0))
            .bg(theme.ink(0.06))
            .hover(|style| style.bg(theme.ink(0.09)))
            .cursor_pointer()
            .text_color(theme.text)
            .font_weight(gpui::FontWeight::MEDIUM)
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
                    .size(px(14.0))
                    .flex_none()
                    .text_color(theme.text_muted),
            )
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|code, _, _, _| code.commit_menu.note_trigger_press()),
            )
            .on_click(cx.listener(|code, _, _, cx| {
                if code.commit_menu.take_press_was_open() {
                    code.close_commit_menu(cx);
                } else {
                    code.commit_menu.open(());
                    cx.notify();
                }
            }));
        if !self.commit_menu.is_open() {
            return trigger.into_any_element();
        }
        let closing = self.commit_menu.closing_since();
        let menu = self.render_commit_menu(theme, cx);
        trigger
            .child(popover::anchored_menu_below(
                "pr-code-scope-menu",
                menu,
                closing,
            ))
            .into_any_element()
    }

    fn render_commit_menu(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let theme = &theme.for_popup();
        let selected = self.commit.as_ref().map(|commit| commit.oid.clone());
        let row = |id: SharedString, active: bool| {
            popover::menu_row(theme, active, id.clone())
                .id(id)
                .role(gpui::Role::MenuItemRadio)
        };
        let mut rows = vec![
            row("pr-code-scope-all".into(), selected.is_none())
                .aria_label("All commits")
                .on_click(cx.listener(|code, _, _, cx| code.choose_commit(None, cx)))
                .child("All commits")
                .into_any_element(),
        ];
        for commit in self.commits.iter().take(self.commits_shown) {
            let chosen = commit.clone();
            let short: String = commit.oid.chars().take(7).collect();
            rows.push(
                row(
                    SharedString::from(format!("pr-code-scope-{}", commit.oid)),
                    selected.as_deref() == Some(commit.oid.as_str()),
                )
                .aria_label(SharedString::from(commit.headline.clone()))
                .tooltip(text_tooltip(commit.headline.clone()))
                .on_click(
                    cx.listener(move |code, _, _, cx| code.choose_commit(Some(chosen.clone()), cx)),
                )
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(commit.headline.clone()),
                )
                .child(
                    div()
                        .ml_auto()
                        .flex_none()
                        .font_family(theme.font_mono.clone())
                        .text_size(px(12.0))
                        .text_color(theme.text_muted)
                        .child(short),
                )
                .into_any_element(),
            );
        }
        let left = self.commits.len().saturating_sub(self.commits_shown);
        if left > 0 {
            rows.push(
                row("pr-code-scope-more".into(), false)
                    .text_color(theme.text_muted)
                    .on_click(cx.listener(|code, _, _, cx| {
                        code.commits_shown += COMMIT_PAGE_SIZE;
                        cx.notify();
                    }))
                    .child(format!("Show more ({left} left)"))
                    .into_any_element(),
            );
        }
        popover::popover_card(theme)
            .w(px(320.0))
            .flex()
            .flex_col()
            .gap(px(popover::MENU_GAP))
            .on_mouse_down_out(cx.listener(|code, _, _, cx| code.close_commit_menu(cx)))
            .children(rows)
            .into_any_element()
    }

    fn render_tree(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let paths: Vec<&str> = self.files.iter().map(|file| file.path.as_str()).collect();
        let rows = tree_rows(&paths, &self.tree_folded);
        let has_folders = rows
            .iter()
            .any(|(_, entry)| matches!(entry, TreeEntry::Folder { .. }));
        let all_folded = has_folders
            && rows.iter().all(|(depth, entry)| {
                *depth > 0 || !matches!(entry, TreeEntry::Folder { path, .. } if !self.tree_folded.contains(path))
            });
        let selected = self.selected.clone();
        let header = div()
            .h(px(TOOLBAR_HEIGHT))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.0))
            .px(px(8.0))
            .border_b_1()
            .border_color(theme.hairline(0.06))
            .text_size(px(12.0))
            .text_color(theme.text_muted)
            .child(
                div()
                    .px(px(4.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child("Files"),
            )
            .child(div().ml_auto().child(self.files.len().to_string()))
            .when(has_folders, |el| {
                let label = if all_folded {
                    "Expand all folders"
                } else {
                    "Collapse all folders"
                };
                el.child(
                    div()
                        .id("pr-code-tree-fold")
                        .role(gpui::Role::Button)
                        .aria_label(label)
                        .tooltip(text_tooltip(label))
                        .size(px(24.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.0))
                        .cursor_pointer()
                        .hover(|style| style.bg(theme.ink(0.06)))
                        .on_click(cx.listener(move |code, _, _, cx| {
                            if all_folded {
                                code.tree_folded.clear();
                            } else {
                                let paths: Vec<&str> =
                                    code.files.iter().map(|file| file.path.as_str()).collect();
                                let folders: Vec<String> = tree_rows(&paths, &HashSet::new())
                                    .into_iter()
                                    .filter_map(|(_, entry)| match entry {
                                        TreeEntry::Folder { path, .. } => Some(path),
                                        TreeEntry::File { .. } => None,
                                    })
                                    .collect();
                                code.tree_folded.extend(folders);
                            }
                            cx.notify();
                        }))
                        .child(
                            icon(if all_folded {
                                icons::LUCIDE_CHEVRONS_UP_DOWN
                            } else {
                                icons::LUCIDE_CHEVRONS_DOWN_UP
                            })
                            .size(px(14.0))
                            .text_color(theme.text_muted),
                        ),
                )
            });
        let entries = rows.into_iter().enumerate().map(|(ix, (depth, entry))| {
            let base = div()
                .id(("pr-code-tree-row", ix))
                .h(px(TREE_ROW_HEIGHT))
                .flex_none()
                .flex()
                .items_center()
                .gap(px(6.0))
                .pl(px(8.0 + 12.0 * depth as f32))
                .pr(px(8.0))
                .rounded(px(5.0))
                .text_size(px(12.0))
                .cursor_pointer()
                .hover(|style| style.bg(theme.text.opacity(0.07)));
            match entry {
                TreeEntry::Folder { path, label } => {
                    let open = !self.tree_folded.contains(&path);
                    base.role(gpui::Role::TreeItem)
                        .aria_label(SharedString::from(label.clone()))
                        .text_color(theme.text_muted)
                        .on_click(cx.listener(move |code, _, _, cx| {
                            if !code.tree_folded.remove(&path) {
                                code.tree_folded.insert(path.clone());
                            }
                            cx.notify();
                        }))
                        .child(
                            icon(if open {
                                icons::LUCIDE_CHEVRON_DOWN
                            } else {
                                icons::LUCIDE_CHEVRON_RIGHT
                            })
                            .size(px(12.0))
                            .flex_none()
                            .text_color(theme.text_muted),
                        )
                        .child(
                            icon(icons::LUCIDE_FOLDER)
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
                }
                TreeEntry::File { index, label } => {
                    let file = &self.files[index];
                    let active = selected.as_deref() == Some(file.path.as_str());
                    let tone = match file.status {
                        FileStatus::Added => Tones::of(theme).open,
                        FileStatus::Deleted => theme.danger,
                        FileStatus::Renamed => theme.accent,
                        FileStatus::Modified => theme.text,
                    };
                    base.role(gpui::Role::TreeItem)
                        .aria_label(SharedString::from(label.clone()))
                        .when(active, |el| el.bg(theme.text.opacity(0.12)))
                        .text_color(tone)
                        .on_click(cx.listener(move |code, _, _, cx| code.reveal(index, cx)))
                        .child(div().w(px(12.0)).flex_none())
                        .child(
                            crate::file_icons::icon(
                                crate::file_icons::FileIconIdentity::file(&file.path),
                                theme.appearance,
                            )
                            .size(px(14.0))
                            .flex_none(),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .child(label),
                        )
                }
            }
        });
        let footer = self.next_cursor.is_some().then(|| {
            let (label, enabled) = match (&self.error, self.pending) {
                (Some(_), _) => ("Retry", true),
                (None, true) => ("Loading more files...", false),
                (None, false) => ("Load more files", true),
            };
            div()
                .flex_none()
                .p(px(8.0))
                .border_t_1()
                .border_color(theme.hairline(0.06))
                .child(
                    outline_button("pr-code-tree-more", label, theme)
                        .w_full()
                        .justify_center()
                        .when(!enabled, |el| el.opacity(0.5))
                        .when(enabled, |el| {
                            el.on_click(cx.listener(|code, _, _, cx| {
                                let cursor = code.next_cursor.clone();
                                code.fetch(cursor, cx);
                            }))
                        }),
                )
        });
        div()
            .w(gpui::relative(0.4))
            .max_w(px(320.0))
            .min_w(px(192.0))
            .flex_none()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(theme.hairline(0.06))
            .id("pr-code-tree-panel")
            .role(gpui::Role::Tree)
            .aria_label(SharedString::from(format!(
                "Pull request #{} files",
                self.key.number
            )))
            .child(header)
            .child(
                div()
                    .id("pr-code-tree")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(4.0))
                    .flex()
                    .flex_col()
                    .children(entries),
            )
            .children(footer)
            .into_any_element()
    }
}

impl Render for PullRequestCode {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let toolbar = self.render_toolbar(&theme, cx);
        let message = |text: String| {
            div()
                .px(px(16.0))
                .py(px(20.0))
                .text_size(px(14.0))
                .text_color(theme.text_muted)
                .child(text)
                .into_any_element()
        };
        let body: AnyElement = if self.slices.is_empty() {
            match &self.error {
                Some(error) => message(error.clone()),
                None => render_skeleton(&theme),
            }
        } else if self.files.is_empty()
            && self.next_cursor.is_none()
            && !self.slices.iter().any(|slice| slice.raw.is_some())
        {
            message(
                if self.commit.is_some() {
                    "This commit has no file changes."
                } else {
                    "This pull request has no file changes."
                }
                .to_string(),
            )
        } else {
            let sticky = self.render_sticky_header(&theme, cx);
            let tree = crate::settings::current(cx)
                .pull_requests_file_tree
                .then(|| self.render_tree(&theme, cx));
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .overflow_hidden()
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .overflow_hidden()
                        .child(
                            list(self.list.clone(), cx.processor(Self::render_row))
                                .size_full()
                                .with_sizing_behavior(gpui::ListSizingBehavior::Auto),
                        )
                        .children(sticky),
                )
                .children(tree)
                .into_any_element()
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(toolbar)
            .child(body)
    }
}

fn separator_row(count: u32, theme: &Theme) -> AnyElement {
    let rule = || div().flex_1().h(px(1.0)).bg(theme.hairline(0.08));
    div()
        .h(px(SEPARATOR_HEIGHT))
        .w_full()
        .flex()
        .items_center()
        .gap(px(8.0))
        .px(px(12.0))
        .text_size(px(11.0))
        .text_color(theme.text_muted.opacity(0.75))
        .child(rule())
        .child(format!(
            "{count} unmodified {}",
            if count == 1 { "line" } else { "lines" }
        ))
        .child(rule())
        .into_any_element()
}

fn outline_button(
    id: &'static str,
    label: &'static str,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .role(gpui::Role::Button)
        .aria_label(label)
        .h(px(24.0))
        .px(px(8.0))
        .flex()
        .items_center()
        .rounded(px(6.0))
        .border_1()
        .border_color(theme.hairline(0.12))
        .text_size(px(12.0))
        .text_color(theme.text)
        .cursor_pointer()
        .hover(|style| style.bg(theme.ink(0.05)))
        .child(label)
}

fn render_skeleton(theme: &Theme) -> AnyElement {
    let bar = theme.text_muted.opacity(0.15);
    let header = || {
        div()
            .h(px(32.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(12.0))
            .child(div().size(px(10.0)).rounded(px(2.0)).bg(bar))
            .child(div().size(px(20.0)).rounded(px(6.0)).bg(bar))
            .child(
                div()
                    .h(px(10.0))
                    .w(gpui::relative(0.5))
                    .max_w(px(256.0))
                    .rounded(px(4.0))
                    .bg(bar),
            )
            .child(div().flex_1())
            .child(div().h(px(12.0)).w(px(20.0)).rounded(px(4.0)).bg(bar))
            .child(div().h(px(12.0)).w(px(20.0)).rounded(px(4.0)).bg(bar))
    };
    div()
        .id("pr-code-loading")
        .role(gpui::Role::Status)
        .aria_label("Loading pull request diff...")
        .flex()
        .flex_col()
        .child(header())
        .child(
            div()
                .h(px(24.0))
                .flex()
                .items_center()
                .gap(px(8.0))
                .px(px(12.0))
                .child(div().flex_1().h(px(1.0)).bg(theme.hairline(0.06)))
                .child(div().h(px(10.0)).w(px(96.0)).rounded(px(4.0)).bg(bar))
                .child(div().flex_1().h(px(1.0)).bg(theme.hairline(0.06))),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .px(px(12.0))
                .py(px(8.0))
                .children([0.66, 0.8, 0.6].map(|width| {
                    div()
                        .flex()
                        .gap(px(8.0))
                        .child(div().h(px(10.0)).w(px(20.0)).rounded(px(4.0)).bg(bar))
                        .child(
                            div()
                                .h(px(10.0))
                                .w(gpui::relative(width))
                                .rounded(px(4.0))
                                .bg(bar),
                        )
                })),
        )
        .child(header())
        .child(header())
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, added: &[&str]) -> FileDiff {
        let mut patch = format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n");
        if !added.is_empty() {
            patch.push_str(&format!("@@ -0,0 +1,{} @@\n", added.len()));
            for line in added {
                patch.push_str(&format!("+{line}\n"));
            }
        }
        changes::parse_patch(&patch).remove(0)
    }

    fn order(files: Vec<FileDiff>) -> Vec<String> {
        order_files(files)
            .into_iter()
            .map(|file| file.path)
            .collect()
    }

    #[test]
    fn source_comes_before_tests_and_generated_files() {
        let source = ["src/app.ts", "src/dist.ts", "src/testing.ts"];
        let tests = [
            "src/__tests__/app.ts",
            "src/app.spec.tsx",
            "src/app.test.ts",
            "test/app.ts",
            "tests/helpers/app.ts",
        ];
        let generated = [
            "apps/web/package-lock.json",
            "dist/app.js",
            "packages/core/vendor/lib.js",
            "pnpm-lock.yaml",
            "public/app.min.js",
            "src/__snapshots__/app.ts",
            "src/api.generated.ts",
            "src/app.test.ts.snap",
        ];
        let mut input: Vec<&str> = generated
            .iter()
            .chain(&tests)
            .chain(&source)
            .copied()
            .collect();
        input.reverse();
        let expected: Vec<&str> = source
            .iter()
            .chain(&tests)
            .chain(&generated)
            .copied()
            .collect();
        assert_eq!(
            order(input.iter().map(|path| file(path, &[])).collect()),
            expected
        );
        assert!(order(Vec::new()).is_empty());
    }

    #[test]
    fn imports_order_source_files() {
        assert_eq!(
            order(vec![
                file("src/a.ts", &["import { b } from \"./b\";"]),
                file("src/b.ts", &["import { c } from \"./c\";"]),
                file("src/c.ts", &[]),
            ]),
            ["src/c.ts", "src/b.ts", "src/a.ts"]
        );
        assert_eq!(
            order(vec![
                file("src/ui/a.ts", &["import { b } from \"../lib/b\";"]),
                file("src/lib/b.ts", &[]),
            ]),
            ["src/lib/b.ts", "src/ui/a.ts"]
        );
        assert_eq!(
            order(vec![
                file("src/a.ts", &["import { b } from \"./lib\";"]),
                file("src/lib/index.ts", &[]),
            ]),
            ["src/lib/index.ts", "src/a.ts"]
        );
        assert_eq!(
            order(vec![
                file("src/a.ts", &["import { b } from \"~/lib/b\";"]),
                file("lib/b.ts", &[]),
            ]),
            ["lib/b.ts", "src/a.ts"]
        );
        assert_eq!(
            order(vec![
                file("src/a.ts", &["import { b } from \"~/somewhere/b\";"]),
                file("one/b.ts", &[]),
                file("two/b.ts", &[]),
            ]),
            ["one/b.ts", "src/a.ts", "two/b.ts"]
        );
        assert_eq!(
            order(vec![
                file("src/a.ts", &["import { b } from \"./b.js\";"]),
                file("src/b.ts", &[]),
            ]),
            ["src/b.ts", "src/a.ts"]
        );
        assert_eq!(
            order(vec![
                file(
                    "src/a.ts",
                    &["const b = require(\"./b\");", "import \"./c\";"]
                ),
                file("src/b.ts", &[]),
                file("src/c.ts", &[]),
            ]),
            ["src/b.ts", "src/c.ts", "src/a.ts"]
        );
        assert_eq!(
            order(vec![
                file("src/b.ts", &["import { a } from \"./a\";"]),
                file("src/a.ts", &["import { b } from \"./b\";"]),
            ]),
            ["src/a.ts", "src/b.ts"]
        );
    }

    #[test]
    fn tests_follow_the_source_they_cover() {
        assert_eq!(
            order(vec![
                file("pnpm-lock.yaml", &[]),
                file("src/a.test.ts", &[]),
                file("src/__tests__/b.ts", &[]),
                file("src/a.ts", &["import { b } from \"./b\";"]),
                file("src/b.ts", &[]),
                file("dist/a.js", &[]),
            ]),
            [
                "src/b.ts",
                "src/a.ts",
                "src/__tests__/b.ts",
                "src/a.test.ts",
                "dist/a.js",
                "pnpm-lock.yaml"
            ]
        );
        assert_eq!(
            order(vec![
                file("src/orphan.spec.ts", &[]),
                file("src/a.test.ts", &[]),
                file("src/a.ts", &[]),
            ]),
            ["src/a.ts", "src/a.test.ts", "src/orphan.spec.ts"]
        );
    }

    #[test]
    fn synthesized_header_only_sections_parse_as_files() {
        let files = changes::parse_patch(
            "diff --git a/src/before.rs b/src/after.rs\n\
             rename from src/before.rs\n\
             rename to src/after.rs\n\
             --- a/src/before.rs\n\
             +++ b/src/after.rs\n\
             diff --git a/logo.png b/logo.png\n\
             --- a/logo.png\n\
             +++ b/logo.png\n\
             diff --git a/new.rs b/new.rs\n\
             new file mode 100644\n\
             --- /dev/null\n\
             +++ b/new.rs\n\
             @@ -0,0 +1 @@\n\
             +fn new() {}\n\
             diff --git a/gone.rs b/gone.rs\n\
             deleted file mode 100644\n\
             --- a/gone.rs\n\
             +++ /dev/null\n\
             @@ -1 +0,0 @@\n\
             -fn gone() {}\n",
        );
        let summary: Vec<(&str, Option<&str>, FileStatus, usize)> = files
            .iter()
            .map(|file| {
                (
                    file.path.as_str(),
                    file.old_path.as_deref(),
                    file.status,
                    file.hunks.len(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (
                    "src/after.rs",
                    Some("src/before.rs"),
                    FileStatus::Renamed,
                    0
                ),
                ("logo.png", None, FileStatus::Modified, 0),
                ("new.rs", None, FileStatus::Added, 1),
                ("gone.rs", None, FileStatus::Deleted, 1),
            ]
        );
    }

    #[test]
    fn separators_count_hidden_lines_between_hunks() {
        let file = changes::parse_patch(
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n\
             @@ -1,2 +1,2 @@\n a\n-b\n+c\n\
             @@ -10,3 +10,3 @@\n x\n-y\n+z\n w\n\
             @@ -20,0 +21,1 @@\n+n\n",
        )
        .remove(0);
        assert_eq!(
            (0..3)
                .map(|ix| unmodified_before(&file, ix))
                .collect::<Vec<_>>(),
            [0, 7, 8]
        );
        let (rows, ranges) = flatten(std::slice::from_ref(&file), DiffMode::Unified, |_| false);
        let separators = rows
            .iter()
            .filter(|row| matches!(row, DiffRow::HunkHeader { .. }))
            .count();
        assert_eq!(separators, 2);
        assert_eq!(ranges, [0..rows.len()]);
        let (folded, _) = flatten(std::slice::from_ref(&file), DiffMode::Unified, |_| true);
        assert_eq!(folded, [DiffRow::FileHeader { file: 0 }]);
    }

    #[test]
    fn tree_keeps_diff_order_and_flattens_single_folder_chains() {
        let paths = [
            "crates/ui/src/a.rs",
            "crates/ui/src/b.rs",
            "README.md",
            "docs/x.md",
        ];
        let rows = tree_rows(&paths, &HashSet::new());
        let shown: Vec<(usize, String)> = rows
            .iter()
            .map(|(depth, entry)| {
                (
                    *depth,
                    match entry {
                        TreeEntry::Folder { label, .. } => format!("{label}/"),
                        TreeEntry::File { label, .. } => label.clone(),
                    },
                )
            })
            .collect();
        assert_eq!(
            shown,
            [
                (0, "crates/ui/src/".to_string()),
                (1, "a.rs".to_string()),
                (1, "b.rs".to_string()),
                (0, "docs/".to_string()),
                (1, "x.md".to_string()),
                (0, "README.md".to_string()),
            ]
        );
        let folded = HashSet::from(["crates/ui/src".to_string()]);
        assert_eq!(tree_rows(&paths, &folded).len(), 4);
    }

    #[test]
    fn import_scanner_matches_t3code_forms() {
        assert_eq!(
            import_specifiers("import x from './a'; export { y } from \"../b\";"),
            ["./a", "../b"]
        );
        assert_eq!(import_specifiers("const z = require ( './c' )"), ["./c"]);
        assert!(import_specifiers("reimport('x'); fromage \"y\"").is_empty());
    }
}
