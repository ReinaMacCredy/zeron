//! A reply's code span naming a file in its chat's folder, as the link it stands for (gpui's `inline_code_links.rs`).

use std::cell::RefCell;
use std::path::{Component, Path, PathBuf};

thread_local! {
    static ROOT: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static CWD: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// The chat being built: its folder, and whether that's on this device (only then probed).
pub(crate) fn set_root(cwd: Option<&str>, here: bool) {
    CWD.with_borrow_mut(|c| *c = cwd.map(str::to_owned));
    ROOT.with_borrow_mut(|r| *r = cwd.filter(|r| here && r.starts_with('/')).map(PathBuf::from));
}

pub(crate) fn cwd() -> Option<String> {
    CWD.with_borrow(Clone::clone)
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    File,
    Dir,
    Missing,
}

fn kind(path: &Path) -> Kind {
    match std::fs::metadata(path) {
        Ok(m) if m.is_file() => Kind::File,
        Ok(m) if m.is_dir() => Kind::Dir,
        _ => Kind::Missing,
    }
}

/// `text` with its file-naming code spans as links, or `None` when none is.
pub(crate) fn linked(text: &str) -> Option<String> {
    let root = ROOT.with_borrow(Clone::clone)?;
    linked_in(text, &root)
}

struct Span {
    start: usize,
    end: usize,
    path: PathBuf,
    anchor: String,
}

pub(crate) fn linked_in(text: &str, root: &Path) -> Option<String> {
    if !text.contains('`') {
        return None;
    }
    let mut spans: Vec<Span> = Vec::new();
    let mut dirs: Vec<PathBuf> = Vec::new();
    for (start, end, inner) in code_spans(text) {
        if text[..start].ends_with('[') && text[end..].starts_with(']') {
            continue;
        }
        let Some((path, anchor)) = resolve(inner, root, &mut dirs) else { continue };
        spans.push(Span { start, end, path, anchor });
    }
    if spans.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(text.len() + spans.len() * 16);
    let mut at = 0;
    for s in &spans {
        out.push_str(&text[at..s.start]);
        let label = label_of(&s.path, &spans);
        let target = s.path.strip_prefix(root).unwrap_or(&s.path);
        out.push('[');
        out.push_str(&escaped(&label));
        out.push_str("](");
        out.push_str(&encoded(&target.to_string_lossy()));
        out.push_str(&s.anchor);
        out.push(')');
        at = s.end;
    }
    out.push_str(&text[at..]);
    Some(out)
}

/// Each inline code span outside fenced blocks: its bytes and its text.
fn code_spans(text: &str) -> Vec<(usize, usize, &str)> {
    let mut out = Vec::new();
    let mut fence: Option<(u8, usize)> = None;
    let mut line_start = 0;
    for line in text.split_inclusive('\n') {
        let at = line_start;
        line_start += line.len();
        let body = line.trim_start_matches(' ');
        let indent = line.len() - body.len();
        let run = |c: u8| body.bytes().take_while(|b| *b == c).count();
        if indent <= 3 {
            let mark = body.bytes().next().filter(|c| *c == b'`' || *c == b'~');
            if let Some(c) = mark
                && run(c) >= 3
            {
                match fence {
                    Some((open, n)) if open == c && run(c) >= n && body[run(c)..].trim().is_empty() => fence = None,
                    None => fence = Some((c, run(c))),
                    _ => {}
                }
                continue;
            }
        }
        if fence.is_some() || indent >= 4 {
            continue;
        }
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\\' {
                i += 2;
                continue;
            }
            if bytes[i] != b'`' {
                i += 1;
                continue;
            }
            let n = bytes[i..].iter().take_while(|b| **b == b'`').count();
            let open = i + n;
            let mut j = open;
            let mut close = None;
            while j < bytes.len() {
                if bytes[j] == b'`' {
                    let m = bytes[j..].iter().take_while(|b| **b == b'`').count();
                    if m == n {
                        close = Some(j);
                        break;
                    }
                    j += m;
                } else {
                    j += 1;
                }
            }
            match close {
                Some(c) => {
                    let mut inner = &line[open..c];
                    if inner.len() > 2 && inner.starts_with(' ') && inner.ends_with(' ') {
                        inner = &inner[1..inner.len() - 1];
                    }
                    out.push((at + i, at + c + n, inner));
                    i = c + n;
                }
                None => i = open,
            }
        }
    }
    out
}

