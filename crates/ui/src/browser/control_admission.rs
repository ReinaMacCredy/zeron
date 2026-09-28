use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

#[derive(Default)]
struct State {
    epoch: u64,
    next: u64,
    threads: HashMap<String, u64>,
    pauses: HashSet<&'static str>,
}
#[derive(Default)]
pub(crate) struct Admission(Mutex<State>);
#[derive(Clone)]
pub(crate) struct Stamp {
    thread: String,
    epoch: u64,
    generation: u64,
}
impl Admission {
    pub fn begin(&self, thread: &str) -> Result<Stamp, &'static str> {
        let mut state = self.0.lock().unwrap();
        if !state.pauses.is_empty() {
            return Err("Browser control is paused while the desktop is locked or unavailable.");
        }
        if !state.threads.contains_key(thread) {
            state.next += 1;
            let next = state.next;
            state.threads.insert(thread.into(), next);
        }
        Ok(Stamp {
            thread: thread.into(),
            epoch: state.epoch,
            generation: state.threads[thread],
        })
    }
    pub fn current(&self, stamp: &Stamp) -> bool {
        let state = self.0.lock().unwrap();
        state.pauses.is_empty()
            && state.epoch == stamp.epoch
            && state.threads.get(&stamp.thread) == Some(&stamp.generation)
    }
    pub fn active(&self) -> bool {
        !self.0.lock().unwrap().threads.is_empty()
    }
    pub fn stop(&self, thread: Option<&str>) -> bool {
        let mut state = self.0.lock().unwrap();
        if let Some(thread) = thread {
            return state.threads.remove(thread).is_some();
        }
        let active = !state.threads.is_empty();
        state.epoch += 1;
        state.threads.clear();
        active
    }
    pub fn pause(&self, reason: &'static str) {
        let mut state = self.0.lock().unwrap();
        state.pauses.insert(reason);
        state.epoch += 1;
        state.threads.clear();
    }
    pub fn resume(&self, reason: &'static str) {
        self.0.lock().unwrap().pauses.remove(reason);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scoped_stop_preserves_other_tasks_and_rejects_queued_generation() {
        let gate = Admission::default();
        let a = gate.begin("a").unwrap();
        let b = gate.begin("b").unwrap();
        gate.stop(Some("a"));
        let next = gate.begin("a").unwrap();
        assert!(!gate.current(&a));
        assert!(gate.current(&b));
        assert!(gate.current(&next));
    }
    #[test]
    fn escape_and_lock_revoke_work_even_without_driver() {
        let gate = Admission::default();
        let first = gate.begin("task").unwrap();
        assert!(gate.stop(None));
        assert!(!gate.current(&first));
        let second = gate.begin("task").unwrap();
        gate.pause("screen-lock");
        assert!(gate.begin("task").is_err());
        gate.pause("system-sleep");
        gate.resume("screen-lock");
        assert!(gate.begin("task").is_err());
        gate.resume("system-sleep");
        assert!(!gate.current(&second));
        assert!(gate.begin("task").is_ok());
    }
}
