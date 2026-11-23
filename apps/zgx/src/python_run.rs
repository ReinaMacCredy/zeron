use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::io::Read;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

use serde_json::json;
use sha2::{Digest, Sha256};
use zgx_script::{ScriptHost, Shared, Value};

const SANDBOX: &str = "/usr/bin/sandbox-exec";
const PLACES: [&str; 2] = ["/opt/homebrew/bin", "/usr/local/bin"];
pub const WALL: Duration = Duration::from_secs(10);
pub const CAP: usize = 64 * 1024;
const WARM: Duration = Duration::from_secs(120);
const BUILDING_FONTS: &str = "Matplotlib is building the font cache";
// The kernel's backstop for what the watchdog can't reach (the app quit, a child left the group); above WALL since numpy's threads add up.
const CPU_SECONDS: libc::rlim_t = 60;

const PROFILE: &str = r#"(version 1)
(deny default)
(allow process-fork process-exec sysctl-read)
(allow signal (target same-sandbox))
(allow file-read* (literal "/") (subpath "/usr") (subpath "/System") (subpath "/Library") (subpath "/bin") (subpath "/sbin") (subpath "/opt") (subpath "/private/etc") (subpath "/private/var/db") (subpath "/dev") (subpath (param "PREFIX")) (subpath (param "ENV")) (subpath (param "CHAT")) (subpath (param "SCRATCH")))
(allow file-write* (subpath (param "SCRATCH")) (literal "/dev/null"))
(deny network*)"#;

const RUNNER: &str = r#"import os, sys
figures = os.environ.get("ZERON_FIGURES")
if figures:
    try:
        import matplotlib
        matplotlib.use("Agg")
        import matplotlib.pyplot as plt
        shown = [0]
        def show(*args, **kwargs):
            for number in plt.get_fignums():
                shown[0] += 1
                plt.figure(number).savefig(os.path.join(figures, "figure-%03d.png" % shown[0]), dpi=144, bbox_inches="tight")
            plt.close("all")
        plt.show = show
    except ImportError:
        pass
with open(sys.argv[1], encoding="utf-8") as f:
    code = compile(f.read(), "main.py", "exec")
try:
    exec(code, {"__name__": "__main__", "__file__": "main.py"})
except SystemExit:
    raise
except BaseException:
    import traceback
    kind, error, trace = sys.exc_info()
    traceback.print_exception(kind, error, trace.tb_next)
    sys.exit(1)
"#;

#[derive(Clone, Debug, PartialEq)]
pub struct Python {
    pub bin: PathBuf,
    prefix: PathBuf,
    env: Option<PathBuf>,
    links: Vec<PathBuf>,
}

fn narrow(path: &Path) -> bool {
    let home = std::env::var_os("HOME").map(PathBuf::from).and_then(|h| h.canonicalize().ok());
    path != Path::new("/") && !home.is_some_and(|h| h.starts_with(path))
}

fn links(path: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut at = path.to_path_buf();
    'again: for _ in 0..32 {
        let parts: Vec<_> = at.components().collect();
        let mut prefix = PathBuf::new();
        for (i, part) in parts.iter().enumerate() {
            prefix.push(part);
            if prefix.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink()) {
                let Ok(target) = std::fs::read_link(&prefix) else { break 'again };
                out.push(prefix.clone());
                let mut next = prefix.parent().unwrap_or(Path::new("/")).join(target);
                next.extend(&parts[i + 1..]);
                at = next;
                continue 'again;
            }
        }
        break;
    }
    out
}

