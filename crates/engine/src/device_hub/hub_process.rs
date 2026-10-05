use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};

pub(crate) const READY_TIMEOUT: Duration = Duration::from_secs(30);
const STOP_GRACE: Duration = Duration::from_secs(5);
const STATE_FILE: &str = "hub.json";
const GUARD_FILE: &str = "hub-guard.mjs";
// Browsers reach a loopback port with a foreign Host (DNS rebinding) or an Origin; the engine sends neither.
const GUARD: &str = r#"import http from "node:http";
const emit = http.Server.prototype.emit;
const local = (server, request) => {
  const address = server.address();
  if (typeof address !== "object" || address === null) return false;
  const host = String(request.headers.host ?? "").toLowerCase();
  return [`127.0.0.1:${address.port}`, `localhost:${address.port}`, `[::1]:${address.port}`].includes(host);
};
http.Server.prototype.emit = function (event, request, target) {
  if (
    ["request", "checkContinue", "checkExpectation"].includes(event) &&
    (!local(this, request) || request.headers.origin !== undefined)
  ) {
    target.writeHead(403, { Connection: "close" }).end();
    return true;
  }
  if (event === "upgrade" && (!local(this, request) || request.headers.origin !== undefined)) {
    target.destroy();
    return true;
  }
  return emit.apply(this, arguments);
};
"#;
const RESTART_STABLE_UPTIME: Duration = Duration::from_secs(60);
const RESTART_MAX_DELAY: Duration = Duration::from_secs(30);
// The kernel closes our end of stdin however the engine dies, so the watcher SIGTERMs the group.
#[cfg(unix)]
const LIFELINE: &str = r#"exec 3<&0 </dev/null; p=$1; shift
(/bin/cat <&3; kill -TERM 0) >/dev/null 2>&1 &
(while kill -0 "$p" 2>/dev/null; do /bin/sleep 1; done; kill -TERM 0) >/dev/null 2>&1 3<&- &
exec "$@" 3<&-"#;

#[derive(Debug, Default)]
pub(crate) struct Backoff {
    next: Duration,
}

impl Backoff {
    pub(crate) fn after_exit(&mut self, uptime: Duration) -> Duration {
        if uptime >= RESTART_STABLE_UPTIME {
            self.next = Duration::from_secs(1);
            return Duration::ZERO;
        }
        let delay = self.next;
        self.next = if delay.is_zero() {
            Duration::from_secs(1)
        } else {
            (delay * 2).min(RESTART_MAX_DELAY)
        };
        delay
    }

