use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};
use zeron_proto::ui_driver::UiDriverDescriptor;

use super::list_in;

const STABLE_PORT: u16 = 27654;
const PORT_BASE: u16 = 49800;
const NUMBERS: std::ops::RangeInclusive<u16> = 60..=99;
const SCENARIO_N: u16 = 80;
const RELEASE_ID: &str = "sh.zeron.app";
const SCRIPT: &str = "scripts/run-macos-dev.sh";
const BUILD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const QUIT_TIMEOUT: Duration = Duration::from_secs(20);
const POLL: Duration = Duration::from_millis(500);
const PROGRESS_EVERY: Duration = Duration::from_secs(15);
const TAIL_LINES: usize = 40;

struct Launch {
    n: Option<u16>,
    mock: bool,
    name: Option<String>,
    app_args: Vec<String>,
}

impl Launch {
    fn parse(args: &Map<String, Value>) -> Result<Launch, String> {
        let mut launch = Launch {
            n: None,
            mock: false,
            name: None,
            app_args: Vec::new(),
        };
        for (key, value) in args {
            match (key.as_str(), value) {
                ("action", _) | (_, Value::Null) => {}
                ("n", Value::Number(n)) => {
                    launch.n = Some(
                        n.as_u64()
                            .and_then(|n| u16::try_from(n).ok())
                            .filter(|n| NUMBERS.contains(n))
                            .ok_or_else(|| {
                                format!(
                                    "`n` must be {}-{}, not {n}",
                                    NUMBERS.start(),
                                    NUMBERS.end()
                                )
                            })?,
                    );
                }
                ("mock", Value::Bool(mock)) => launch.mock = *mock,
                ("name", Value::String(name)) if !name.trim().is_empty() && !name.contains('/') => {
                    launch.name = Some(name.trim().to_owned());
                }
                ("appArgs", Value::Array(items)) => {
                    launch.app_args = items
                        .iter()
                        .map(|item| item.as_str().map(str::to_owned))
                        .collect::<Option<_>>()
                        .ok_or("`appArgs` must be an array of strings")?;
                }
                (key, value) => {
                    return Err(format!(
                        "`launch` takes n, mock, name and appArgs, not `{key}`: {value}"
                    ));
                }
            }
        }
        Ok(launch)
    }
}

pub(super) async fn launch(
    dir: &Path,
    args: &Map<String, Value>,
    own_port: Option<u16>,
) -> Result<Value, String> {
    let launch = Launch::parse(args)?;
    let checkout = checkout()?;
    let target = target_dir(&checkout);
    let live = list_in(dir);
    let n = match launch.n {
        Some(n) => match taken(n, &target, &live, own_port) {
            Some(why) => return Err(format!("Zeron Dev {n} is not free: {why}")),
            None => n,
        },
        None => NUMBERS
            .filter(|&n| n != SCENARIO_N)
            .find(|&n| taken(n, &target, &live, own_port).is_none())
            .ok_or("every Dev number from 60 to 99 is in use")?,
    };
    let port = PORT_BASE + n;
    let root = target.join(format!("macos-dev{n}"));
    let name = launch.name.unwrap_or_else(|| format!("Zeron Dev {n}"));
    let bundle_id = format!("{RELEASE_ID}.dev{n}");
    std::fs::create_dir_all(&root).map_err(|error| format!("{}: {error}", root.display()))?;
    let root = std::fs::canonicalize(&root).unwrap_or(root);
    let log_path = root.join("launch.log");
    let log = std::fs::File::create(&log_path)
        .map_err(|error| format!("{}: {error}", log_path.display()))?;
    let stdout = log
        .try_clone()
        .map_err(|error| format!("{}: {error}", log_path.display()))?;
    let mut command = Command::new("bash");
    command
        .arg(checkout.join(SCRIPT))
        .args(&launch.app_args)
        .current_dir(&checkout)
        .env("ZERON_DEV_ROOT", &root)
        .env("ZERON_DEV_APP_NAME", &name)
        .env("ZERON_DEV_BUNDLE_ID", &bundle_id)
        .env("ZERON_DEV_IPC_PORT", port.to_string())
        .env_remove("ZERON_DEV_DATA_DIR")
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(log);
    if launch.mock {
        command.env("ZERON_HARNESS", "mock");
    }
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command
        .spawn()
        .map_err(|error| format!("bash {}: {error}", checkout.join(SCRIPT).display()))?;
    eprintln!(
        "launch: building and opening {name} (port {port}); log {}",
        log_path.display()
    );
    let started = Instant::now();
    let mut said = Instant::now();
    loop {
        if let Some(app) = list_in(dir).into_iter().find(|app| inside(app, &root)) {
            return Ok(json!({
                "label": app.label,
                "pid": app.pid,
                "port": app.ipc_port,
                "root": root,
                "log": log_path,
                "bundleId": bundle_id,
            }));
        }
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            stop_group(child.id());
            return Err(format!(
                "{SCRIPT} exited ({status}) before {name} came up; the end of {}:\n{}",
                log_path.display(),
                tail(&log_path, TAIL_LINES)
            ));
        }
        if started.elapsed() >= BUILD_TIMEOUT {
            return Err(format!(
                "{name} did not come up within {} min; the script is still running (pid {}). The end of {}:\n{}",
                BUILD_TIMEOUT.as_secs() / 60,
                child.id(),
                log_path.display(),
                tail(&log_path, TAIL_LINES)
            ));
        }
        if said.elapsed() >= PROGRESS_EVERY {
            said = Instant::now();
            eprintln!(
                "launch: {name} {}s: {}",
                started.elapsed().as_secs(),
                tail(&log_path, 1)
            );
        }
        tokio::time::sleep(POLL).await;
    }
}