pub fn candidates(path: Option<OsString>) -> Vec<Python> {
    let mut dirs: Vec<PathBuf> = path.map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.extend(PLACES.iter().map(PathBuf::from));
    let mut seen: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
    let mut out = Vec::new();
    for dir in dirs {
        let bin = dir.join("python3");
        let Ok(real) = bin.canonicalize() else { continue };
        let Some(prefix) = real.parent().and_then(Path::parent).filter(|p| narrow(p)).map(Path::to_path_buf) else { continue };
        let env = dir.parent().filter(|d| d.join("pyvenv.cfg").is_file()).and_then(|d| d.canonicalize().ok()).filter(|d| narrow(d));
        if !real.is_file() || real.starts_with("/usr/bin") || seen.contains(&(real.clone(), env.clone())) {
            continue;
        }
        seen.push((real, env.clone()));
        out.push(Python { links: links(&bin), bin, prefix, env });
    }
    out
}

pub fn find(path: Option<OsString>) -> Option<Python> {
    if !Path::new(SANDBOX).is_file() {
        return None;
    }
    candidates(path).into_iter().find(usable)
}

fn usable(python: &Python) -> bool {
    static PROBES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = PROBES.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("zeron-python-probe-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let Ok(scratch) = std::fs::create_dir_all(&dir).and_then(|()| dir.canonicalize()) else { return false };
    let job = Job { python, code: "print(1)\n", scratch: &scratch, chat: None, wall: Duration::from_secs(5) };
    let ok = run(&job, &Live::default()).is_ok_and(|r| matches!(r.ended, Ended::Done) && r.exit == Some(0) && r.output == "1\n");
    let _ = std::fs::remove_dir_all(&dir);
    ok
}

pub fn readable(root: &str) -> Option<PathBuf> {
    let path = Path::new(root);
    if !path.is_absolute() {
        return None;
    }
    path.canonicalize().ok().filter(|p| p.is_dir() && narrow(p))
}

pub struct Job<'a> {
    pub python: &'a Python,
    pub code: &'a str,
    pub scratch: &'a Path,
    pub chat: Option<&'a Path>,
    pub wall: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ended {
    Done,
    Stopped,
    TimedOut,
}

impl Ended {
    fn name(self) -> &'static str {
        match self {
            Ended::Done => "done",
            Ended::Stopped => "stopped",
            Ended::TimedOut => "timed_out",
        }
    }
}

#[derive(Debug)]
pub struct Ran {
    pub ended: Ended,
    pub output: String,
    pub cut: bool,
    pub figures: Vec<PathBuf>,
    pub ms: u64,
    pub exit: Option<i32>,
}

#[derive(Default)]
pub struct Live {
    group: Mutex<Option<i32>>,
    stopped: AtomicBool,
}

impl Live {
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(group) = *self.group.lock().unwrap_or_else(|e| e.into_inner()) {
            kill(group);
        }
    }
}

fn kill(group: i32) {
    // SAFETY: a signal to a process group this module started.
    unsafe {
        libc::killpg(group, libc::SIGKILL);
    }
}