    pub(crate) fn reset(&mut self) {
        self.next = Duration::ZERO;
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HubStateFile {
    pid: u32,
    port: u16,
    entry_path: String,
}

pub(crate) struct HubProcess {
    pub origin: String,
    pgid: i32,
    started: std::time::Instant,
    child: Option<Child>,
    _lifeline: Option<ChildStdin>,
    state_file: Option<PathBuf>,
}

impl HubProcess {
    #[cfg(test)]
    pub(crate) fn external(origin: String) -> Self {
        Self {
            origin,
            pgid: 0,
            started: std::time::Instant::now(),
            child: None,
            _lifeline: None,
            state_file: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_child(origin: String, child: Child) -> Self {
        Self {
            origin,
            pgid: child
                .id()
                .and_then(|pid| i32::try_from(pid).ok())
                .unwrap_or_default(),
            started: std::time::Instant::now(),
            child: Some(child),
            _lifeline: None,
            state_file: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn detach_for_test(&mut self) -> (Option<Child>, Option<ChildStdin>) {
        (self.child.take(), self._lifeline.take())
    }

    pub(crate) fn owned(origin: String, mut child: Child) -> Self {
        if let Some(stdout) = child.stdout.take() {
            tokio::spawn(log_lines(stdout));
        }
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(log_lines(stderr));
        }
        Self {
            origin,
            pgid: child
                .id()
                .and_then(|pid| i32::try_from(pid).ok())
                .unwrap_or_default(),
            started: std::time::Instant::now(),
            _lifeline: child.stdin.take(),
            child: Some(child),
            state_file: None,
        }
    }

    pub(crate) fn uptime(&self) -> Duration {
        self.started.elapsed()
    }

    pub(crate) fn signal_stop(&self) {
        #[cfg(unix)]
        if self.child.is_some() {
            signal_group(self.pgid, libc::SIGTERM);
        }
    }

    pub(crate) fn alive(&mut self) -> bool {
        match &mut self.child {
            Some(child) => matches!(child.try_wait(), Ok(None)),
            None => true,
        }
    }

    pub(crate) async fn stop(mut self) {
        if let Some(mut child) = self.child.take() {
            terminate(&mut child, self.pgid).await;
        }
        if let Some(file) = self.state_file.take() {
            let _ = std::fs::remove_file(file);
        }
    }
}

impl Drop for HubProcess {
    fn drop(&mut self) {
        #[cfg(unix)]
        if self.child.is_some() {
            signal_group(self.pgid, libc::SIGKILL);
        }
        if let Some(file) = self.state_file.take() {
            let _ = std::fs::remove_file(file);
        }
    }
}

#[cfg(unix)]
fn signal_group(pgid: i32, signal: libc::c_int) -> bool {
    // SAFETY: pgid > 1 keeps the negative pid off our own group (0) and every process (-1).
    pgid > 1 && unsafe { libc::kill(-pgid, signal) == 0 }
}

async fn terminate(child: &mut Child, pgid: i32) {
    #[cfg(unix)]
    {
        signal_group(pgid, libc::SIGTERM);
        let deadline = tokio::time::Instant::now() + STOP_GRACE;
        loop {
            let _ = child.try_wait();
            if !signal_group(pgid, 0) {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                signal_group(pgid, libc::SIGKILL);
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    #[cfg(not(unix))]
    let _ = pgid;
    let _ = child.kill().await;
}

fn reserve_port() -> std::io::Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?
        .local_addr()?
        .port())
}

#[cfg(all(test, unix))]
fn pid_alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

fn command_line(pid: u32) -> String {
    std::process::Command::new("ps")
        .args(["-o", "command=", "-p", &pid.to_string()])
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        .unwrap_or_default()
}

fn stale_group(record: &HubStateFile, entry: &str, command_line: &str) -> Option<i32> {
    let pgid = i32::try_from(record.pid).ok().filter(|pid| *pid > 1)?;
    let port = record.port.to_string();
    let listens = command_line
        .split_whitespace()
        .collect::<Vec<_>>()
        .windows(2)
        .any(|pair| pair[0] == "--port" && pair[1] == port);
    (!entry.is_empty() && record.entry_path == entry && command_line.contains(entry) && listens)
        .then_some(pgid)
}

pub(crate) fn reap_stale(state_dir: &Path, entry: &Path) {
    let path = state_dir.join(STATE_FILE);
    let previous = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<HubStateFile>(&text).ok());
    if let Some(previous) = previous {
        let entry = entry.display().to_string();
        if let Some(pgid) = stale_group(&previous, &entry, &command_line(previous.pid)) {
            tracing::warn!(
                pid = previous.pid,
                "stopping a device hub left behind by a previous engine"
            );
            #[cfg(unix)]
            signal_group(pgid, libc::SIGTERM);
            #[cfg(not(unix))]
            let _ = pgid;
        }
    }
    let _ = std::fs::remove_file(&path);
}

pub(crate) fn scrub_env(
    command: &mut Command,
    prefix: &str,
    vars: impl IntoIterator<Item = std::ffi::OsString>,
) {
    for name in vars {
        if name.to_string_lossy().starts_with(prefix) {
            command.env_remove(name);
        }
    }
}

pub(crate) fn scrub_adb_env(
    command: &mut Command,
    vars: impl IntoIterator<Item = std::ffi::OsString>,
) {
    for name in vars {
        let text = name.to_string_lossy();
        if text.starts_with("SERVE_EMU_")
            || matches!(
                &*text,
                "ADB_SERVER_SOCKET" | "ANDROID_ADB_SERVER_ADDRESS" | "ANDROID_SERIAL"
            )
        {
            command.env_remove(&name);
        }
    }
}

fn write_state_file(path: &Path, record: &HubStateFile) {
    use std::io::Write as _;
    let Ok(json) = serde_json::to_string(record) else {
        return;
    };
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let _ = options
        .open(path)
        .and_then(|mut file| file.write_all(json.as_bytes()));
}

pub(crate) fn guarded_args(state_dir: &Path, args: &[String]) -> Result<Vec<String>, String> {
    use std::io::Write as _;
    let failed = |e: std::io::Error| format!("writing the device hub guard: {e}");
    std::fs::create_dir_all(state_dir).map_err(failed)?;
    let path = state_dir.join(GUARD_FILE);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options
        .open(&path)
        .and_then(|mut file| file.write_all(GUARD.as_bytes()))
        .map_err(failed)?;
    let url = reqwest::Url::from_file_path(&path)
        .map_err(|()| format!("{} is not an absolute path", path.display()))?;
    Ok(["--import".to_string(), url.to_string()]
        .into_iter()
        .chain(args.iter().cloned())
        .collect())
}

pub(crate) fn lifeline_command(program: &Path) -> Command {
    #[cfg(unix)]
    let mut command = {
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", LIFELINE, "sh"])
            .arg(std::process::id().to_string())
            .arg(program);
        command.process_group(0);
        command
    };
    #[cfg(not(unix))]
    let mut command = Command::new(program);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

pub(crate) async fn spawn(
    http: &reqwest::Client,
    program: &Path,
    args: &[String],
    entry: &Path,
    state_dir: &Path,
    ready_timeout: Duration,
    env: Vec<(&'static str, std::ffi::OsString)>,
) -> Result<HubProcess, String> {
    reap_stale(state_dir, entry);
    std::fs::create_dir_all(state_dir).map_err(|e| e.to_string())?;
    let port = reserve_port().map_err(|e| format!("reserving a port for the device hub: {e}"))?;
    let mut command = lifeline_command(program);
    command
        .args(args)
        .args(["--port", &port.to_string(), "--host", "127.0.0.1"])
        .args(["--hide-sidebar", "--hide-boot-device"]);
    scrub_env(
        &mut command,
        "EXPO_DEVICE_HUB_",
        std::env::vars_os().map(|(name, _)| name),
    );
    scrub_adb_env(&mut command, std::env::vars_os().map(|(name, _)| name));
    command
        .env("FORCE_COLOR", "0")
        .env("NO_COLOR", "1")
        .envs(env);
    let child = command
        .spawn()
        .map_err(|e| format!("starting the device hub: {e}"))?;
    let mut hub = HubProcess::owned(format!("http://127.0.0.1:{port}"), child);
    let state_file = state_dir.join(STATE_FILE);
    write_state_file(
        &state_file,
        &HubStateFile {
            pid: u32::try_from(hub.pgid).unwrap_or_default(),
            port,
            entry_path: entry.display().to_string(),
        },
    );
    hub.state_file = Some(state_file);
    let ready = match hub.child.as_mut() {
        Some(child) => wait_ready(http, &hub.origin, child, ready_timeout).await,
        None => Ok(()),
    };
    if let Err(error) = ready {
        hub.stop().await;
        return Err(error);
    }
    tracing::info!("device hub started");
    Ok(hub)
}

async fn log_lines(stream: impl tokio::io::AsyncRead + Unpin) {
    let mut lines = BufReader::new(stream).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim();
        if !line.is_empty() {
            tracing::debug!(output = line, "device hub");
        }
    }
}

async fn wait_ready(
    http: &reqwest::Client,
    origin: &str,
    child: &mut Child,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;
    let url = format!("{origin}/readyz");
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!(
                "the device hub exited before it was ready ({status})"
            ));
        }
        let ready = http
            .get(&url)
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success());
        if ready {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "the device hub did not answer within {} s",
                timeout.as_secs()
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    const FAKE_HUB: &str = r#"
import http.server, subprocess, sys, time
port = int(sys.argv[sys.argv.index("--port") + 1])
if sys.argv[2] != "--port":
    helper = subprocess.Popen(["/bin/sleep", "30"])
    open(sys.argv[2], "w").write(str(helper.pid))
time.sleep(float(sys.argv[1]))
class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200 if self.path == "/readyz" else 404)
        self.end_headers()
    def log_message(self, *args):
        pass
http.server.HTTPServer(("127.0.0.1", port), Handler).serve_forever()
"#;

    fn python() -> Option<PathBuf> {
        [
            "/usr/bin/python3",
            "/opt/homebrew/bin/python3",
            "/usr/local/bin/python3",
        ]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
    }

    async fn eventually(mut done: impl FnMut() -> bool) -> bool {
        for _ in 0..300 {
            if done() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
    }

    fn read_pid(path: &Path) -> Option<u32> {
        std::fs::read_to_string(path).ok()?.trim().parse().ok()
    }

    #[tokio::test]
    async fn a_hub_that_answers_readyz_after_a_delay_is_detected_within_the_cap() {
        use std::os::unix::fs::PermissionsExt as _;
        let Some(python) = python() else {
            eprintln!("python3 not found; skipping");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-hub.py");
        std::fs::write(&script, FAKE_HUB).unwrap();
        let args = vec![script.display().to_string(), "0.6".to_string()];
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let state_dir = dir.path().join("device");
        let started = std::time::Instant::now();
        let hub = spawn(
            &http,
            &python,
            &args,
            &script,
            &state_dir,
            Duration::from_secs(20),
            Vec::new(),
        )
        .await
        .expect("hub becomes ready");
        assert!(started.elapsed() >= Duration::from_millis(500));
        let state_file = state_dir.join(STATE_FILE);
        assert_eq!(
            std::fs::metadata(&state_file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let record: HubStateFile =
            serde_json::from_str(&std::fs::read_to_string(&state_file).unwrap()).unwrap();
        assert_eq!(record.entry_path, script.display().to_string());
        assert_eq!(hub.origin, format!("http://127.0.0.1:{}", record.port));
        let pid = record.pid;
        hub.stop().await;
        assert!(!pid_alive(pid));
        assert!(!state_file.exists());

        let late = spawn(
            &http,
            &python,
            &[script.display().to_string(), "5".to_string()],
            &script,
            &state_dir,
            Duration::from_millis(800),
            Vec::new(),
        )
        .await;
        assert!(late.err().unwrap().contains("did not answer"));
    }

    #[tokio::test]
    async fn a_hub_dropped_before_it_is_ready_takes_its_whole_group_down() {
        let Some(python) = python() else {
            eprintln!("python3 not found; skipping");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-hub.py");
        std::fs::write(&script, FAKE_HUB).unwrap();
        let helper_file = dir.path().join("helper.pid");
        let args = vec![
            script.display().to_string(),
            "30".to_string(),
            helper_file.display().to_string(),
        ];
        let state_dir = dir.path().join("device");
        let state_file = state_dir.join(STATE_FILE);
        let task = {
            let (state_dir, script) = (state_dir.clone(), script.clone());
            tokio::spawn(async move {
                let http = reqwest::Client::builder().no_proxy().build().unwrap();
                let _ = spawn(
                    &http,
                    &python,
                    &args,
                    &script,
                    &state_dir,
                    Duration::from_secs(60),
                    Vec::new(),
                )
                .await;
            })
        };
        assert!(eventually(|| read_pid(&helper_file).is_some() && state_file.exists()).await);
        let helper = read_pid(&helper_file).unwrap();
        let record: HubStateFile =
            serde_json::from_str(&std::fs::read_to_string(&state_file).unwrap()).unwrap();
        assert!(pid_alive(record.pid) && pid_alive(helper));
        task.abort();
        let _ = task.await;
        assert!(
            eventually(|| !pid_alive(helper)).await,
            "the hub's helper died with it"
        );
        assert!(!state_file.exists());
    }

    #[tokio::test]
    async fn stop_signals_the_group_after_the_leader_was_reaped() {
        let Some(python) = python() else {
            eprintln!("python3 not found; skipping");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-hub.py");
        std::fs::write(&script, FAKE_HUB).unwrap();
        let helper_file = dir.path().join("helper.pid");
        let args = vec![
            script.display().to_string(),
            "0".to_string(),
            helper_file.display().to_string(),
        ];
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let mut hub = spawn(
            &http,
            &python,
            &args,
            &script,
            &dir.path().join("device"),
            Duration::from_secs(20),
            Vec::new(),
        )
        .await
        .expect("hub becomes ready");
        let helper = read_pid(&helper_file).unwrap();
        unsafe { libc::kill(hub.pgid, libc::SIGKILL) };
        assert!(eventually(|| !hub.alive()).await, "leader reaped");
        hub.stop().await;
        assert!(
            eventually(|| !pid_alive(helper)).await,
            "the helper is signalled through the group"
        );
    }

    fn sleeper(entry: &str, helper_file: &Path) -> std::process::Child {
        use std::os::unix::process::CommandExt as _;
        let script = format!(
            "/bin/sleep 30 & echo $! > {}; /bin/sleep 30; :",
            helper_file.display()
        );
        std::process::Command::new("/bin/sh")
            .args(["-c", &script, entry, "--port", "1"])
            .process_group(0)
            .spawn()
            .unwrap()
    }

    fn write_state(dir: &Path, pid: u32, port: u16, entry: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let record = HubStateFile {
            pid,
            port,
            entry_path: entry.into(),
        };
        std::fs::write(
            dir.join(STATE_FILE),
            serde_json::to_string(&record).unwrap(),
        )
        .unwrap();
    }

    fn exited(child: &mut std::process::Child) -> bool {
        for _ in 0..200 {
            if child.try_wait().unwrap().is_some() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    fn helper_of(file: &Path) -> u32 {
        for _ in 0..200 {
            if let Some(pid) = read_pid(file) {
                return pid;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("sleeper never wrote its helper pid");
    }

    #[test]
    fn a_stale_hub_group_is_stopped_only_when_entry_and_port_match() {
        let dir = tempfile::tempdir().unwrap();
        let entry_path = dir.path().join("expo-device-hub/dist/server/cli.mjs");
        let entry = entry_path.display().to_string();

        let helper_file = dir.path().join("matching.pid");
        let mut matching = sleeper(&entry, &helper_file);
        let helper = helper_of(&helper_file);
        write_state(dir.path(), matching.id(), 1, &entry);
        reap_stale(dir.path(), &entry_path);
        assert!(exited(&mut matching), "matching hub was SIGTERMed");
        assert!(
            (0..200).any(|_| {
                std::thread::sleep(Duration::from_millis(10));
                !pid_alive(helper)
            }),
            "and so was the rest of its group"
        );
        assert!(!dir.path().join(STATE_FILE).exists());

        let mut stranger = sleeper("/somewhere/else.mjs", &dir.path().join("stranger.pid"));
        write_state(dir.path(), stranger.id(), 1, &entry);
        reap_stale(dir.path(), &entry_path);
        assert!(!dir.path().join(STATE_FILE).exists());
        let mut other_port = sleeper(&entry, &dir.path().join("other-port.pid"));
        write_state(dir.path(), other_port.id(), 2, &entry);
        reap_stale(dir.path(), &entry_path);
        let mut other_entry = sleeper(&entry, &dir.path().join("other-entry.pid"));
        write_state(dir.path(), other_entry.id(), 1, &entry);
        reap_stale(dir.path(), &dir.path().join("expo-device-hub/dist/server"));
        for (name, child) in [
            ("stranger", &mut stranger),
            ("other port", &mut other_port),
            ("other entry", &mut other_entry),
        ] {
            assert!(matches!(child.try_wait(), Ok(None)), "{name} left alone");
            unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
            let _ = child.wait();
        }
    }

    #[test]
    fn stale_records_with_unsafe_pids_or_partial_matches_are_refused() {
        let entry = "/adapters/expo-device-hub/cli.mjs";
        let line = format!("node {entry} --port 4100 --host 127.0.0.1");
        let record = |pid, port, entry: &str| HubStateFile {
            pid,
            port,
            entry_path: entry.into(),
        };
        assert_eq!(
            stale_group(&record(4242, 4100, entry), entry, &line),
            Some(4242)
        );
        for (pid, port, recorded, current) in [
            (0, 4100, entry, entry),
            (1, 4100, entry, entry),
            (u32::MAX, 4100, entry, entry),
            (i32::MAX as u32 + 1, 4100, entry, entry),
            (4242, 410, entry, entry),
            (4242, 4100, "", ""),
            (4242, 4100, "/adapters", entry),
            (4242, 4100, entry, "/adapters/other/cli.mjs"),
        ] {
            assert_eq!(
                stale_group(&record(pid, port, recorded), current, &line),
                None,
                "pid {pid} port {port} recorded {recorded:?} current {current:?}"
            );
        }
    }

    #[test]
    fn restarts_back_off_from_zero_to_thirty_seconds_and_reset_after_a_stable_minute() {
        let mut backoff = Backoff::default();
        let quick = Duration::from_secs(5);
        let delays: Vec<u64> = (0..8)
            .map(|_| backoff.after_exit(quick).as_secs())
            .collect();
        assert_eq!(delays, [0, 1, 2, 4, 8, 16, 30, 30]);
        assert_eq!(
            backoff.after_exit(Duration::from_secs(59)),
            Duration::from_secs(30)
        );
        assert_eq!(backoff.after_exit(Duration::from_secs(60)), Duration::ZERO);
        assert_eq!(backoff.after_exit(quick), Duration::from_secs(1));
        assert_eq!(backoff.after_exit(quick), Duration::from_secs(2));
        backoff.reset();
        assert_eq!(backoff.after_exit(quick), Duration::ZERO);
    }

    #[tokio::test]
    async fn the_hub_group_dies_when_the_engine_side_of_its_lifeline_closes() {
        let Some(python) = python() else {
            eprintln!("python3 not found; skipping");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-hub.py");
        std::fs::write(&script, FAKE_HUB).unwrap();
        let helper_file = dir.path().join("helper.pid");
        let args = vec![
            script.display().to_string(),
            "0".to_string(),
            helper_file.display().to_string(),
        ];
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let mut hub = spawn(
            &http,
            &python,
            &args,
            &script,
            &dir.path().join("device"),
            Duration::from_secs(20),
            Vec::new(),
        )
        .await
        .expect("hub becomes ready");
        let helper = read_pid(&helper_file).unwrap();
        let leader = hub.pgid as u32;
        assert!(command_line(leader).contains(&script.display().to_string()));
        assert!(pid_alive(leader) && pid_alive(helper));
        let lifeline = hub._lifeline.take().expect("stdin is the lifeline");
        let mut leader_child = hub.child.take().unwrap();
        drop(hub);
        assert!(pid_alive(leader), "only the lifeline closing may stop it");
        drop(lifeline);
        let status = tokio::time::timeout(Duration::from_secs(5), leader_child.wait())
            .await
            .expect("the hub exits once its lifeline closes")
            .unwrap();
        assert!(!status.success(), "{status:?}");
        assert!(
            eventually(|| !pid_alive(helper)).await,
            "and so does its helper"
        );
    }

    #[tokio::test]
    async fn a_lifeline_without_path_lives_until_its_engine_pid_is_gone() {
        let mut engine = std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", LIFELINE, "sh"])
            .arg(engine.id().to_string())
            .args(["/bin/sleep", "30"])
            .env("PATH", "/nonexistent")
            .stdin(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true);
        let mut hub = command.spawn().unwrap();
        let _lifeline = hub.stdin.take().unwrap();
        let pgid = hub.id().unwrap() as i32;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert!(
            hub.try_wait().unwrap().is_none(),
            "the lifeline holds while its engine lives"
        );
        engine.kill().unwrap();
        engine.wait().unwrap();
        let status = tokio::time::timeout(Duration::from_secs(5), hub.wait())
            .await
            .expect("the hub exits once its engine pid is gone")
            .unwrap();
        assert!(!status.success(), "{status:?}");
        assert!(
            eventually(|| !signal_group(pgid, 0)).await,
            "and so does the rest of its group"
        );
    }

    const FAKE_WS_HUB: &str = r#"import http from "node:http";
import crypto from "node:crypto";
const port = Number(process.argv[process.argv.indexOf("--port") + 1]);
const server = http.createServer((req, res) => { res.writeHead(200); res.end("ok"); });
server.on("upgrade", (req, socket) => {
  const accept = crypto.createHash("sha1")
    .update(req.headers["sec-websocket-key"] + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").digest("base64");
  socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
  socket.write(Buffer.from([0x81, 2, 0x68, 0x69]));
});
server.listen(port, "127.0.0.1");
"#;

    fn node() -> Option<PathBuf> {
        std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .chain(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from))
            .map(|dir| dir.join("node"))
            .find(|node| node.is_file())
    }

    async fn first_reply(port: u16, request: &str) -> String {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        socket.write_all(request.as_bytes()).await.unwrap();
        let mut buf = [0u8; 256];
        let read = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut buf))
            .await
            .expect("the hub answers or closes");
        String::from_utf8_lossy(&buf[..read.unwrap_or(0)]).into_owned()
    }

    #[tokio::test]
    async fn the_hub_guard_refuses_foreign_hosts_and_browser_origins_and_passes_the_engine() {
        use futures::StreamExt as _;
        use std::os::unix::fs::PermissionsExt as _;
        let Some(node) = node() else {
            eprintln!("node not found; skipping");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-hub.mjs");
        std::fs::write(&script, FAKE_WS_HUB).unwrap();
        let state_dir = dir.path().join("device dir");
        let args = guarded_args(&state_dir, &[script.display().to_string()]).unwrap();
        assert_eq!(args[0], "--import");
        assert!(
            args[1].starts_with("file:///") && args[1].ends_with("/device%20dir/hub-guard.mjs")
        );
        let guard = state_dir.join(GUARD_FILE);
        assert_eq!(std::fs::read_to_string(&guard).unwrap(), GUARD);
        assert_eq!(
            std::fs::metadata(&guard).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let hub = spawn(
            &http,
            &node,
            &args,
            &script,
            &state_dir,
            Duration::from_secs(20),
            Vec::new(),
        )
        .await
        .expect("an engine readyz passes the guard");
        let port: u16 = hub.origin.rsplit(':').next().unwrap().parse().unwrap();

        let reply = http
            .get(format!("{}/api/devices", hub.origin))
            .send()
            .await
            .unwrap();
        assert_eq!(reply.status().as_u16(), 200);
        let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws"))
            .await
            .expect("an engine upgrade passes the guard");
        assert_eq!(
            socket.next().await.unwrap().unwrap(),
            tokio_tungstenite::tungstenite::Message::Text("hi".into())
        );

        let get = |host: &str| format!("GET /api/devices HTTP/1.1\r\nHost: {host}\r\n\r\n");
        for host in [format!("localhost:{port}"), format!("[::1]:{port}")] {
            assert!(
                first_reply(port, &get(&host))
                    .await
                    .starts_with("HTTP/1.1 200"),
                "{host}"
            );
        }
        for host in [
            "evil.example".to_string(),
            format!("evil.example:{port}"),
            format!("127.0.0.1:{}", port.wrapping_add(1)),
        ] {
            assert!(
                first_reply(port, &get(&host))
                    .await
                    .starts_with("HTTP/1.1 403"),
                "{host}"
            );
        }
        let cross_origin_post = format!(
            "POST /api/devices/boot HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://evil.example\r\nContent-Type: text/plain\r\nContent-Length: 2\r\n\r\n{{}}"
        );
        assert!(
            first_reply(port, &cross_origin_post)
                .await
                .starts_with("HTTP/1.1 403"),
            "a browser form post carries an Origin"
        );
        let upgrade = |host: &str, extra: &str| {
            format!(
                "GET /ws HTTP/1.1\r\nHost: {host}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n{extra}\r\n"
            )
        };
        let local = format!("127.0.0.1:{port}");
        assert!(
            first_reply(port, &upgrade(&local, ""))
                .await
                .starts_with("HTTP/1.1 101")
        );
        for request in [
            upgrade(&local, "Origin: http://evil.example\r\n"),
            upgrade(&local, &format!("Origin: http://{local}\r\n")),
            upgrade("evil.example", ""),
        ] {
            assert_eq!(first_reply(port, &request).await, "", "{request}");
        }
        hub.stop().await;
    }

    #[test]
    fn adb_runs_drop_inherited_adb_targets_and_serve_emu_variables() {
        let mut command = Command::new("/usr/bin/true");
        scrub_adb_env(
            &mut command,
            [
                "ADB_SERVER_SOCKET",
                "ANDROID_ADB_SERVER_ADDRESS",
                "ANDROID_SERIAL",
                "SERVE_EMU_CAMERA_DIR",
                "ANDROID_HOME",
                "PATH",
            ]
            .map(std::ffi::OsString::from),
        );
        command.env("SERVE_EMU_CAMERA_DIR", "/dev/null/x");
        let env: Vec<_> = command
            .as_std()
            .get_envs()
            .map(|(name, value)| (name.to_string_lossy().into_owned(), value.is_some()))
            .collect();
        assert_eq!(
            env,
            [
                ("ADB_SERVER_SOCKET".to_string(), false),
                ("ANDROID_ADB_SERVER_ADDRESS".to_string(), false),
                ("ANDROID_SERIAL".to_string(), false),
                ("SERVE_EMU_CAMERA_DIR".to_string(), true),
            ]
        );
    }

    #[test]
    fn hub_env_drops_every_expo_device_hub_variable() {
        let mut command = Command::new("/usr/bin/true");
        scrub_env(
            &mut command,
            "EXPO_DEVICE_HUB_",
            [
                "EXPO_DEVICE_HUB_BASE_PATH",
                "PATH",
                "EXPO_DEVICE_HUB_SERVE_SIM_OPTIONS",
                "HOME",
            ]
            .map(std::ffi::OsString::from),
        );
        let removed: Vec<_> = command
            .as_std()
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(name, _)| name.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            removed,
            [
                "EXPO_DEVICE_HUB_BASE_PATH",
                "EXPO_DEVICE_HUB_SERVE_SIM_OPTIONS"
            ]
        );
    }
}