pub(super) async fn quit(dir: &Path, app: &UiDriverDescriptor) -> Result<Value, String> {
    let bundle = app.bundle_path.as_deref();
    if app.ipc_port == STABLE_PORT
        || bundle.is_some_and(|bundle| bundle.starts_with("/Applications"))
    {
        return Err(format!(
            "`quit` never quits {} (pid {}, port {}): it hosts the running sessions",
            app.label, app.pid, app.ipc_port
        ));
    }
    let bundle = bundle.ok_or_else(|| {
        format!(
            "{} (pid {}) runs outside an app bundle, so `quit` has no bundle to ask",
            app.label, app.pid
        )
    })?;
    let plist = bundle.join("Contents/Info.plist");
    let id = output(
        Command::new("plutil")
            .args(["-extract", "CFBundleIdentifier", "raw"])
            .arg(&plist),
    )?;
    if id == RELEASE_ID {
        return Err(format!(
            "`quit` never quits the release bundle id {RELEASE_ID} ({})",
            bundle.display()
        ));
    }
    let script = format!("tell application id \"{id}\" to quit");
    let asked = Command::new("osascript")
        .args(["-e", &script])
        .output()
        .map_err(|error| format!("osascript: {error}"))?;
    let started = Instant::now();
    while list_in(dir).iter().any(|live| live.pid == app.pid) {
        if started.elapsed() >= QUIT_TIMEOUT {
            return Err(format!(
                "{} (pid {}) is still running {} s after `{script}` ({})",
                app.label,
                app.pid,
                QUIT_TIMEOUT.as_secs(),
                String::from_utf8_lossy(&asked.stderr).trim()
            ));
        }
        tokio::time::sleep(POLL).await;
    }
    Ok(json!({ "quit": app.label, "pid": app.pid, "bundleId": id }))
}

#[cfg(unix)]
fn stop_group(leader: u32) {
    // SAFETY: the script led its own process group, so this reaches only the builds it left behind.
    unsafe {
        libc::kill(-(leader as libc::pid_t), libc::SIGTERM);
    }
}

#[cfg(not(unix))]
fn stop_group(_: u32) {}

fn checkout() -> Result<PathBuf, String> {
    let cwd = std::env::current_dir().map_err(|error| format!("current directory: {error}"))?;
    cwd.ancestors()
        .find(|dir| dir.join(SCRIPT).is_file())
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            format!(
                "`launch` runs from a Zeron checkout, and {} is not inside one (no {SCRIPT})",
                cwd.display()
            )
        })
}

