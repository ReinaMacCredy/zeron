use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};

use super::{
    build::{self, Problem, Program, Root, Tool},
    synctex,
};

const KEEP_VIEWS: usize = 2;
const QUOTE_LINES: u32 = 40;
const QUOTE_BYTES: usize = 4000;
const DRAG: f64 = 3.;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Default)]
struct Slot {
    latest: AtomicU64,
    group: Mutex<Option<i32>>,
    busy: Mutex<()>,
    view: Mutex<Option<PathBuf>>,
}

fn slot(root: &Path) -> Arc<Slot> {
    static SLOTS: OnceLock<Mutex<HashMap<PathBuf, Arc<Slot>>>> = OnceLock::new();
    lock(SLOTS.get_or_init(Default::default)).entry(root.to_path_buf()).or_default().clone()
}

fn roots() -> &'static Mutex<HashMap<PathBuf, PathBuf>> {
    static ROOTS: OnceLock<Mutex<HashMap<PathBuf, PathBuf>>> = OnceLock::new();
    ROOTS.get_or_init(Default::default)
}

fn dir_for(cache: &Path, root: &Path) -> PathBuf {
    let hash = Sha256::digest(root.to_string_lossy().as_bytes());
    let key: String = hash.iter().take(12).map(|b| format!("{b:02x}")).collect();
    cache.join(key)
}

pub struct Done {
    pub state: &'static str,
    pub pdf: Option<PathBuf>,
    pub problems: Vec<Problem>,
    pub tool: &'static str,
    pub root: Option<PathBuf>,
}

impl Done {
    fn only(state: &'static str) -> Done {
        Done { state, pdf: None, problems: Vec::new(), tool: "", root: None }
    }

    fn failed(message: String) -> Done {
        Done { problems: vec![Problem { message, line: None, file: None }], ..Done::only("failed") }
    }
}

pub fn compile(file: &Path, cache: &Path, tectonic: Option<PathBuf>) -> Done {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) => return Done::failed(e.to_string()),
    };
    let root = match build::root_of(file, &text) {
        Root::Itself => file.to_path_buf(),
        Root::At(path) => path,
        Root::Missing(path) => return Done { root: Some(path), ..Done::only("no_root") },
        Root::None => return Done::only("not_document"),
    };
    let program = build::program_of(&if root == file { text } else { std::fs::read_to_string(&root).unwrap_or_default() });
    let Some(tool) = build::find(program, &build::places(std::env::var_os("PATH")), tectonic) else {
        return Done::only("missing");
    };
    compile_with(root, program, tool, cache)
}

pub fn compile_with(root: PathBuf, program: Program, tool: Tool, cache: &Path) -> Done {
    let slot = slot(&root);
    let me = slot.latest.fetch_add(1, Ordering::SeqCst) + 1;
    if let Some(group) = *lock(&slot.group) {
        build::stop(group);
    }
    let _busy = lock(&slot.busy);
    let current = || slot.latest.load(Ordering::SeqCst) == me;
    if !current() {
        return Done::only("superseded");
    }
    let dir = dir_for(cache, &root);
    let out = dir.join("build");
    if let Err(e) = std::fs::create_dir_all(&out) {
        return Done::failed(e.to_string());
    }
    let base = root.parent().unwrap_or(Path::new("/")).to_path_buf();
    lock(roots()).insert(dir.clone(), base.clone());
    let started = SystemTime::now();
    let output = dir.join("output.txt");
    let ran = build::run(build::command(&tool, program, &root, &out), &output, tool.timeout(), |group| {
        *lock(&slot.group) = Some(group);
        if !current() {
            build::stop(group);
        }
    });
    *lock(&slot.group) = None;
    if !current() {
        return Done::only("superseded");
    }
    let ran = match ran {
        Ok(ran) => ran,
        Err(e) => return Done::failed(e),
    };
    let stem = root.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let log = std::fs::read(out.join(format!("{stem}.log"))).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let mut problems = build::problems(&log, &base);
    let ok = !ran.timed_out && ran.status.is_some_and(|s| s.success());
    if problems.is_empty() && !ok && !ran.timed_out {
        let said = std::fs::read(&output).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        problems.extend(build::last_words(&said).map(|message| Problem { message, line: None, file: None }));
    }
    Done {
        state: if ran.timed_out {
            "timeout"
        } else if ok {
            "done"
        } else {
            "failed"
        },
        pdf: publish(&slot, &dir, &out, &stem, started),
        problems,
        tool: tool.name(),
        root: Some(root),
    }
}

