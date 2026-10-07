use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::rc::Rc;
use std::time::Duration;

use futures::FutureExt;
use tokio::io::AsyncBufReadExt;
use tokio::process::{ChildStdin, Command};
use tokio::sync::mpsc;

use super::Chain;
use super::support::{CUA_CURSOR_IDLE_HIDE_MS, CUA_CURSOR_THEME};
use crate::computer::runtime_ownership::{
    RuntimeSweep, cua_host_process_is_alive, sweep_owned_cua_runtime_directories,
};

pub(super) const TERMINATE_GRACE: Duration = Duration::from_millis(500);
pub(super) const TERMINATE_FORCE: Duration = Duration::from_millis(1_500);
pub(super) const TERMINATE_GIVE_UP: Duration = Duration::from_millis(4_000);

#[derive(Debug, Clone, Copy)]
enum ChildSignal {
    Terminate,
    Kill,
}

pub(super) struct DriverChild {
    pub pid: u32,
    pub did_exit: Rc<Cell<bool>>,
    pub exited: Chain,
    stdin: RefCell<Option<ChildStdin>>,
    signals: mpsc::UnboundedSender<ChildSignal>,
}

pub(super) struct SpawnPlan<'a> {
    pub binary: &'a Path,
    pub socket: &'a Path,
    pub runtime_dir: &'a Path,
    pub bundle_id: &'a str,
    pub patched: bool,
}

impl SpawnPlan<'_> {
    pub fn args(&self) -> Vec<String> {
        let mut args = vec![
            "serve".to_string(),
            "--embedded".into(),
            "--socket".into(),
            self.socket.to_string_lossy().into_owned(),
            "--grant".into(),
            "existing-profile".into(),
        ];
        if self.patched {
            args.extend([
                "--compact-cursor".into(),
                "--idle-hide-ms".into(),
                CUA_CURSOR_IDLE_HIDE_MS.to_string(),
                "--cursor-theme".into(),
                CUA_CURSOR_THEME.into(),
            ]);
        }
        args
    }

    pub fn env(&self) -> Vec<(&'static str, String)> {
        vec![
            ("CUA_DRIVER_EMBEDDED", "1".into()),
            ("CUA_DRIVER_HOST_BUNDLE_ID", self.bundle_id.into()),
            ("CUA_DRIVER_PERMISSION_MODE", "standard".into()),
            ("CUA_DRIVER_RS_TELEMETRY_ENABLED", "0".into()),
            ("CUA_DRIVER_RS_UPDATE_CHECK", "0".into()),
            ("SYNARA_CUA_FOREGROUND_OBSERVATION_MS", "100".into()),
            ("SYNARA_CUA_BACKGROUND_OBSERVATION_MS", "350".into()),
            ("CUA_DRIVER_PARENT_LIVENESS_STDIN", "1".into()),
            (
                "CUA_DRIVER_EMBEDDED_HOST_PID",
                std::process::id().to_string(),
            ),
            (
                "CUA_DRIVER_RS_HOME",
                self.runtime_dir
                    .join("state")
                    .to_string_lossy()
                    .into_owned(),
            ),
        ]
    }
}

fn stderr_event(line: &str) -> Option<String> {
    if let Some(code) = line.strip_prefix("synara_cua_overlay_init code=")
        && matches!(
            code,
            "overlay_display_unavailable" | "overlay_window_unavailable"
        )
    {
        return Some(
            serde_json::json!({ "event": "computer_cursor_init", "ts": super::now_iso(), "code": code })
                .to_string(),
        );
    }
    if let Some(status) = line.strip_prefix("synara_cua_focus_restore status=")
        && matches!(
            status,
            "not-needed" | "restored" | "failed" | "unobservable" | "user-changed"
        )
    {
        return Some(
            serde_json::json!({ "event": "computer_focus_restore", "ts": super::now_iso(), "status": status })
                .to_string(),
        );
    }
    None
}

