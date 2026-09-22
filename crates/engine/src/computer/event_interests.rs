use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use zeron_proto::computer::ComputerEvent;

pub const MAX_COMPUTER_THREAD_INTERESTS_PER_CONNECTION: usize = 64;

pub type ConnectionCleanup = Box<dyn FnOnce() + Send>;
pub type OnConnectionClose = Arc<dyn Fn(&str, ConnectionCleanup) -> bool + Send + Sync>;
pub type ComputerEventListener = Arc<dyn Fn(&ComputerEvent) + Send + Sync>;

// `None` is the broadcast fallback past the per-connection view budget.
type Connections = HashMap<String, Option<HashSet<String>>>;

#[derive(Clone)]
pub struct ComputerEventInterests {
    connections: Arc<Mutex<Connections>>,
    on_connection_close: OnConnectionClose,
}

impl ComputerEventInterests {
    pub fn new(on_connection_close: OnConnectionClose) -> Self {
        Self {
            connections: Arc::default(),
            on_connection_close,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connections> {
        self.connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn connect(&self, connection_key: Option<&str>) {
        let Some(key) = connection_key else {
            return;
        };
        if self.lock().contains_key(key) {
            return;
        }
        let connections = Arc::downgrade(&self.connections);
        let owned = key.to_owned();
        let cleanup: ConnectionCleanup = Box::new(move || {
            if let Some(connections) = connections.upgrade() {
                connections
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&owned);
            }
        });
        if (self.on_connection_close)(key, cleanup) {
            self.lock()
                .entry(key.to_owned())
                .or_insert_with(|| Some(HashSet::new()));
        }
    }

    pub fn subscribe<S>(
        &self,
        connection_key: Option<&str>,
        on_event: impl FnOnce(ComputerEventListener) -> S,
        listener: ComputerEventListener,
    ) -> S {
        self.connect(connection_key);
        let interests = self.clone();
        let key = connection_key.map(str::to_owned);
        on_event(Arc::new(move |event| {
            if interests.accepts(key.as_deref(), event) {
                listener(event);
            }
        }))
    }

    pub fn watch(&self, connection_key: Option<&str>, thread_id: &str) {
        self.connect(connection_key);
        let Some(key) = connection_key else {
            return;
        };
        let mut connections = self.lock();
        let Some(interests) = connections.get_mut(key) else {
            return;
        };
        let Some(threads) = interests else {
            return;
        };
        threads.insert(thread_id.to_owned());
        if threads.len() > MAX_COMPUTER_THREAD_INTERESTS_PER_CONNECTION {
            *interests = None;
        }
    }

    pub fn accepts(&self, connection_key: Option<&str>, event: &ComputerEvent) -> bool {
        let Some(key) = connection_key else {
            return true;
        };
        let connections = self.lock();
        let Some(interests) = connections.get(key) else {
            return false;
        };
        let thread_id = match event {
            ComputerEvent::ThreadState { state } => Some(state.thread_id.as_str()),
            ComputerEvent::Action { thread_id, .. } => thread_id.as_deref(),
            _ => None,
        };
        match (thread_id, interests) {
            (None, _) | (_, None) => true,
            (Some(thread_id), Some(threads)) => threads.contains(thread_id),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    use zeron_proto::computer::{
        ComputerAvailability, ComputerCapabilities, ComputerHealth, ComputerHealthStatus,
        ComputerScreenSize, ThreadComputerState,
    };

    use super::*;

    type Listeners = Arc<Mutex<HashMap<u64, ComputerEventListener>>>;

    #[derive(Default)]
    struct Sessions {
        next: AtomicU64,
        open: Mutex<HashMap<String, Vec<ConnectionCleanup>>>,
    }

    impl Sessions {
        fn register(&self) -> String {
            let key = format!("connection-{}", self.next.fetch_add(1, Ordering::SeqCst));
            self.open.lock().unwrap().insert(key.clone(), Vec::new());
            key
        }

        fn close(&self, key: &str) {
            let cleanups = self.open.lock().unwrap().remove(key).unwrap_or_default();
            for cleanup in cleanups {
                cleanup();
            }
        }

        fn on_close(&self, key: &str, cleanup: ConnectionCleanup) -> bool {
            match self.open.lock().unwrap().get_mut(key) {
                Some(cleanups) => {
                    cleanups.push(cleanup);
                    true
                }
                None => false,
            }
        }
    }

    struct Fixture {
        sessions: Arc<Sessions>,
        interests: ComputerEventInterests,
        cleanup_registrations: Arc<AtomicUsize>,
        listeners: Listeners,
        next_listener: Arc<AtomicU64>,
    }

    impl Fixture {
        fn new() -> Self {
            let sessions = Arc::new(Sessions::default());
            let cleanup_registrations = Arc::new(AtomicUsize::new(0));
            let registry = sessions.clone();
            let registrations = cleanup_registrations.clone();
            let interests = ComputerEventInterests::new(Arc::new(move |key, cleanup| {
                registrations.fetch_add(1, Ordering::SeqCst);
                registry.on_close(key, cleanup)
            }));
            Self {
                sessions,
                interests,
                cleanup_registrations,
                listeners: Listeners::default(),
                next_listener: Arc::default(),
            }
        }

        fn on_event(&self) -> impl FnOnce(ComputerEventListener) -> Box<dyn FnOnce()> + use<> {
            let listeners = self.listeners.clone();
            let id = self.next_listener.fetch_add(1, Ordering::SeqCst);
            move |listener| {
                listeners.lock().unwrap().insert(id, listener);
                Box::new(move || {
                    listeners.lock().unwrap().remove(&id);
                })
            }
        }

        fn emit(&self, event: &ComputerEvent) {
            let listeners: Vec<_> = self.listeners.lock().unwrap().values().cloned().collect();
            for listener in listeners {
                listener(event);
            }
        }

        fn listener_count(&self) -> usize {
            self.listeners.lock().unwrap().len()
        }
    }

    fn state_event(thread_id: &str) -> ComputerEvent {
        ComputerEvent::ThreadState {
            state: Box::new(ThreadComputerState {
                control_generation: None,
                thread_id: thread_id.to_owned(),
                version: 1,
                computer_id: "desktop".to_owned(),
                windows: vec![],
                screen_size: ComputerScreenSize {
                    width: 1_000,
                    height: 800,
                    scale: None,
                },
                cursor: None,
                agent_active: false,
                activity: None,
                input_pause: None,
                control_owner_thread_id: None,
                control_owner_label: None,
                controlled_by_other_thread: false,
                shared_preview_unavailable: None,
                availability: ComputerAvailability::Available { backend: None },
                health: ComputerHealth {
                    status: ComputerHealthStatus::Connected,
                    consecutive_failures: 0,
                    reconnects: 0,
                    last_failure: None,
                    capture_available: true,
                    background_input_degraded: None,
                },
                capabilities: ComputerCapabilities {
                    windows: true,
                    window_bounds: true,
                    stacking: true,
                    capture: true,
                    input: true,
                    clipboard: true,
                    focus: true,
                    raise: true,
                    ghost_cursor: true,
                    visible_desktop: true,
                },
                input_stopped: None,
                last_error: None,
            }),
        }
    }

    fn action_event(thread_id: Option<&str>) -> ComputerEvent {
        ComputerEvent::Action {
            window_id: None,
            delivery: None,
            action: "computer_click".to_owned(),
            ok: true,
            message: None,
            thread_id: thread_id.map(str::to_owned),
        }
    }

    #[test]
    fn filters_thread_state_and_actions_while_preserving_global_and_human_events() {
        let f = Fixture::new();
        let first = f.sessions.register();
        let second = f.sessions.register();
        f.interests.watch(Some(&first), "thread-a");
        f.interests.watch(Some(&second), "thread-b");
        assert!(f.interests.accepts(Some(&first), &state_event("thread-a")));
        assert!(
            f.interests
                .accepts(Some(&first), &action_event(Some("thread-a")))
        );
        assert!(!f.interests.accepts(Some(&first), &state_event("thread-b")));
        assert!(
            !f.interests
                .accepts(Some(&second), &action_event(Some("thread-a")))
        );
        assert!(f.interests.accepts(Some(&first), &action_event(None)));
        assert!(f.interests.accepts(
            Some(&first),
            &ComputerEvent::WindowsChanged { windows: vec![] }
        ));
    }

    #[test]
    fn retains_live_views_beyond_256_connections_and_across_disconnected_client_churn() {
        let f = Fixture::new();
        let first = f.sessions.register();
        f.interests.watch(Some(&first), "first-view");
        let mut live_keys = Vec::new();
        for index in 0..300 {
            let live = f.sessions.register();
            f.interests.watch(Some(&live), &format!("view-{index}"));
            live_keys.push(live);
            let transient = f.sessions.register();
            f.interests.watch(Some(&transient), "transient-view");
            f.sessions.close(&transient);
            assert!(
                !f.interests
                    .accepts(Some(&transient), &state_event("transient-view"))
            );
        }
        assert!(
            f.interests
                .accepts(Some(&first), &state_event("first-view"))
        );
        for (index, key) in live_keys.iter().enumerate() {
            assert!(
                f.interests
                    .accepts(Some(key), &state_event(&format!("view-{index}")))
            );
            assert!(!f.interests.accepts(Some(key), &state_event("first-view")));
        }
    }

    #[test]
    fn falls_back_to_broadcast_beyond_64_distinct_views_without_dropping_any() {
        let f = Fixture::new();
        let connection = f.sessions.register();
        for index in 0..100 {
            f.interests
                .watch(Some(&connection), &format!("view-{index}"));
        }
        for index in 0..100 {
            assert!(
                f.interests
                    .accepts(Some(&connection), &state_event(&format!("view-{index}")))
            );
        }
        assert!(
            f.interests
                .accepts(Some(&connection), &state_event("another-view"))
        );
        f.sessions.close(&connection);
        assert!(
            !f.interests
                .accepts(Some(&connection), &state_event("view-0"))
        );
    }

    #[test]
    fn does_not_consume_the_distinct_view_budget_on_repeated_state_reads() {
        let f = Fixture::new();
        let connection = f.sessions.register();
        for _ in 0..100 {
            f.interests.watch(Some(&connection), "same-view");
        }
        assert!(
            f.interests
                .accepts(Some(&connection), &state_event("same-view"))
        );
        assert!(
            !f.interests
                .accepts(Some(&connection), &state_event("another-view"))
        );
    }

    #[test]
    fn cleans_state_only_connections_and_rejects_late_reads_after_socket_close() {
        let f = Fixture::new();
        let connection = f.sessions.register();
        f.interests.watch(Some(&connection), "view");
        f.sessions.close(&connection);
        f.interests.watch(Some(&connection), "view");
        assert!(!f.interests.accepts(Some(&connection), &state_event("view")));
        assert!(!f.sessions.on_close(&connection, Box::new(|| {})));
        assert_eq!(f.listener_count(), 0);
    }

    #[test]
    fn keeps_interests_across_stream_retries_but_releases_the_old_event_listener() {
        let f = Fixture::new();
        let connection = f.sessions.register();
        let received = Arc::new(Mutex::new(Vec::new()));
        let sink = received.clone();
        let listener: ComputerEventListener =
            Arc::new(move |event| sink.lock().unwrap().push(event.clone()));
        f.interests.watch(Some(&connection), "view");
        let stop_first = f
            .interests
            .subscribe(Some(&connection), f.on_event(), listener.clone());
        f.emit(&state_event("view"));
        stop_first();
        assert_eq!(f.listener_count(), 0);
        f.emit(&state_event("view"));
        let stop_second = f
            .interests
            .subscribe(Some(&connection), f.on_event(), listener);
        f.emit(&state_event("other"));
        f.emit(&state_event("view"));
        assert_eq!(
            *received.lock().unwrap(),
            [state_event("view"), state_event("view")]
        );
        assert_eq!(f.cleanup_registrations.load(Ordering::SeqCst), 1);
        f.sessions.close(&connection);
        f.emit(&state_event("view"));
        assert_eq!(received.lock().unwrap().len(), 2);
        stop_second();
        assert_eq!(f.listener_count(), 0);
    }

    #[test]
    fn preserves_broadcast_for_callers_without_a_connection_context() {
        let f = Fixture::new();
        let received = Arc::new(Mutex::new(Vec::new()));
        let sink = received.clone();
        f.interests.watch(None, "view");
        let stop = f.interests.subscribe(
            None,
            f.on_event(),
            Arc::new(move |event| sink.lock().unwrap().push(event.clone())),
        );
        f.emit(&state_event("other"));
        assert_eq!(*received.lock().unwrap(), [state_event("other")]);
        stop();
    }
}
