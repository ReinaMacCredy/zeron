use crate::HarnessId;

/// A model pick on an existing chat warns before it lands only when there
/// is history for the harness to re-read and the pick is a real change.
pub fn needs_switch_warning(has_history: bool, current: Option<&str>, next: &str) -> bool {
    has_history && current != Some(next)
}

/// The switch-model dialog's body, in the harness's own terms: Claude Code
/// re-reads the whole cached history on the next turn; Codex warns of
/// degraded performance and an automatic compaction.
pub fn switch_warning(harness: Option<HarnessId>, to: &str) -> String {
    match harness {
        Some(HarnessId::ClaudeCode) => format!(
            "This conversation is cached for the current model. Switching to {to} means the full history gets re-read on your next message."
        ),
        Some(HarnessId::Codex) => "Changing models mid-conversation will degrade performance. Context may automatically compact.".into(),
        _ => format!(
            "Changing models mid-conversation may degrade performance. Switching to {to} means the full history gets re-read on your next message."
        ),
    }
}

/// The switch-provider dialog's body: the chat's session belongs to the
/// harness that produced it, so the new one starts fresh with the
/// conversation handed over as text.
pub fn provider_switch_warning(from: &str, to: &str) -> String {
    format!(
        "{to} has never seen this chat. The {from} session is closed; on your next message the conversation so far is handed to {to} as plain text. Tool results, open files and {from}'s cache are not carried over."
    )
}

/// The transcript divider's hover hint after a provider switch.
pub fn provider_switch_hint(from: &str, to: &str) -> String {
    format!(
        "{from} session closed. The messages above were handed to {to} as text on the next turn; {from}'s tool results and file state were not carried."
    )
}

/// The transcript divider's hover hint once the switch has happened.
pub fn switch_hint(harness: Option<HarnessId>) -> &'static str {
    match harness {
        Some(HarnessId::ClaudeCode) => {
            "Changing models mid-conversation drops the prompt cache. The full history is re-read on the next message."
        }
        Some(HarnessId::Codex) => {
            "Changing models mid-conversation will degrade performance. Context may automatically compact."
        }
        _ => {
            "Changing models mid-conversation may degrade performance. The full history is re-read on the next message."
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSwitchPrompt {
    pub title: String,
    pub lead: String,
    pub body: String,
    pub confirm: String,
    pub confirm_detail: String,
    pub cancel: String,
    pub cancel_detail: String,
    pub note: String,
    pub note_hint: String,
}

pub fn model_switch_prompt(harness: Option<HarnessId>, from: &str, to: &str) -> ModelSwitchPrompt {
    ModelSwitchPrompt {
        title: "Switch model?".into(),
        lead: "Your next response will be slower and use more tokens".into(),
        body: switch_warning(harness, to),
        confirm: format!("Yes, switch to {to}"),
        confirm_detail: "Applies to this chat only. The change is marked in the transcript.".into(),
        cancel: "No, go back".into(),
        cancel_detail: format!("Keep {from} for this chat."),
        note: format!("Model changed from {from} to {to}."),
        note_hint: switch_hint(harness).into(),
    }
}

pub fn provider_switch_prompt(
    from: &str,
    to: &str,
    from_model: &str,
    to_model: &str,
) -> ModelSwitchPrompt {
    ModelSwitchPrompt {
        title: "Switch provider?".into(),
        lead: format!("{to_model} starts from a text copy of this chat"),
        body: provider_switch_warning(from, to),
        confirm: format!("Yes, switch to {to_model}"),
        confirm_detail: format!("Same chat, same tab. A divider marks where {from_model} stopped."),
        cancel: format!("No, keep {from_model}"),
        cancel_detail: format!("Keep {from_model} for this chat."),
        note: format!("Provider changed from {from} to {to}."),
        note_hint: provider_switch_hint(from, to),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_real_change_on_a_chat_with_history_asks() {
        assert!(needs_switch_warning(true, Some("a"), "b"));
        assert!(needs_switch_warning(true, None, "b"));
        assert!(!needs_switch_warning(true, Some("b"), "b"));
        assert!(!needs_switch_warning(false, Some("a"), "b"));
        let prompt = model_switch_prompt(Some(HarnessId::ClaudeCode), "Opus", "Sonnet");
        assert_eq!(prompt.confirm, "Yes, switch to Sonnet");
        assert_eq!(prompt.cancel_detail, "Keep Opus for this chat.");
        assert_eq!(prompt.note, "Model changed from Opus to Sonnet.");
        assert!(prompt.body.contains("Switching to Sonnet"));
    }

    #[test]
    fn a_provider_switch_names_both_providers_and_the_new_model() {
        let prompt = provider_switch_prompt("Claude Code", "Codex", "Opus 5", "GPT-5.5");
        assert_eq!(prompt.title, "Switch provider?");
        assert_eq!(prompt.confirm, "Yes, switch to GPT-5.5");
        assert_eq!(prompt.cancel, "No, keep Opus 5");
        assert_eq!(prompt.note, "Provider changed from Claude Code to Codex.");
    }
}