/// gpui's `resolve_inline_code_path`, for files inside the chat's folder.
fn resolve(candidate: &str, root: &Path, dirs: &mut Vec<PathBuf>) -> Option<(PathBuf, String)> {
    if candidate.is_empty() || candidate.trim() != candidate || candidate.contains(['\n', '\r', '\\']) || candidate.contains("://") || candidate.starts_with("mailto:") {
        return None;
    }
    let (raw, line, column) = split_line(candidate);
    let raw = raw.trim_end_matches('/');
    if raw.is_empty() || raw.starts_with('~') {
        return None;
    }
    let anchor = match (line, column) {
        (Some(l), Some(c)) => format!(":{l}:{c}"),
        (Some(l), None) => format!(":{l}"),
        _ => String::new(),
    };
    let path = Path::new(raw);
    let clean = path.components().all(|c| matches!(c, Component::Normal(_) | Component::RootDir));
    if !clean {
        return None;
    }
    if path.has_root() {
        return (path.starts_with(root) && kind(path) == Kind::File).then(|| (path.to_owned(), anchor));
    }
    if raw.split_once(':').is_some_and(|(scheme, _)| !scheme.is_empty() && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))) {
        return None;
    }
    let joined = root.join(path);
    match kind(&joined) {
        Kind::File => return Some((joined, anchor)),
        Kind::Dir => {
            dirs.push(joined);
            return None;
        }
        Kind::Missing => {}
    }
    dirs.iter().map(|d| d.join(path)).find(|p| kind(p) == Kind::File).map(|p| (p, anchor))
}

/// `path:12`, `path:12:5`, `path#L12`, `path#L12C5`.
fn split_line(candidate: &str) -> (&str, Option<u32>, Option<u32>) {
    let num = |s: &str| s.parse::<u32>().ok().filter(|n| *n > 0);
    if let Some((path, frag)) = candidate.rsplit_once("#L") {
        let (l, c) = frag.split_once('C').map_or((frag, None), |(l, c)| (l, Some(c)));
        if let Some(line) = num(l) {
            return (path, Some(line), c.and_then(num));
        }
    }
    let mut parts = candidate.rsplitn(3, ':');
    let last = parts.next().unwrap_or_default();
    let mid = parts.next();
    let head = parts.next();
    match (head, mid.and_then(num), num(last)) {
        (Some(path), Some(line), Some(col)) => (path, Some(line), Some(col)),
        _ => match (mid, num(last)) {
            (Some(_), Some(line)) => (&candidate[..candidate.len() - last.len() - 1], Some(line), None),
            _ => (candidate, None, None),
        },
    }
}

/// Its file name, with as many folders before it as tell it from another span's.
fn label_of(path: &Path, spans: &[Span]) -> String {
    let parts: Vec<String> = path.components().filter_map(|c| match c {
        Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
        _ => None,
    }).collect();
    let others: Vec<Vec<String>> = spans
        .iter()
        .filter(|s| s.path != path && s.path.file_name() == path.file_name())
        .map(|s| s.path.components().filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        }).collect())
        .collect();
    let mut n = 1;
    while n < parts.len() && others.iter().any(|o| o.len() >= n && o[o.len() - n..] == parts[parts.len() - n..]) {
        n += 1;
    }
    parts[parts.len().saturating_sub(n)..].join("/")
}

fn escaped(label: &str) -> String {
    label.chars().fold(String::new(), |mut out, c| {
        if "\\`*_[]<>".contains(c) {
            out.push('\\');
        }
        out.push(c);
        out
    })
}

fn encoded(path: &str) -> String {
    path.bytes().fold(String::new(), |mut out, b| {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("zgx-code-links-{}", std::process::id()));
        for f in ["src/parse.rs", "src/ui/lib.rs", "crates/a/lib.rs", "docs/SOURCES.md", "__init__.py"] {
            let p = dir.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, "x").unwrap();
        }
        dir
    }

    #[test]
    fn a_code_span_naming_a_file_in_the_folder_becomes_its_link() {
        let root = tree();
        let text = "See `src/parse.rs:12` and `src/missing.rs`, then all under `docs/`: `SOURCES.md`.\n\n```\n`src/parse.rs`\n```\n`__init__.py` and [`src/parse.rs`](src/parse.rs) and `src/ui/lib.rs` beside `crates/a/lib.rs`, `cat x`.";
        let out = linked_in(text, &root).unwrap();
        assert!(out.starts_with("See [parse.rs](src/parse.rs:12) and `src/missing.rs`, then all under `docs/`: [SOURCES.md](docs/SOURCES.md)."), "{out}");
        assert!(out.contains("```\n`src/parse.rs`\n```"), "a fence stays code: {out}");
        assert!(out.contains("[\\_\\_init\\_\\_.py](__init__.py)"), "{out}");
        assert!(out.contains("[`src/parse.rs`](src/parse.rs)"), "a link's own code stays: {out}");
        assert!(out.contains("[ui/lib.rs](src/ui/lib.rs) beside [a/lib.rs](crates/a/lib.rs)"), "same names told apart: {out}");
        assert!(out.ends_with("`cat x`."), "{out}");
        assert_eq!(linked_in("no `such/file.rs` here", &root), None);
        assert_eq!(linked_in("`../parse.rs` `/etc/hosts` `https://x.rs`", &root), None);
        let _ = std::fs::remove_dir_all(root);
    }
}