pub fn run(job: &Job, live: &Live) -> Result<Ran, String> {
    let said = |e: std::io::Error| e.to_string();
    let main = job.scratch.join("main.py");
    std::fs::write(&main, job.code).map_err(said)?;
    let figures = job.scratch.join("figures");
    std::fs::create_dir_all(&figures).map_err(said)?;
    let (mut reader, writer) = std::io::pipe().map_err(said)?;
    let param = |name: &str, path: &Path| format!("{name}={}", path.display());
    let mut stat_only: Vec<&Path> = Vec::new();
    let reached = [Some(job.scratch), Some(job.python.prefix.as_path()), job.python.env.as_deref(), job.chat];
    for path in reached.into_iter().flatten().chain(job.python.links.iter().map(PathBuf::as_path)) {
        for up in path.ancestors().filter(|p| *p != Path::new("/") && !p.as_os_str().is_empty()) {
            if !stat_only.contains(&up) {
                stat_only.push(up);
            }
        }
    }
    let mut profile = PROFILE.to_owned();
    profile.push_str("\n(allow file-read-metadata (literal \"/\")");
    for i in 0..stat_only.len() {
        profile.push_str(&format!(" (literal (param \"M{i}\"))"));
    }
    profile.push(')');
    let mut cmd = Command::new(SANDBOX);
    for (i, path) in stat_only.iter().enumerate() {
        cmd.arg("-D").arg(param(&format!("M{i}"), path));
    }
    cmd.arg("-D").arg(param("SCRATCH", job.scratch));
    cmd.arg("-D").arg(param("PREFIX", &job.python.prefix));
    cmd.arg("-D").arg(param("ENV", job.python.env.as_deref().unwrap_or(job.scratch)));
    cmd.arg("-D").arg(param("CHAT", job.chat.unwrap_or(job.scratch)));
    cmd.arg("-p").arg(&profile).arg(&job.python.bin).args(["-I", "-u", "-c", RUNNER]).arg(&main);
    let bin_dir = job.python.bin.parent().map(|d| format!("{}:", d.display())).unwrap_or_default();
    cmd.env_clear()
        .env("PATH", format!("{bin_dir}/usr/bin:/bin:/usr/sbin:/sbin"))
        .env("HOME", job.scratch)
        .env("TMPDIR", job.scratch)
        .env("LANG", "en_US.UTF-8")
        .env("MPLBACKEND", "Agg")
        .env("MPLCONFIGDIR", job.scratch.join(".matplotlib"));
    if job.code.contains("matplotlib") {
        cmd.env("ZERON_FIGURES", &figures);
    }
    cmd.current_dir(job.chat.unwrap_or(job.scratch));
    cmd.stdin(Stdio::null()).stdout(writer.try_clone().map_err(said)?).stderr(writer);
    cmd.process_group(0);
    // SAFETY: setrlimit is async-signal-safe; it only lowers this child's own limit.
    unsafe {
        cmd.pre_exec(|| {
            let limit = libc::rlimit { rlim_cur: CPU_SECONDS, rlim_max: CPU_SECONDS };
            libc::setrlimit(libc::RLIMIT_CPU, &limit);
            Ok(())
        });
    }
    let started = Instant::now();
    let spawned = {
        let mut held = live.group.lock().unwrap_or_else(|e| e.into_inner());
        let child = cmd.spawn();
        if let Ok(child) = &child {
            *held = Some(child.id() as i32);
        }
        child
    };
    drop(cmd);
    let mut child = spawned.map_err(said)?;
    let group = child.id() as i32;
    if live.stopped.load(Ordering::SeqCst) {
        kill(group);
    }
    let captured = Arc::new(Mutex::new((Vec::<u8>::new(), false)));
    let (read_done, read_wait) = mpsc::channel::<()>();
    {
        let captured = captured.clone();
        std::thread::Builder::new()
            .name("zeron-python-output".into())
            .spawn(move || {
                let mut chunk = [0u8; 8192];
                loop {
                    match reader.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => {
                            let mut held = captured.lock().unwrap_or_else(|e| e.into_inner());
                            let room = CAP.saturating_sub(held.0.len());
                            held.0.extend_from_slice(&chunk[..n.min(room)]);
                            held.1 |= n > room;
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(_) => break,
                    }
                }
                let _ = read_done.send(());
            })
            .map_err(said)?;
    }
    let late = Arc::new(AtomicBool::new(false));
    let (done, wait) = mpsc::channel::<()>();
    let watch = {
        let (late, wall) = (late.clone(), job.wall);
        std::thread::Builder::new()
            .name("zeron-python-timeout".into())
            .spawn(move || {
                if let Err(mpsc::RecvTimeoutError::Timeout) = wait.recv_timeout(wall) {
                    late.store(true, Ordering::SeqCst);
                    kill(group);
                }
            })
            .map_err(said)?
    };
    let status = child.wait();
    let ms = started.elapsed().as_millis() as u64;
    let _ = done.send(());
    let _ = watch.join();
    {
        let mut held = live.group.lock().unwrap_or_else(|e| e.into_inner());
        kill(group);
        *held = None;
    }
    let _ = read_wait.recv_timeout(Duration::from_millis(500));
    let (bytes, cut) = std::mem::take(&mut *captured.lock().unwrap_or_else(|e| e.into_inner()));
    let mut shown: Vec<PathBuf> = std::fs::read_dir(&figures)
        .map(|dir| dir.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "png")).collect())
        .unwrap_or_default();
    shown.sort();
    let ended = if live.stopped.load(Ordering::SeqCst) {
        Ended::Stopped
    } else if late.load(Ordering::SeqCst) {
        Ended::TimedOut
    } else {
        Ended::Done
    };
    let exit = status.ok().and_then(|s| s.code().or_else(|| s.signal().map(|n| 128 + n)));
    Ok(Ran { ended, output: String::from_utf8_lossy(&bytes).into_owned(), cut, figures: shown, ms, exit })
}