pub(super) fn spawn_driver(plan: &SpawnPlan<'_>) -> std::io::Result<DriverChild> {
    let mut child = Command::new(plan.binary)
        .args(plan.args())
        .envs(plan.env())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let pid = child.id().unwrap_or_default();
    let stdin = child.stdin.take();
    let stderr = child.stderr.take();
    let tail = Rc::new(RefCell::new(Vec::<String>::new()));
    if let Some(stderr) = stderr {
        let tail = tail.clone();
        tokio::task::spawn_local(async move {
            let mut lines = tokio::io::BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                if let Some(event) = stderr_event(&line) {
                    super::log(&event);
                }
                let mut tail = tail.borrow_mut();
                tail.push(line.chars().take(200).collect());
                if tail.len() > 20 {
                    tail.remove(0);
                }
            }
        });
    }
    let (signals, mut received) = mpsc::unbounded_channel();
    let did_exit = Rc::new(Cell::new(false));
    let exited_flag = did_exit.clone();
    let reaper = tokio::task::spawn_local(async move {
        loop {
            tokio::select! {
                _ = child.wait() => break,
                Some(signal) = received.recv() => match signal {
                    ChildSignal::Terminate => {
                        if let Ok(pid) = libc::pid_t::try_from(pid) {
                            unsafe { libc::kill(pid, libc::SIGTERM) };
                        }
                    }
                    ChildSignal::Kill => {
                        let _ = child.start_kill();
                    }
                },
            }
        }
        exited_flag.set(true);
        let tail = tail.borrow();
        if !tail.is_empty() {
            super::log(&format!("driver stderr tail: {}", tail.join(" | ")));
        }
    });
    let exited = async move {
        let _ = reaper.await;
    }
    .boxed_local()
    .shared();
    Ok(DriverChild {
        pid,
        did_exit,
        exited,
        stdin: RefCell::new(stdin),
        signals,
    })
}

impl DriverChild {
    pub async fn terminate(&self) {
        if self.did_exit.get() {
            return;
        }
        drop(self.stdin.borrow_mut().take());
        let signals = self.signals.clone();
        let timeline = async move {
            tokio::time::sleep(TERMINATE_GRACE).await;
            let _ = signals.send(ChildSignal::Terminate);
            tokio::time::sleep(TERMINATE_FORCE - TERMINATE_GRACE).await;
            let _ = signals.send(ChildSignal::Kill);
            tokio::time::sleep(TERMINATE_GIVE_UP - TERMINATE_FORCE).await;
        };
        tokio::select! {
            () = self.exited.clone() => {}
            () = timeline => {}
        }
        if !self.did_exit.get() {
            super::log(&format!(
                "driver pid={} did not exit after SIGKILL; releasing the generation anyway",
                self.pid
            ));
        }
    }
}

pub(super) struct OrphanCandidate {
    pub pid: u32,
    pub socket_dir: Option<PathBuf>,
}

pub(super) fn embedded_driver_processes(listing: &str, own_pid: u32) -> Vec<OrphanCandidate> {
    listing
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let pid: u32 = words.next()?.parse().ok()?;
            let args: Vec<&str> = words.collect();
            let serve = args
                .windows(3)
                .any(|w| w[0].ends_with("cua-driver") && w[1] == "serve" && w[2] == "--embedded");
            if !serve || pid == 0 || pid == own_pid {
                return None;
            }
            let socket_dir = args
                .windows(2)
                .find(|w| w[0] == "--socket")
                .and_then(|w| w[1].rsplit_once('/'))
                .map(|(dir, _)| PathBuf::from(dir));
            Some(OrphanCandidate { pid, socket_dir })
        })
        .collect()
}

