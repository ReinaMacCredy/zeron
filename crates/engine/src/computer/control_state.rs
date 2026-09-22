use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

#[derive(Debug, Default)]
pub struct ComputerControlState {
    generations: Mutex<HashMap<String, u64>>,
}

impl ComputerControlState {
    pub fn with_generations(generations: impl IntoIterator<Item = (String, u64)>) -> Self {
        Self {
            generations: Mutex::new(generations.into_iter().collect()),
        }
    }

    pub fn generation(&self, chat_id: &str) -> u64 {
        self.generations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(chat_id)
            .copied()
            .unwrap_or(0)
    }

    pub fn allows(&self, chat_id: &str, generation: u64) -> bool {
        self.generation(chat_id) == generation
    }

    pub fn bump(&self, chat_id: &str) -> u64 {
        let mut generations = self
            .generations
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let generation = generations.entry(chat_id.to_owned()).or_insert(0);
        *generation += 1;
        *generation
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::computer::activation::{ComputerControlMode, TurnDispatch, resolve_turn_mode};

    #[test]
    fn rejects_queued_generations_after_a_stop_and_across_a_restart() {
        let state = ComputerControlState::default();
        assert!(state.allows("thread", 0));
        assert_eq!(state.bump("thread"), 1);
        assert!(!state.allows("thread", 0));
        assert!(state.allows("thread", 1));
        let restored = ComputerControlState::with_generations([(
            "thread".to_owned(),
            state.generation("thread"),
        )]);
        assert!(!restored.allows("thread", 0));
        assert!(restored.allows("thread", 1));
        assert!(!state.allows("other", 1));
        assert!(state.allows("other", 0));
    }

    #[test]
    fn concurrent_bumps_never_lose_an_increment() {
        let state = Arc::new(ComputerControlState::default());
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let state = state.clone();
                std::thread::spawn(move || {
                    for _ in 0..100 {
                        state.bump("thread");
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(state.generation("thread"), 800);
    }

    #[test]
    fn a_slash_turn_queued_before_stop_runs_without_computer_after_it() {
        let state = ComputerControlState::default();
        let queued = TurnDispatch {
            settings_enabled: false,
            user_authored: true,
            text: "/computer-use open Calculator",
            stamped_generation: state.generation("thread"),
        };
        state.bump("thread");
        assert_eq!(
            resolve_turn_mode(queued, state.generation("thread")),
            ComputerControlMode::Off
        );
        let unrelated = TurnDispatch {
            text: "thanks, what next?",
            stamped_generation: state.generation("thread"),
            ..queued
        };
        assert_eq!(
            resolve_turn_mode(unrelated, state.generation("thread")),
            ComputerControlMode::Off
        );
        assert_eq!(
            resolve_turn_mode(queued, state.generation("thread")),
            ComputerControlMode::Off
        );
        let fresh = TurnDispatch {
            stamped_generation: state.generation("thread"),
            ..queued
        };
        assert_eq!(
            resolve_turn_mode(fresh, state.generation("thread")),
            ComputerControlMode::Request
        );
    }

    #[test]
    fn a_physical_escape_stop_fences_the_same_way() {
        let state = ComputerControlState::default();
        let queued = TurnDispatch {
            settings_enabled: false,
            user_authored: true,
            text: "/computer-use open Calculator",
            stamped_generation: state.generation("thread"),
        };
        let escape_stop = |chat: &str| state.bump(chat);
        escape_stop("thread");
        assert_eq!(
            resolve_turn_mode(queued, state.generation("thread")),
            ComputerControlMode::Off
        );
    }

    #[test]
    fn with_settings_on_a_turn_queued_before_stop_keeps_the_catalog() {
        let state = ComputerControlState::default();
        let queued = TurnDispatch {
            settings_enabled: true,
            user_authored: true,
            text: "keep testing the app",
            stamped_generation: state.generation("thread"),
        };
        state.bump("thread");
        assert_eq!(
            resolve_turn_mode(queued, state.generation("thread")),
            ComputerControlMode::Chat
        );
    }
}
