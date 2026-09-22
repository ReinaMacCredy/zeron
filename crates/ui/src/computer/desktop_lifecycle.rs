use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::future::BoxFuture;

pub trait ComputerDesktopLifecycleHost: Send + Sync + 'static {
    fn pause_desktop(&self, reason: &'static str) -> BoxFuture<'static, Result<(), String>>;
    fn resume_desktop(&self, reason: &'static str);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopPowerEvent {
    LockScreen,
    UnlockScreen,
    Suspend,
    Resume,
    UserDidResignActive,
    UserDidBecomeActive,
}

pub const SCREEN_LOCK: &str = "screen-lock";
pub const SYSTEM_SLEEP: &str = "system-sleep";
pub const USER_SESSION: &str = "user-session";

impl DesktopPowerEvent {
    fn gate(self) -> (bool, &'static str) {
        match self {
            Self::LockScreen => (true, SCREEN_LOCK),
            Self::UnlockScreen => (false, SCREEN_LOCK),
            Self::Suspend => (true, SYSTEM_SLEEP),
            Self::Resume => (false, SYSTEM_SLEEP),
            Self::UserDidResignActive => (true, USER_SESSION),
            Self::UserDidBecomeActive => (false, USER_SESSION),
        }
    }
}

pub struct ComputerDesktopLifecycle {
    host: Arc<dyn ComputerDesktopLifecycleHost>,
    runtime: tokio::runtime::Handle,
    on_error: Arc<dyn Fn(String) + Send + Sync>,
    registered: AtomicBool,
}

impl ComputerDesktopLifecycle {
    pub fn register(
        host: Arc<dyn ComputerDesktopLifecycleHost>,
        runtime: tokio::runtime::Handle,
        on_error: Arc<dyn Fn(String) + Send + Sync>,
        already_locked: bool,
    ) -> Self {
        let lifecycle = Self {
            host,
            runtime,
            on_error,
            registered: AtomicBool::new(true),
        };
        if already_locked {
            lifecycle.pause(SCREEN_LOCK);
        }
        lifecycle
    }

    fn pause(&self, reason: &'static str) {
        let pausing = self.host.pause_desktop(reason);
        let on_error = self.on_error.clone();
        self.runtime.spawn(async move {
            if let Err(error) = pausing.await {
                on_error(error);
            }
        });
    }

    pub fn handle(&self, event: DesktopPowerEvent) {
        if !self.registered.load(Ordering::SeqCst) {
            return;
        }
        match event.gate() {
            (true, reason) => self.pause(reason),
            (false, reason) => self.host.resume_desktop(reason),
        }
    }

    pub fn dispose(&self) {
        self.registered.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Recorder {
        reasons: Mutex<BTreeSet<&'static str>>,
        paused: Mutex<Vec<&'static str>>,
        fail: bool,
    }

    impl ComputerDesktopLifecycleHost for Recorder {
        fn pause_desktop(&self, reason: &'static str) -> BoxFuture<'static, Result<(), String>> {
            self.reasons.lock().unwrap().insert(reason);
            self.paused.lock().unwrap().push(reason);
            let fail = self.fail;
            Box::pin(async move {
                if fail {
                    Err("Native cleanup not acknowledged".into())
                } else {
                    Ok(())
                }
            })
        }

        fn resume_desktop(&self, reason: &'static str) {
            self.reasons.lock().unwrap().remove(reason);
        }
    }

    #[tokio::test]
    async fn keeps_lock_sleep_and_user_session_gates_independent_and_stops_after_dispose() {
        let host = Arc::new(Recorder::default());
        let lifecycle = ComputerDesktopLifecycle::register(
            host.clone(),
            tokio::runtime::Handle::current(),
            Arc::new(|error| panic!("unexpected cleanup failure: {error}")),
            false,
        );
        lifecycle.handle(DesktopPowerEvent::LockScreen);
        lifecycle.handle(DesktopPowerEvent::Suspend);
        lifecycle.handle(DesktopPowerEvent::UserDidResignActive);
        lifecycle.handle(DesktopPowerEvent::Resume);
        lifecycle.handle(DesktopPowerEvent::UnlockScreen);
        assert_eq!(
            host.reasons
                .lock()
                .unwrap()
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![USER_SESSION]
        );
        lifecycle.handle(DesktopPowerEvent::UserDidBecomeActive);
        assert!(host.reasons.lock().unwrap().is_empty());
        lifecycle.dispose();
        lifecycle.handle(DesktopPowerEvent::LockScreen);
        assert!(host.reasons.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn blocks_an_already_locked_desktop_and_reports_failed_native_cleanup() {
        let host = Arc::new(Recorder {
            fail: true,
            ..Recorder::default()
        });
        let errors = Arc::new(Mutex::new(Vec::new()));
        let recorded = errors.clone();
        let lifecycle = ComputerDesktopLifecycle::register(
            host.clone(),
            tokio::runtime::Handle::current(),
            Arc::new(move |error| recorded.lock().unwrap().push(error)),
            true,
        );
        for _ in 0..100 {
            if !errors.lock().unwrap().is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(*host.paused.lock().unwrap(), vec![SCREEN_LOCK]);
        assert_eq!(
            *errors.lock().unwrap(),
            vec!["Native cleanup not acknowledged".to_string()]
        );
        lifecycle.dispose();
    }
}
