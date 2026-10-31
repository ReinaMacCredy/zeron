//! Keeps this Mac from idle-sleeping while a local session is Working.
//! `caffeinate -i` leaves display and lid-close sleep alone; `-w` drops the
//! hold if the engine dies without releasing it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::time::Instant;
use zeron_proto::Session;
use zeron_proto::view::{Indicator, effective_indicator};

const RELEASE_GRACE: Duration = Duration::from_secs(60);
const RECHECK: Duration = Duration::from_secs(30);

fn working(sessions: &[Session], now: DateTime<Utc>) -> bool {
    sessions
        .iter()
        .any(|s| effective_indicator(Some(s), now) == Indicator::Working)
}

fn preference_on(data_dir: &Path) -> bool {
    std::fs::read(data_dir.join("ui-settings.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|v| v.get("keepAwakeWhileWorking").and_then(|v| v.as_bool()))
        .unwrap_or(true)
}

#[derive(Default)]
struct Hold {
    child: Option<tokio::process::Child>,
    idle_since: Option<Instant>,
}

impl Hold {
    fn update(&mut self, wanted: bool, now: Instant) {
        if self
            .child
            .as_mut()
            .is_some_and(|c| c.try_wait().ok().flatten().is_some())
        {
            self.child = None;
        }
        if wanted {
            self.idle_since = None;
            if self.child.is_none() {
                self.child = acquire();
            }
        } else if self.child.is_some() {
            let since = *self.idle_since.get_or_insert(now);
            if now.saturating_duration_since(since) >= RELEASE_GRACE {
                self.child = None;
                self.idle_since = None;
            }
        }
    }

    fn release_now(&mut self) {
        self.child = None;
        self.idle_since = None;
    }
}

#[cfg(target_os = "macos")]
fn acquire() -> Option<tokio::process::Child> {
    tokio::process::Command::new("/usr/bin/caffeinate")
        .args(["-i", "-w", &std::process::id().to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .inspect_err(|err| tracing::warn!(error = %err, "keep-awake: caffeinate failed to start"))
        .ok()
}

#[cfg(not(target_os = "macos"))]
fn acquire() -> Option<tokio::process::Child> {
    None
}

pub(crate) fn spawn(core: &crate::EngineCore, data_dir: PathBuf) -> tokio::task::JoinHandle<()> {
    let mut sessions = core.sessions.watch_sessions();
    tokio::spawn(async move {
        let mut hold = Hold::default();
        let mut tick = tokio::time::interval(RECHECK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let busy = working(&sessions.borrow_and_update(), Utc::now());
            if busy && !preference_on(&data_dir) {
                hold.release_now();
            } else {
                hold.update(busy, Instant::now());
            }
            tokio::select! {
                changed = sessions.changed() => {
                    if changed.is_err() { break; }
                }
                _ = tick.tick() => {}
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preference_defaults_on_and_reads_the_flag() {
        let dir = tempfile::tempdir().unwrap();
        assert!(preference_on(dir.path()));
        std::fs::write(
            dir.path().join("ui-settings.json"),
            r#"{"keepAwakeWhileWorking": false}"#,
        )
        .unwrap();
        assert!(!preference_on(dir.path()));
    }

    #[test]
    fn no_sessions_is_not_working() {
        assert!(!working(&[], Utc::now()));
    }
}