static PYTHON: OnceLock<Option<Python>> = OnceLock::new();

#[derive(Default)]
struct Runs {
    dir: Option<PathBuf>,
    shared: Vec<Shared>,
    rev: f64,
    results: HashMap<String, Value>,
    live: HashMap<String, Arc<Live>>,
}

thread_local! {
    static RUNS: RefCell<Runs> = RefCell::new(Runs::default());
}

fn key_of(arg: &Value) -> String {
    let text = |k: &str| arg.field(k).map(crate::auth::text).unwrap_or_default();
    let mut hash = Sha256::new();
    hash.update(text("chat").as_bytes());
    hash.update([0]);
    hash.update(text("code").as_bytes());
    hash.finalize().iter().take(16).map(|b| format!("{b:02x}")).collect()
}

fn publish() {
    RUNS.with(|r| {
        let mut r = r.borrow_mut();
        r.rev += 1.;
        let rev = Value::from(r.rev);
        for shared in &r.shared {
            shared.set(rev.clone());
        }
    });
}

fn stored(dir: &Path, key: &str) -> Option<serde_json::Value> {
    std::fs::read(dir.join(format!("{key}.json"))).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

fn scratch_of(json: &serde_json::Value) -> Option<String> {
    json.get("scratch").and_then(|p| p.as_str()).filter(|n| !n.is_empty() && !n.contains('/') && *n != "..").map(str::to_owned)
}

fn forget(dir: &Path, key: &str) {
    if let Some(name) = stored(dir, key).as_ref().and_then(scratch_of) {
        let _ = std::fs::remove_dir_all(dir.join(name));
    }
    let _ = std::fs::remove_file(dir.join(format!("{key}.json")));
}

fn sweep(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let entries: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    let kept: HashSet<String> = entries
        .iter()
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| std::fs::read(p).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()))
        .filter_map(|s| scratch_of(&s))
        .collect();
    for path in entries {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if path.is_dir() && !kept.contains(&name) && !name.starts_with(".matplotlib-") {
            let _ = std::fs::remove_dir_all(&path);
        } else if path.extension().is_some_and(|x| x == "part") {
            let _ = std::fs::remove_file(&path);
        }
    }
}

fn record(ran: &Ran, name: &str) -> serde_json::Value {
    json!({
        "state": ran.ended.name(),
        "output": ran.output,
        "cut": ran.cut,
        "figures": ran.figures.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>(),
        "ms": ran.ms,
        "exit": ran.exit,
        "scratch": name,
    })
}

fn as_value(json: &serde_json::Value) -> Value {
    Value::from_json(&json.to_string()).unwrap_or(Value::Nil)
}

fn result(key: &str) -> Value {
    RUNS.with(|r| {
        let mut r = r.borrow_mut();
        if r.live.contains_key(key) {
            return Value::record([("state", Value::from("running"))]);
        }
        if let Some(v) = r.results.get(key) {
            return v.clone();
        }
        let v = r.dir.as_deref().and_then(|d| stored(d, key)).map_or(Value::Nil, |s| as_value(&s));
        r.results.insert(key.to_owned(), v.clone());
        v
    })
}

