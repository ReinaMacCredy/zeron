/// Zeron's own slash command — the composer runs it, the prompt never
/// reaches the agent.
pub const COMPACT_COMMAND: &str = "compact";

/// Zeron's second own command — it opens a side conversation instead of
/// running a turn on this chat.
pub const SIDECHAT_COMMAND: &str = "sidechat";
pub const BTW_COMMAND: &str = "btw";

pub const COMPUTER_USE_COMMAND: &str = "computer-use";

pub const CONFIG_COMMAND: &str = "config";

pub fn opens_config_panel(text: &str) -> bool {
    app_command_argument(text, CONFIG_COMMAND) == Some(None)
}

/// `Some(argument)` when the prompt invokes zeron's own `command`: the first
/// token must be exactly `/command`, and the rest of the line is its argument.
pub fn app_command_argument(text: &str, command: &str) -> Option<Option<String>> {
    let rest = text.strip_prefix('/')?.strip_prefix(command)?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let argument = rest.trim();
    Some((!argument.is_empty()).then(|| argument.to_string()))
}

/// `Some(focus)` when the prompt is a `/compact` invocation: the first token
/// must be exactly `/compact`, and the rest of the line is the focus the
/// harness compacts around.
pub fn compact_focus(text: &str) -> Option<Option<String>> {
    app_command_argument(text, COMPACT_COMMAND)
}

/// `Some(prompt)` when the prompt is a `/btw` or `/sidechat` invocation: bare
/// opens the side conversation, and the rest of the line is its first turn.
pub fn sidechat_prompt(text: &str) -> Option<Option<String>> {
    app_command_argument(text, BTW_COMMAND).or_else(|| app_command_argument(text, SIDECHAT_COMMAND))
}

pub const COMPACT_NEEDS_CHAT: &str = "Start a chat before compacting its context.";
pub const SIDECHAT_NEEDS_CHAT: &str = "Start a chat before opening a sidechat.";

pub fn draft_command_refusal(text: &str) -> Option<&'static str> {
    if compact_focus(text).is_some() {
        Some(COMPACT_NEEDS_CHAT)
    } else if sidechat_prompt(text).is_some() {
        Some(SIDECHAT_NEEDS_CHAT)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_is_intercepted_with_its_focus_and_nothing_else_is() {
        assert_eq!(compact_focus("/compact"), Some(None));
        assert_eq!(
            compact_focus("/compact focus on auth"),
            Some(Some("focus on auth".into()))
        );
        // A longer name that merely starts with it is another command.
        assert!(compact_focus("/compaction").is_none());
        assert!(compact_focus("/compact-now").is_none());
        // Not the first token → an ordinary prompt, sent as a turn.
        assert!(compact_focus("please /compact").is_none());
        assert!(compact_focus("/goal ship it").is_none());
    }

    #[test]
    fn sidechat_is_intercepted_with_its_prompt_and_nothing_else_is() {
        for command in ["sidechat", "btw"] {
            assert_eq!(sidechat_prompt(&format!("/{command}")), Some(None));
            assert_eq!(
                sidechat_prompt(&format!("/{command} compare the two")),
                Some(Some("compare the two".into()))
            );
            // A longer name that merely starts with it is another command.
            assert!(sidechat_prompt(&format!("/{command}s")).is_none());
            assert!(sidechat_prompt(&format!("/{command}-now")).is_none());
            // Not the first token → an ordinary prompt, sent as a turn.
            assert!(sidechat_prompt(&format!("ask in a /{command}")).is_none());
        }
        assert!(sidechat_prompt("/compact").is_none());
    }

    #[test]
    fn only_a_bare_config_opens_the_panel() {
        assert!(opens_config_panel("/config"));
        assert!(opens_config_panel("/config  "));
        assert!(!opens_config_panel("/config theme=dark"));
        assert!(!opens_config_panel("/configure"));
        assert!(!opens_config_panel("open /config"));
    }
}