pub(super) fn embedded_host_pid(environment: &str) -> Option<u32> {
    let start = environment.find("CUA_DRIVER_EMBEDDED_HOST_PID=")? + 29;
    let digits: String = environment[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok().filter(|pid| *pid > 0)
}

fn ps(args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("ps").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn sweep_orphaned_cua_drivers() {
    let Some(listing) = ps(&["-axo", "pid,args"]) else {
        return;
    };
    let mut live_socket_dirs = HashSet::new();
    for candidate in embedded_driver_processes(&listing, std::process::id()) {
        if let Some(dir) = candidate.socket_dir {
            live_socket_dirs.insert(dir);
        }
        let Some(environment) = ps(&["eww", "-p", &candidate.pid.to_string(), "-o", "command"])
        else {
            continue;
        };
        let Some(host_pid) = embedded_host_pid(&environment) else {
            continue;
        };
        if cua_host_process_is_alive(host_pid) {
            continue;
        }
        if let Ok(pid) = libc::pid_t::try_from(candidate.pid)
            && unsafe { libc::kill(pid, libc::SIGKILL) } == 0
        {
            super::log(&format!(
                "killed orphaned cua-driver pid={} (host pid {host_pid} gone)",
                candidate.pid
            ));
        }
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default();
    for entry in sweep_owned_cua_runtime_directories(RuntimeSweep {
        directory: &std::env::temp_dir(),
        live_socket_dirs: &live_socket_dirs,
        now_ms,
        is_process_alive: &cua_host_process_is_alive,
    }) {
        super::log(&format!("removed stale owned driver directory {entry}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_plan_matches_the_embedded_host_contract() {
        let plan = SpawnPlan {
            binary: Path::new("/b/cua-driver"),
            socket: Path::new("/t/zeron-cua-abc123/driver-1.sock"),
            runtime_dir: Path::new("/t/zeron-cua-abc123"),
            bundle_id: "sh.zeron.app",
            patched: true,
        };
        assert_eq!(
            plan.args(),
            [
                "serve",
                "--embedded",
                "--socket",
                "/t/zeron-cua-abc123/driver-1.sock",
                "--grant",
                "existing-profile",
                "--compact-cursor",
                "--idle-hide-ms",
                "60000",
                "--cursor-theme",
                "zeron.codex"
            ]
        );
        let unpatched = SpawnPlan {
            patched: false,
            ..plan
        };
        assert_eq!(unpatched.args().len(), 6);
        let env = plan.env();
        let get = |key: &str| env.iter().find(|(k, _)| *k == key).map(|(_, v)| v.as_str());
        assert_eq!(get("CUA_DRIVER_EMBEDDED"), Some("1"));
        assert_eq!(get("CUA_DRIVER_HOST_BUNDLE_ID"), Some("sh.zeron.app"));
        assert_eq!(get("CUA_DRIVER_PERMISSION_MODE"), Some("standard"));
        assert_eq!(get("CUA_DRIVER_RS_TELEMETRY_ENABLED"), Some("0"));
        assert_eq!(get("CUA_DRIVER_RS_UPDATE_CHECK"), Some("0"));
        assert_eq!(get("SYNARA_CUA_FOREGROUND_OBSERVATION_MS"), Some("100"));
        assert_eq!(get("SYNARA_CUA_BACKGROUND_OBSERVATION_MS"), Some("350"));
        assert_eq!(get("CUA_DRIVER_PARENT_LIVENESS_STDIN"), Some("1"));
        assert_eq!(
            get("CUA_DRIVER_EMBEDDED_HOST_PID"),
            Some(std::process::id().to_string().as_str())
        );
        assert_eq!(get("CUA_DRIVER_RS_HOME"), Some("/t/zeron-cua-abc123/state"));
    }

    #[test]
    fn finds_embedded_drivers_and_their_socket_dirs_in_a_ps_listing() {
        let listing = "  PID ARGS\n  12 /Applications/Zeron.app/Contents/Resources/cua-driver/cua-driver serve --embedded --socket /tmp/zeron-cua-abc123/driver-1.sock --compact-cursor\n  13 /bin/zsh\n  14 cua-driver serve --socket /x/y.sock\n  99 cua-driver serve --embedded --socket /tmp/zeron-cua-self/driver.sock\n";
        let found = embedded_driver_processes(listing, 99);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].pid, 12);
        assert_eq!(
            found[0].socket_dir.as_deref(),
            Some(Path::new("/tmp/zeron-cua-abc123"))
        );
        assert_eq!(
            embedded_host_pid("cua-driver serve A=1 CUA_DRIVER_EMBEDDED_HOST_PID=4242 B=2"),
            Some(4242)
        );
        assert_eq!(embedded_host_pid("cua-driver serve"), None);
    }

    #[test]
    fn stderr_literals_become_structured_events_and_other_lines_do_not() {
        assert!(
            stderr_event("synara_cua_overlay_init code=overlay_window_unavailable")
                .unwrap()
                .contains("\"computer_cursor_init\"")
        );
        assert!(
            stderr_event("synara_cua_focus_restore status=user-changed")
                .unwrap()
                .contains("\"user-changed\"")
        );
        assert!(stderr_event("synara_cua_focus_restore status=private text").is_none());
        assert!(stderr_event("panic at the disco").is_none());
    }
}