fn font_lists(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("fontlist-") && n.ends_with(".json"))).collect())
        .unwrap_or_default()
}

// A fresh MPLCONFIGDIR rebuilds matplotlib's font list (15 s here), past WALL: it's built once by our own script and copied into each run.
fn fonts(dir: &Path, python: &Python, scratch: &Path, live: &Live) -> PathBuf {
    let id: String = Sha256::digest(python.bin.as_os_str().as_encoded_bytes()).iter().take(8).map(|b| format!("{b:02x}")).collect();
    let cache = dir.join(format!(".matplotlib-{id}"));
    if font_lists(&cache).is_empty() {
        let warming = dir.join(format!(".warming-{id}"));
        let _ = std::fs::remove_dir_all(&warming);
        if let Ok(at) = std::fs::create_dir_all(&warming).and_then(|()| warming.canonicalize()) {
            let job = Job { python, code: "import matplotlib.font_manager\n", scratch: &at, chat: None, wall: WARM };
            if run(&job, live).is_ok_and(|r| r.ended == Ended::Done && r.exit == Some(0)) && std::fs::create_dir_all(&cache).is_ok() {
                for list in font_lists(&at.join(".matplotlib")) {
                    let _ = list.file_name().map(|n| std::fs::copy(&list, cache.join(n)));
                }
            }
            let _ = std::fs::remove_dir_all(&warming);
        }
    }
    let into = scratch.join(".matplotlib");
    if std::fs::create_dir_all(&into).is_ok() {
        for list in font_lists(&cache) {
            let _ = list.file_name().map(|n| std::fs::copy(&list, into.join(n)));
        }
    }
    cache
}

fn keep_run(dir: &Path, key: &str, python: &Python, code: &str, chat: Option<&Path>, live: &Live) -> Result<serde_json::Value, String> {
    forget(dir, key);
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    let name = format!("{key}-{stamp}");
    let made = dir.join(&name);
    let scratch = std::fs::create_dir_all(&made).and_then(|()| made.canonicalize()).map_err(|e| e.to_string())?;
    let cache = code.contains("matplotlib").then(|| fonts(dir, python, &scratch, live));
    let ran = match run(&Job { python, code, scratch: &scratch, chat, wall: WALL }, live) {
        Ok(ran) => ran,
        Err(why) => {
            let _ = std::fs::remove_dir_all(&scratch);
            return Err(why);
        }
    };
    if let Some(cache) = cache
        && ran.output.contains(BUILDING_FONTS)
    {
        let _ = std::fs::remove_dir_all(cache);
    }
    let kept = record(&ran, &name);
    let path = dir.join(format!("{key}.json"));
    let part = path.with_extension("json.part");
    if let Err(why) = std::fs::write(&part, kept.to_string()).and_then(|()| std::fs::rename(&part, &path)) {
        eprintln!("zeron-zgx: python run {key}: couldn't keep its result: {why}");
    }
    Ok(kept)
}

async fn start(arg: Value) -> Result<Value, String> {
    crate::plugins::preview::refuse("python.run")?;
    let Some(Some(python)) = PYTHON.get() else { return Err("Python isn't available on this Mac".into()) };
    let code = arg.field("code").map(crate::auth::text).unwrap_or_default();
    if code.trim().is_empty() {
        return Err("Nothing to run".into());
    }
    let key = key_of(&arg);
    let chat = arg.field("root").map(crate::auth::text).and_then(|r| readable(&r));
    let dir = RUNS.with(|r| r.borrow().dir.clone()).ok_or("Nowhere to run it")?;
    let live = Arc::new(Live::default());
    RUNS.with(|r| {
        let mut r = r.borrow_mut();
        if r.live.contains_key(&key) {
            return Err("It's already running");
        }
        r.live.insert(key.clone(), live.clone());
        r.results.remove(&key);
        Ok(())
    })?;
    publish();
    let (python, k) = (python.clone(), key.clone());
    let ran = crate::favicons::off_thread(move || keep_run(&dir, &k, &python, &code, chat.as_deref(), &live))
        .await
        .unwrap_or_else(|| Err("The run stopped".into()));
    let value = match &ran {
        Ok(kept) => as_value(kept),
        Err(why) => Value::record([("state", Value::from("failed")), ("error", Value::from(why.as_str()))]),
    };
    RUNS.with(|r| {
        let mut r = r.borrow_mut();
        r.live.remove(&key);
        r.results.insert(key, value.clone());
    });
    publish();
    Ok(value)
}