fn fresh(path: &Path, since: SystemTime) -> bool {
    std::fs::metadata(path).and_then(|m| m.modified()).is_ok_and(|t| t >= since)
}

fn place(from: &Path, to: &Path) -> std::io::Result<()> {
    let part = to.with_extension(format!("part{}", std::process::id()));
    std::fs::copy(from, &part)?;
    std::fs::rename(&part, to)
}

fn publish(slot: &Slot, dir: &Path, out: &Path, stem: &str, started: SystemTime) -> Option<PathBuf> {
    let built = out.join(format!("{stem}.pdf"));
    let shown = lock(&slot.view).clone().filter(|p| p.is_file());
    if !built.is_file() || (shown.is_some() && !fresh(&built, started)) {
        return shown;
    }
    let id = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
    let view = dir.join(format!("view-{id}.pdf"));
    for ext in ["synctex.gz", "synctex"] {
        let sync = out.join(format!("{stem}.{ext}"));
        if fresh(&sync, started) || (shown.is_none() && sync.is_file()) {
            let _ = place(&sync, &dir.join(format!("view-{id}.{ext}")));
        }
    }
    if place(&built, &view).is_err() {
        return shown;
    }
    *lock(&slot.view) = Some(view.clone());
    prune(dir);
    Some(view)
}

fn prune(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let files: Vec<(u128, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let id = name.strip_prefix("view-")?.split('.').next()?.parse().ok()?;
            Some((id, e.path()))
        })
        .collect();
    let mut ids: Vec<u128> = files.iter().map(|(id, _)| *id).collect();
    ids.sort_unstable_by(|a, b| b.cmp(a));
    ids.dedup();
    for (id, path) in files {
        if !ids.iter().take(KEEP_VIEWS).any(|k| *k == id) {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn parsed(path: &Path) -> Result<Arc<synctex::Synctex>, String> {
    static LAST: Mutex<Option<(PathBuf, Arc<synctex::Synctex>)>> = Mutex::new(None);
    if let Some((p, s)) = lock(&LAST).as_ref()
        && p == path
    {
        return Ok(s.clone());
    }
    let s = Arc::new(synctex::read(path)?);
    *lock(&LAST) = Some((path.to_path_buf(), s.clone()));
    Ok(s)
}

pub struct Found {
    pub file: PathBuf,
    pub line: u32,
    pub end: u32,
    pub text: String,
}

pub fn sync(pdf: &Path, page: u32, from: (f64, f64), to: Option<(f64, f64)>) -> Result<Found, String> {
    let dir = pdf.parent().ok_or("no folder")?;
    let base = lock(roots()).get(dir).cloned().ok_or("Not a compiled preview")?;
    let stem = pdf.file_stem().ok_or("no name")?.to_string_lossy().into_owned();
    let file = ["synctex.gz", "synctex"].iter().map(|ext| dir.join(format!("{stem}.{ext}"))).find(|p| p.is_file()).ok_or("No SyncTeX for this preview")?;
    let tex = parsed(&file)?;
    let mine = |p: &Path| p.starts_with(&base) || (p.is_file() && !p.to_string_lossy().contains("/texmf"));
    let spot = match to {
        Some(to) if (to.0 - from.0).abs() > DRAG || (to.1 - from.1).abs() > DRAG => tex.span(page, from, to, &base, mine),
        _ => tex.spot(page, from.0, from.1, &base, mine),
    }
    .ok_or("Nothing there maps to the source")?;
    let text = quote(&spot.file, spot.line, spot.end);
    Ok(Found { file: spot.file, line: spot.line, end: spot.end, text })
}

fn quote(file: &Path, line: u32, end: u32) -> String {
    let Ok(bytes) = std::fs::read(file) else { return String::new() };
    let text = String::from_utf8_lossy(&bytes);
    let last = end.min(line + QUOTE_LINES - 1);
    let mut out: String = text.lines().skip(line.saturating_sub(1) as usize).take((last + 1 - line) as usize).collect::<Vec<_>>().join("\n");
    if out.len() > QUOTE_BYTES {
        let mut cut = QUOTE_BYTES;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
    }
    out
}
