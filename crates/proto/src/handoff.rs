use crate::ChatIndicator;

pub const HAND_OFF_THREAD: &str = "Hand off thread";
pub const CONTINUE_IN_THIS_THREAD: &str = "Continue in this thread";
pub const CONTINUE_IN_A_NEW_THREAD: &str = "Continue in a new thread";
pub const HAND_OFF_TO_PROJECT: &str = "Hand off to project";

/// A handoff carries the finished conversation, so a live turn or a chat
/// with nothing finished yet has nothing to hand over.
pub fn handoff_blocked(indicator: ChatIndicator, has_finished_turn: bool) -> bool {
    matches!(
        indicator,
        ChatIndicator::Working | ChatIndicator::AwaitingInput
    ) || !has_finished_turn
}

pub fn handoff_failed_message(err: &str) -> String {
    format!("Couldn't continue in a new thread: {err}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_idle_chat_with_a_finished_turn_hands_off() {
        assert!(!handoff_blocked(ChatIndicator::Idle, true));
        assert!(handoff_blocked(ChatIndicator::Idle, false));
        assert!(handoff_blocked(ChatIndicator::Working, true));
        assert!(handoff_blocked(ChatIndicator::AwaitingInput, true));
    }
}
