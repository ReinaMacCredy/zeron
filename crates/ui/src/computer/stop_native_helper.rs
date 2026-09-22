use std::time::Duration;

use tokio::process::Child;

pub const HELPER_KILL_AFTER: Duration = Duration::from_secs(1);
pub const HELPER_GIVE_UP_AFTER: Duration = Duration::from_secs(1);

pub async fn stop_native_helper(child: &mut Child) -> Result<(), String> {
    if child.try_wait().map_err(|e| e.to_string())?.is_some() {
        return Ok(());
    }
    if let Some(pid) = child.id().and_then(|pid| libc::pid_t::try_from(pid).ok()) {
        unsafe { libc::kill(pid, libc::SIGTERM) };
    }
    if tokio::time::timeout(HELPER_KILL_AFTER, child.wait())
        .await
        .is_ok()
    {
        return Ok(());
    }
    let _ = child.start_kill();
    if tokio::time::timeout(HELPER_GIVE_UP_AFTER, child.wait())
        .await
        .is_ok()
    {
        return Ok(());
    }
    Err("Native capture helper has not stopped; another capture is blocked until it exits.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use tokio::process::Command;

    #[tokio::test]
    async fn a_helper_that_honours_sigterm_stops_before_the_kill_deadline() {
        let mut child = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let started = Instant::now();
        stop_native_helper(&mut child).await.unwrap();
        assert!(started.elapsed() < HELPER_KILL_AFTER);
    }

    #[tokio::test]
    async fn a_helper_ignoring_sigterm_is_killed_after_the_grace() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "trap '' TERM; exec /bin/sleep 30"])
            .spawn()
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let started = Instant::now();
        stop_native_helper(&mut child).await.unwrap();
        assert!(started.elapsed() >= HELPER_KILL_AFTER);
        assert!(child.try_wait().unwrap().is_some());
    }

    #[tokio::test]
    async fn an_exited_helper_needs_nothing() {
        let mut child = Command::new("/usr/bin/true").spawn().unwrap();
        child.wait().await.unwrap();
        stop_native_helper(&mut child).await.unwrap();
    }
}
