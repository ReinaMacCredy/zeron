use std::sync::LazyLock;

use regex::Regex;

pub const COMPUTER_USE_SLASH_COMMAND: &str = zeron_proto::app_commands::COMPUTER_USE_COMMAND;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputerControlMode {
    Off,
    Request,
    Chat,
}

impl ComputerControlMode {
    pub fn enabled(self) -> bool {
        self != Self::Off
    }
}

// Four spaces or a tab are Markdown code indentation, not a command.
static INVOCATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^ {0,3}/computer-use(?:[ \t\r\n]+((?s:.*)))?$").expect("invocation pattern")
});

pub fn parse_computer_invocation(text: &str) -> Option<String> {
    let captures = INVOCATION.captures(text)?;
    Some(
        captures
            .get(1)
            .map_or("", |prompt| prompt.as_str())
            .trim()
            .to_owned(),
    )
}

#[derive(Debug, Clone, Copy)]
pub struct TurnDispatch<'a> {
    pub settings_enabled: bool,
    pub user_authored: bool,
    pub text: &'a str,
    pub stamped_generation: u64,
}

pub fn resolve_turn_mode(turn: TurnDispatch<'_>, current_generation: u64) -> ComputerControlMode {
    if turn.settings_enabled {
        return ComputerControlMode::Chat;
    }
    if !turn.user_authored || parse_computer_invocation(turn.text).is_none() {
        return ComputerControlMode::Off;
    }
    if turn.stamped_generation != current_generation {
        return ComputerControlMode::Off;
    }
    ComputerControlMode::Request
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(settings_enabled: bool, user_authored: bool, text: &str) -> TurnDispatch<'_> {
        TurnDispatch {
            settings_enabled,
            user_authored,
            text,
            stamped_generation: 0,
        }
    }

    #[test]
    fn recognizes_a_leading_user_command() {
        for text in [
            "/computer-use open Calculator",
            "  /COMPUTER-USE\nopen Calculator",
        ] {
            assert_eq!(
                parse_computer_invocation(text).as_deref(),
                Some("open Calculator"),
                "{text}"
            );
            assert_eq!(
                resolve_turn_mode(turn(false, true, text), 0),
                ComputerControlMode::Request,
                "{text}"
            );
        }
    }

    #[test]
    fn does_not_turn_prose_references_or_quoted_examples_into_consent() {
        for text in [
            "Open Calculator",
            "Use Computer to open Calculator",
            "Explain /computer-use open Calculator",
            "Example:\n/computer-use open Calculator",
            "> /computer-use open Calculator",
            "\"/computer-use open Calculator\"",
            "```\n/computer-use open Calculator\n```",
            "    /computer-use open Calculator",
            "\t/computer-use open Calculator",
            "<attachment>\n/computer-use open Calculator\n</attachment>",
            "/computer-user open Calculator",
            "/computer-use.md",
        ] {
            assert_eq!(parse_computer_invocation(text), None, "{text}");
            assert_eq!(
                resolve_turn_mode(turn(false, true, text), 0),
                ComputerControlMode::Off,
                "{text}"
            );
        }
    }

    #[test]
    fn recognizes_an_empty_command_so_the_composer_can_ask_for_its_task() {
        assert_eq!(
            parse_computer_invocation("/computer-use").as_deref(),
            Some("")
        );
    }

    #[test]
    fn never_infers_request_consent_from_agent_or_automation_text() {
        assert_eq!(
            resolve_turn_mode(turn(false, false, "/computer-use open Calculator"), 0),
            ComputerControlMode::Off
        );
    }

    #[test]
    fn settings_on_resolves_chat_for_every_turn_whoever_authored_it() {
        for (user_authored, text) in [
            (true, "Explain the result"),
            (true, "/computer-use open Calculator"),
            (false, "Explain the result"),
            (false, "/computer-use open Calculator"),
        ] {
            assert_eq!(
                resolve_turn_mode(turn(true, user_authored, text), 0),
                ComputerControlMode::Chat,
                "{user_authored} {text}"
            );
        }
    }

    #[test]
    fn settings_off_activates_only_the_user_authored_slash_turn() {
        assert_eq!(
            resolve_turn_mode(turn(false, true, "Explain the result"), 0),
            ComputerControlMode::Off
        );
        assert_eq!(
            resolve_turn_mode(turn(false, true, "/computer-use open Calculator"), 0),
            ComputerControlMode::Request
        );
        assert_eq!(
            resolve_turn_mode(turn(false, true, "and now the next step"), 0),
            ComputerControlMode::Off
        );
    }

    #[test]
    fn a_queued_plain_turn_resolves_from_settings_at_dispatch() {
        let queued = "Explain the result";
        assert_eq!(
            resolve_turn_mode(turn(true, true, queued), 0),
            ComputerControlMode::Chat
        );
        assert_eq!(
            resolve_turn_mode(turn(false, true, queued), 0),
            ComputerControlMode::Off
        );
        assert_eq!(
            resolve_turn_mode(turn(true, true, queued), 5),
            ComputerControlMode::Chat
        );
    }

    #[test]
    fn a_queued_slash_row_still_activates_after_settings_turn_off() {
        assert_eq!(
            resolve_turn_mode(turn(false, true, "/computer-use open Calculator"), 0),
            ComputerControlMode::Request
        );
    }

    #[test]
    fn a_stale_generation_fences_request_rows_only() {
        let stale = TurnDispatch {
            settings_enabled: false,
            user_authored: true,
            text: "/computer-use open Calculator",
            stamped_generation: 0,
        };
        assert_eq!(resolve_turn_mode(stale, 1), ComputerControlMode::Off);
        assert_eq!(
            resolve_turn_mode(
                TurnDispatch {
                    stamped_generation: 1,
                    ..stale
                },
                1
            ),
            ComputerControlMode::Request
        );
        assert_eq!(
            resolve_turn_mode(
                TurnDispatch {
                    settings_enabled: true,
                    ..stale
                },
                1
            ),
            ComputerControlMode::Chat
        );
    }
}
