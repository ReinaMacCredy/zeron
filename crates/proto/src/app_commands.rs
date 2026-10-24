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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClaudeCardCommand {
    Context,
    Usage,
    Model(Option<String>),
    Mcp,
    TerminalOnly(String),
}

// Headless `claude -p "/<name>"` answers "isn't available in this environment"
// (or "here yet") for these, and init lists doctor/color/focus/reload-plugins as
// terminal_slash_commands (2.1.310).
pub const TERMINAL_ONLY_COMMANDS: &[&str] = &[
    "add-dir",
    "bashes",
    "bug",
    "color",
    "doctor",
    "export",
    "feedback",
    "focus",
    "help",
    "hooks",
    "ide",
    "install-github-app",
    "login",
    "logout",
    "memory",
    "permissions",
    "plugin",
    "privacy-settings",
    "release-notes",
    "reload-plugins",
    "resume",
    "rewind",
    "status",
    "statusline",
    "tasks",
    "terminal-setup",
    "theme",
    "upgrade",
    "vim",
];

pub fn claude_card_command(
    text: &str,
    is_skill: impl Fn(&str) -> bool,
) -> Option<ClaudeCardCommand> {
    let (name, rest) = crate::invocation::leading_command(text.trim_start())?;
    let rest = rest.trim();
    if is_skill(name) {
        return None;
    }
    match name {
        "context" => Some(ClaudeCardCommand::Context),
        "usage" | "cost" => Some(ClaudeCardCommand::Usage),
        "model" => Some(ClaudeCardCommand::Model(
            (!rest.is_empty()).then(|| rest.to_string()),
        )),
        "mcp" if rest.is_empty() => Some(ClaudeCardCommand::Mcp),
        name if TERMINAL_ONLY_COMMANDS.contains(&name) => {
            Some(ClaudeCardCommand::TerminalOnly(name.to_string()))
        }
        _ => None,
    }
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
    fn card_commands_are_recognized_and_skills_keep_their_turn() {
        let none = |_: &str| false;
        assert_eq!(
            claude_card_command("/context", none),
            Some(ClaudeCardCommand::Context)
        );
        assert_eq!(
            claude_card_command("/cost", none),
            Some(ClaudeCardCommand::Usage)
        );
        assert_eq!(
            claude_card_command("/model opus", none),
            Some(ClaudeCardCommand::Model(Some("opus".into())))
        );
        assert_eq!(
            claude_card_command("/mcp", none),
            Some(ClaudeCardCommand::Mcp)
        );
        assert_eq!(claude_card_command("/mcp reconnect x", none), None);
        assert_eq!(
            claude_card_command("/status", none),
            Some(ClaudeCardCommand::TerminalOnly("status".into()))
        );
        assert_eq!(
            claude_card_command("/memory", |name| name == "memory"),
            None
        );
        assert_eq!(claude_card_command("/review this", none), None);
        assert_eq!(claude_card_command("explain /context", none), None);
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