pub fn install(cx: &mut zgx_desktop::WindowContext, host: &ScriptHost) {
    let dir = RUNS.with(|r| {
        let mut r = r.borrow_mut();
        if r.dir.is_none() {
            r.dir = zgx_script::app_data_dir(crate::APP_ID).map(|d| d.join("python-runs"));
            return r.dir.clone();
        }
        None
    });
    if let Some(dir) = dir {
        cx.tasks.spawn(async move {
            let _ = crate::favicons::off_thread(move || {
                sweep(&dir);
                PYTHON.get_or_init(|| find(std::env::var_os("PATH")));
            })
            .await;
            publish();
        });
    }
    RUNS.with(|r| {
        let rev = Value::from(r.borrow().rev);
        r.borrow_mut().shared.push(host.shared("python.runs", rev));
    });
    host.function("python.available", |_| Value::from(matches!(PYTHON.get(), Some(Some(_)))));
    host.function("python.result", |arg| result(&key_of(&arg)));
    host.async_action("python.run", |arg| async move { start(arg).await });
    host.action("python.stop", |arg| {
        let key = key_of(&arg);
        RUNS.with(|r| {
            if let Some(live) = r.borrow().live.get(&key) {
                live.stop();
            }
        });
    });
    host.action("python.clear", |arg| {
        let key = key_of(&arg);
        let dir = RUNS.with(|r| {
            let mut r = r.borrow_mut();
            if r.live.contains_key(&key) {
                return None;
            }
            r.results.insert(key.clone(), Value::Nil);
            r.dir.clone()
        });
        if let Some(dir) = dir {
            forget(&dir, &key);
            publish();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn python() -> Option<Python> {
        let found = find(std::env::var_os("PATH"));
        if found.is_none() {
            eprintln!("skipped: no sandboxed python3 on this machine");
        }
        found
    }

    fn scratch(name: &str) -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("zgx python \"{name}\" {}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("scratch")).unwrap();
        let base = base.canonicalize().unwrap();
        (base.join("scratch"), base)
    }

    fn ran(python: &Python, scratch: &Path, code: &str, wall: Duration) -> Ran {
        run(&Job { python, code, scratch, chat: None, wall }, &Live::default()).unwrap()
    }

    #[test]
    fn python_run_gives_back_its_output_and_exit_and_writes_in_its_scratch() {
        let Some(python) = python() else { return };
        let (scratch, base) = scratch("ok");
        let r = ran(&python, &scratch, "print('hi')\nopen('made.txt', 'w').write('x')\nraise SystemExit(3)\n", WALL);
        assert_eq!((r.ended, r.exit, r.output.as_str(), r.cut), (Ended::Done, Some(3), "hi\n", false));
        assert_eq!(std::fs::read_to_string(scratch.join("made.txt")).unwrap(), "x");
        let r = ran(&python, &scratch, "import sys\nsys.stdout.write('a' * 200000)\nprint('late', file=sys.stderr)\n", WALL);
        assert!(r.cut && r.output.len() == CAP && r.exit == Some(0));
        let r = ran(&python, &scratch, "def f():\n    1 / 0\nf()\n", WALL);
        assert!(r.exit == Some(1) && r.output.contains("File \"main.py\", line 2") && r.output.contains("ZeroDivisionError"), "{}", r.output);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn python_run_has_no_network() {
        let Some(python) = python() else { return };
        let (scratch, base) = scratch("net");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let code = format!("import socket\ntry:\n    socket.create_connection(('127.0.0.1', {port}), timeout=2)\n    print('connected')\nexcept OSError as e:\n    print('refused', e)\n");
        let r = ran(&python, &scratch, &code, WALL);
        assert!(r.output.starts_with("refused"), "{}", r.output);
        assert!(listener.accept().is_err_and(|e| e.kind() == std::io::ErrorKind::WouldBlock));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn python_run_writes_nowhere_but_its_scratch() {
        let Some(python) = python() else { return };
        let (scratch, base) = scratch("write");
        let outside = base.join("outside.txt");
        let code = format!("try:\n    open({:?}, 'w').write('x')\n    print('wrote')\nexcept OSError as e:\n    print('refused', type(e).__name__)\n", outside.display().to_string());
        let r = ran(&python, &scratch, &code, WALL);
        assert!(r.output.starts_with("refused PermissionError"), "{}", r.output);
        assert!(!outside.exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn python_run_reads_in_the_chats_folder_and_changes_nothing_there() {
        let Some(python) = python() else { return };
        let (scratch, base) = scratch("chat folder");
        let chat = base.join("my project");
        std::fs::create_dir_all(&chat).unwrap();
        std::fs::write(chat.join("data.txt"), "kept").unwrap();
        let code = "import os\nprint(open('data.txt').read())\nfor act in (lambda: open('new.txt', 'w'), lambda: os.remove('data.txt'), lambda: open('data.txt', 'w')):\n    try:\n        act()\n        print('changed')\n    except OSError as e:\n        print('refused', type(e).__name__)\n";
        let r = run(&Job { python: &python, code, scratch: &scratch, chat: Some(&chat), wall: WALL }, &Live::default()).unwrap();
        assert_eq!(r.output, "kept\nrefused PermissionError\nrefused PermissionError\nrefused PermissionError\n");
        assert_eq!(std::fs::read_to_string(chat.join("data.txt")).unwrap(), "kept");
        assert!(!chat.join("new.txt").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn python_run_reads_nothing_of_the_home_folder() {
        let Some(python) = python() else { return };
        let (scratch, base) = scratch("read");
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap().canonicalize().unwrap();
        let file = home.join("Library/Preferences/.GlobalPreferences.plist");
        assert!(std::fs::read(&file).is_ok(), "the test itself reads {}", file.display());
        let code = format!("try:\n    open({:?}, 'rb').read()\n    print('read')\nexcept OSError as e:\n    print('refused', type(e).__name__)\n", file.display().to_string());
        let r = ran(&python, &scratch, &code, WALL);
        assert!(r.output.starts_with("refused PermissionError"), "{}", r.output);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn python_run_past_its_time_is_killed_and_says_so() {
        let Some(python) = python() else { return };
        let (scratch, base) = scratch("loop");
        let started = Instant::now();
        let r = ran(&python, &scratch, "print('started')\nwhile True:\n    pass\n", Duration::from_millis(800));
        assert_eq!(r.ended, Ended::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(5) && r.output == "started\n", "{}", r.output);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn python_run_takes_its_children_with_it() {
        let Some(python) = python() else { return };
        let (scratch, base) = scratch("child");
        let r = ran(&python, &scratch, "import subprocess\np = subprocess.Popen(['/bin/sleep', '30'])\nprint(p.pid)\n", WALL);
        let child: i32 = r.output.trim().parse().unwrap();
        std::thread::sleep(Duration::from_millis(300));
        // SAFETY: signal 0 only asks whether the pid is alive.
        assert_ne!(unsafe { libc::kill(child, 0) }, 0, "the child went with the run");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn python_run_stops_when_asked() {
        let Some(python) = python() else { return };
        let (scratch, base) = scratch("stop");
        let live = Arc::new(Live::default());
        let stopper = {
            let live = live.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(600));
                live.stop();
            })
        };
        let started = Instant::now();
        let r = run(&Job { python: &python, code: "import time\nprint('a')\ntime.sleep(30)\n", scratch: &scratch, chat: None, wall: WALL }, &live).unwrap();
        stopper.join().unwrap();
        assert_eq!(r.ended, Ended::Stopped);
        assert!(started.elapsed() < Duration::from_secs(5) && r.output == "a\n");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn python_run_shows_its_figures_where_matplotlib_is() {
        let Some(python) = python() else { return };
        let has = Command::new(&python.bin).args(["-I", "-c", "import matplotlib"]).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
        if !has {
            eprintln!("skipped: {} has no matplotlib", python.bin.display());
            return;
        }
        let (dir, base) = scratch("figure");
        let code = "import matplotlib.pyplot as plt\nplt.bar([1, 2], [3, 4])\nplt.show()\nplt.plot([1, 2])\nplt.show()\n";
        for _ in 0..2 {
            let kept = keep_run(&dir, "figure", &python, code, None, &Live::default()).unwrap();
            let figures: Vec<PathBuf> = kept["figures"].as_array().unwrap().iter().filter_map(|f| f.as_str().map(PathBuf::from)).collect();
            assert_eq!((kept["state"].as_str(), figures.len()), (Some("done"), 2), "{}", kept["output"]);
            assert!(figures.iter().all(|f| f.starts_with(&dir) && std::fs::metadata(f).is_ok_and(|m| m.len() > 0)));
            assert!(kept["ms"].as_u64().unwrap() < WALL.as_millis() as u64);
        }
        sweep(&dir);
        assert!(std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).any(|e| e.file_name().to_string_lossy().starts_with(".matplotlib-")), "the sweep keeps the font list");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn python_run_result_outlives_a_restart_and_clear_lets_it_go() {
        let Some(python) = python() else { return };
        let (dir, base) = scratch("keep");
        RUNS.with(|r| *r.borrow_mut() = Runs { dir: Some(dir.clone()), ..Runs::default() });
        let arg = |chat: &str| Value::record([("chat", Value::from(chat)), ("code", Value::from("print(2)"))]);
        let key = key_of(&arg("c1"));
        assert_ne!(key, key_of(&arg("c2")));
        let first = keep_run(&dir, &key, &python, "print(2)", None, &Live::default()).unwrap();
        let first = dir.join(scratch_of(&first).unwrap());
        assert!(first.is_dir());
        let shown = result(&key);
        assert_eq!((crate::auth::text(shown.field("state").unwrap()), crate::auth::text(shown.field("output").unwrap())), ("done".into(), "2\n".into()));
        RUNS.with(|r| r.borrow_mut().results.clear());
        assert_eq!(crate::auth::text(result(&key).field("output").unwrap()), "2\n", "read back from disk, as after a restart");
        std::thread::sleep(Duration::from_millis(5));
        let second = dir.join(scratch_of(&keep_run(&dir, &key, &python, "print(2)", None, &Live::default()).unwrap()).unwrap());
        assert!(second.is_dir() && !first.exists(), "a new run lets the last one's scratch go");
        std::fs::create_dir(dir.join("left-by-a-quit")).unwrap();
        sweep(&dir);
        assert!(second.is_dir() && !dir.join("left-by-a-quit").exists());
        forget(&dir, &key);
        assert!(!second.exists() && !dir.join(format!("{key}.json")).exists());
        RUNS.with(|r| *r.borrow_mut() = Runs::default());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn python_run_reads_only_a_chat_folder_that_is_not_home() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(readable(&home), None);
        assert_eq!(readable("/"), None);
        assert_eq!(readable("relative/dir"), None);
        let (scratch, base) = scratch("chat");
        assert_eq!(readable(&scratch.display().to_string()), Some(scratch.clone()));
        let _ = std::fs::remove_dir_all(&base);
    }
}