fn target_dir(checkout: &Path) -> PathBuf {
    match std::env::var_os("CARGO_TARGET_DIR").filter(|dir| !dir.is_empty()) {
        Some(dir) => std::env::current_dir()
            .map(|cwd| cwd.join(&dir))
            .unwrap_or_else(|_| PathBuf::from(dir)),
        None => checkout.join("target"),
    }
}

fn taken(
    n: u16,
    target: &Path,
    live: &[UiDriverDescriptor],
    own_port: Option<u16>,
) -> Option<String> {
    let port = PORT_BASE + n;
    if own_port == Some(port) {
        return Some(format!("port {port} serves this chat"));
    }
    let root = target.join(format!("macos-dev{n}"));
    let root = std::fs::canonicalize(&root).unwrap_or(root);
    let label = format!("Zeron Dev {n}");
    if let Some(app) = live
        .iter()
        .find(|app| app.label == label || app.ipc_port == port || inside(app, &root))
    {
        return Some(format!(
            "{} (pid {}) runs on port {}",
            app.label, app.pid, app.ipc_port
        ));
    }
    let pattern = format!("{}/", root.display());
    if Command::new("pgrep")
        .args(["-f", &pattern])
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
    {
        return Some(format!("a process runs from {}", root.display()));
    }
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    if std::net::TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_ok()
        || std::net::TcpListener::bind(address).is_err()
    {
        return Some(format!("port {port} is in use"));
    }
    None
}

fn inside(app: &UiDriverDescriptor, root: &Path) -> bool {
    app.bundle_path.as_deref().is_some_and(|bundle| {
        std::fs::canonicalize(bundle)
            .unwrap_or_else(|_| bundle.to_path_buf())
            .starts_with(root)
    })
}

fn output(command: &mut Command) -> Result<String, String> {
    let shown = format!("{command:?}");
    let out = command
        .output()
        .map_err(|error| format!("{shown}: {error}"))?;
    if !out.status.success() {
        return Err(format!(
            "{shown} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn tail(path: &Path, lines: usize) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let all: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(value: Value) -> Map<String, Value> {
        let Value::Object(map) = value else {
            unreachable!()
        };
        map
    }

    #[test]
    fn launch_arguments_are_checked_before_anything_spawns() {
        let launch = Launch::parse(&args(json!({
            "action": "launch", "n": 63, "mock": true, "name": "Zeron Dev 63", "appArgs": ["--x"]
        })))
        .unwrap();
        assert_eq!(
            (
                launch.n,
                launch.mock,
                launch.name.as_deref(),
                launch.app_args
            ),
            (Some(63), true, Some("Zeron Dev 63"), vec!["--x".to_owned()])
        );
        for (bad, says) in [
            (json!({ "n": 12 }), "`n` must be 60-99"),
            (json!({ "n": 70000 }), "`n` must be 60-99"),
            (json!({ "appArgs": [1] }), "array of strings"),
            (json!({ "name": "../x" }), "not `name`"),
            (json!({ "window": 1 }), "not `window`"),
        ] {
            let error = Launch::parse(&args(bad)).err().unwrap();
            assert!(error.contains(says), "{error}");
        }
    }

    #[test]
    fn a_number_is_taken_by_the_stable_port_the_callers_port_or_a_live_label() {
        let target = Path::new("/nonexistent/target");
        let live = vec![UiDriverDescriptor {
            pid: 7,
            endpoint: zeron_proto::ui_driver::UiDriverEndpoint::Unix {
                socket: PathBuf::from("/run/ui-7.sock"),
            },
            label: "Zeron Dev 61".into(),
            bundle_path: Some(PathBuf::from("/elsewhere/Zeron Dev 61.app")),
            data_dir: PathBuf::from("/data"),
            ipc_port: 49862,
            build: zeron_proto::ui_driver::UiDriverBuild::Debug,
            input: true,
            version: "0.0.0".into(),
            started_at: chrono::Utc::now(),
        }];
        assert!(taken(61, target, &live, None).unwrap().contains("pid 7"));
        assert!(taken(62, target, &live, None).unwrap().contains("pid 7"));
        assert!(
            taken(63, target, &[], Some(49863))
                .unwrap()
                .contains("serves this chat")
        );
    }
}
