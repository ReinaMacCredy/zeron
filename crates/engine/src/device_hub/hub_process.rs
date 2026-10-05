use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

pub(crate) const READY_TIMEOUT: Duration = Duration::from_secs(30);
const STOP_GRACE: Duration = Duration::from_secs(5);
pub(crate) const MAX_DIMENSION: u32 = 1280;
const STATE_FILE: &str = "hub.json";

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HubStateFile {
    pid: u32,
    port: u16,
    entry_path: String,
}

pub(crate) struct HubProcess {
    pub origin: String,
    child: Option<Child>,
    state_file: Option<PathBuf>,
}

impl HubProcess {
    #[cfg(test)]
    pub(crate) fn external(origin: String) -> Self {
        Self {
            origin,
            child: None,
            state_file: None,
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
            terminate(&mut child).await;
        }
        if let Some(file) = self.state_file.take() {
            let _ = std::fs::remove_file(file);
        }
    }
}

async fn terminate(child: &mut Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        let group = -(pid as i32);
        // SAFETY: signals only the hub's own process group.
        unsafe { libc::kill(group, libc::SIGTERM) };
        let deadline = tokio::time::Instant::now() + STOP_GRACE;
        loop {
            let _ = child.try_wait();
            if unsafe { libc::kill(group, 0) } != 0 {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                unsafe { libc::kill(group, libc::SIGKILL) };
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    let _ = child.kill().await;
}

fn reserve_port() -> std::io::Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?
        .local_addr()?
        .port())
}

fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

fn command_line(pid: u32) -> String {
    std::process::Command::new("ps")
        .args(["-o", "command=", "-p", &pid.to_string()])
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        .unwrap_or_default()
}

pub(crate) fn reap_stale(state_dir: &Path) {
    let path = state_dir.join(STATE_FILE);
    let Some(previous) = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<HubStateFile>(&text).ok())
    else {
        let _ = std::fs::remove_file(&path);
        return;
    };
    if pid_alive(previous.pid) && command_line(previous.pid).contains(&previous.entry_path) {
        tracing::warn!(
            pid = previous.pid,
            port = previous.port,
            "stopping a device hub left behind by a previous engine"
        );
        #[cfg(unix)]
        // SAFETY: the pid was verified by its command line just above.
        unsafe {
            libc::kill(previous.pid as i32, libc::SIGTERM);
        }
    }
    let _ = std::fs::remove_file(&path);
}

pub(crate) async fn spawn(
    http: &reqwest::Client,
    program: &Path,
    args: &[String],
    entry: &Path,
    state_dir: &Path,
    ready_timeout: Duration,
) -> Result<HubProcess, String> {
    reap_stale(state_dir);
    std::fs::create_dir_all(state_dir).map_err(|e| e.to_string())?;
    let port = reserve_port().map_err(|e| format!("reserving a port for the device hub: {e}"))?;
    let mut command = Command::new(program);
    command
        .args(args)
        .args(["--port", &port.to_string(), "--host", "127.0.0.1"])
        .args(["--hide-sidebar", "--hide-boot-device"])
        .args(["--max-dimension", &MAX_DIMENSION.to_string()])
        .env("FORCE_COLOR", "0")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .spawn()
        .map_err(|e| format!("starting the device hub: {e}"))?;
    if let Some(stdout) = child.stdout.take() {
        tokio::spawn(log_lines(stdout));
    }
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(log_lines(stderr));
    }
    let origin = format!("http://127.0.0.1:{port}");
    if let Err(error) = wait_ready(http, &origin, &mut child, ready_timeout).await {
        terminate(&mut child).await;
        return Err(error);
    }
    let state_file = state_dir.join(STATE_FILE);
    if let Some(pid) = child.id() {
        let record = HubStateFile {
            pid,
            port,
            entry_path: entry.display().to_string(),
        };
        if let Ok(json) = serde_json::to_string(&record) {
            let _ = std::fs::write(&state_file, json);
        }
    }
    tracing::info!(port, "device hub started");
    Ok(HubProcess {
        origin,
        child: Some(child),
        state_file: Some(state_file),
    })
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
import http.server, sys, time
port = int(sys.argv[sys.argv.index("--port") + 1])
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

    #[tokio::test]
    async fn a_hub_that_answers_readyz_after_a_delay_is_detected_within_the_cap() {
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
        )
        .await
        .expect("hub becomes ready");
        assert!(started.elapsed() >= Duration::from_millis(500));
        let record: HubStateFile =
            serde_json::from_str(&std::fs::read_to_string(state_dir.join(STATE_FILE)).unwrap())
                .unwrap();
        assert_eq!(record.entry_path, script.display().to_string());
        assert_eq!(hub.origin, format!("http://127.0.0.1:{}", record.port));
        let pid = record.pid;
        hub.stop().await;
        assert!(!pid_alive(pid));
        assert!(!state_dir.join(STATE_FILE).exists());

        let late = spawn(
            &http,
            &python,
            &[script.display().to_string(), "5".to_string()],
            &script,
            &state_dir,
            Duration::from_millis(800),
        )
        .await;
        assert!(late.err().unwrap().contains("did not answer"));
    }

    fn sleeper(entry: &str) -> std::process::Child {
        std::process::Command::new("/bin/sh")
            .args(["-c", "sleep 30; :", entry])
            .spawn()
            .unwrap()
    }

    fn write_state(dir: &Path, pid: u32, entry: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let record = HubStateFile {
            pid,
            port: 1,
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

    #[test]
    fn a_stale_hub_is_stopped_only_when_its_command_line_matches() {
        let dir = tempfile::tempdir().unwrap();
        let entry = dir.path().join("expo-device-hub/dist/server/cli.mjs");
        let entry = entry.display().to_string();

        let mut matching = sleeper(&entry);
        write_state(dir.path(), matching.id(), &entry);
        reap_stale(dir.path());
        assert!(exited(&mut matching), "matching hub was SIGTERMed");
        assert!(!dir.path().join(STATE_FILE).exists());

        let mut stranger = sleeper("/somewhere/else.mjs");
        write_state(dir.path(), stranger.id(), &entry);
        reap_stale(dir.path());
        assert!(!dir.path().join(STATE_FILE).exists());
        assert!(
            matches!(stranger.try_wait(), Ok(None)),
            "stranger left alone"
        );
        stranger.kill().unwrap();
        let _ = stranger.wait();
    }
}
