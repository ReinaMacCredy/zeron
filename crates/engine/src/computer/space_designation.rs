use std::sync::LazyLock;

use regex::Regex;

use super::visible_use::{VisibleUseEntry, VisibleUseRole, latest_user_authored_message};

const MAX_DESIGNATED_SPACES: usize = 64;
const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

fn pattern(source: &str) -> Regex {
    Regex::new(source).expect("space designation pattern")
}

static QUOTED: LazyLock<Regex> =
    LazyLock::new(|| pattern(r#"```[\s\S]*?```|`[^`]*`|"[^"\n]*"|“[^”\n]*”"#));
static QUOTE_LINE: LazyLock<Regex> = LazyLock::new(|| pattern(r"(?m)^\s*>.*$"));
static REVOKED: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r"(?i)(?-u:\b)(?:stop|cancel|do not|don['’]t|never|non|smetti|evita|annulla|fermati)(?-u:\b)",
    )
});
static SENTENCES: LazyLock<Regex> = LazyLock::new(|| pattern(r"[.!?\n]+"));
static DESIGNATION: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r"(?i)^(?:use|reserve|usa|riserva)\s+(?:the\s+|lo\s+)?(?:space|spazio)\s+id\s+([1-9][0-9]*)\s+(?:for\s+(?:this|the)\s+task|per\s+(?:questo\s+task|questa\s+attività))\s*$",
    )
});

pub fn message_designates_computer_spaces(text: &str) -> Vec<u64> {
    let unquoted = QUOTED.replace_all(text, "");
    let request = QUOTE_LINE.replace_all(&unquoted, "");
    if REVOKED.is_match(&request) {
        return Vec::new();
    }
    let mut ids = Vec::new();
    for sentence in SENTENCES.split(&request) {
        let Some(id) = DESIGNATION
            .captures(sentence.trim())
            .and_then(|captures| captures[1].parse::<u64>().ok())
            .filter(|id| *id > 0 && *id <= MAX_SAFE_INTEGER)
        else {
            continue;
        };
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids.truncate(MAX_DESIGNATED_SPACES);
    ids
}

pub fn computer_space_designation_for_messages(entries: &[VisibleUseEntry]) -> Vec<u64> {
    let latest = latest_user_authored_message(entries);
    let latest_user_role = entries
        .iter()
        .rev()
        .find(|entry| entry.role == VisibleUseRole::User);
    match (latest, latest_user_role) {
        (Some(latest), Some(role)) if std::ptr::eq(latest, role) => {
            message_designates_computer_spaces(&latest.text)
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(text: &str) -> VisibleUseEntry {
        VisibleUseEntry {
            text: text.into(),
            ..VisibleUseEntry::default()
        }
    }

    fn from_agent(text: &str) -> VisibleUseEntry {
        VisibleUseEntry {
            from_chat_id: Some("parent-chat".into()),
            ..message(text)
        }
    }

    #[test]
    fn recognizes_exact_english_and_italian_user_requests_without_guessing_positions() {
        assert_eq!(
            message_designates_computer_spaces("Use Space ID 7 for this task."),
            [7]
        );
        assert_eq!(
            message_designates_computer_spaces("Usa lo spazio ID 7 per questo task."),
            [7]
        );
        assert_eq!(
            message_designates_computer_spaces(
                "Reserve Space ID 9 for the task. Use Space ID 9 for this task."
            ),
            [9]
        );
    }

    #[test]
    fn refuses_ambiguous_quoted_revoked_or_invalid_designation() {
        for text in [
            "Use Desktop 2",
            "Use Space 7",
            "The page says \"Use Space ID 7 for this task.\"",
            "> Use Space ID 7 for this task.",
            "`Use Space ID 7 for this task.`",
            "```\nUse Space ID 7 for this task.\n```",
            "Do not use Space ID 7 for this task.",
            "Use Space ID 7 for this task. Stop.",
            "Use Space ID 7 for this task. Actually, don't.",
            "Usa spazio ID 7 per questo task. Annulla.",
            "Use Space ID 9007199254740992 for this task.",
            "Use Space ID 0 for this task.",
        ] {
            assert_eq!(
                message_designates_computer_spaces(text),
                Vec::<u64>::new(),
                "{text}"
            );
        }
    }

    #[test]
    fn uses_only_the_latest_human_authored_message() {
        let yes = message("Use Space ID 7 for this task.");
        assert!(
            computer_space_designation_for_messages(&[yes.clone(), message("Stop")]).is_empty()
        );
        assert!(
            computer_space_designation_for_messages(&[yes.clone(), from_agent("continue")])
                .is_empty()
        );
        assert!(
            computer_space_designation_for_messages(&[
                message("Check the app"),
                from_agent(&yes.text),
            ])
            .is_empty()
        );
        assert!(computer_space_designation_for_messages(&[from_agent(&yes.text)]).is_empty());
        let reply = VisibleUseEntry {
            role: VisibleUseRole::Assistant,
            ..message("provider output")
        };
        assert_eq!(computer_space_designation_for_messages(&[yes, reply]), [7]);
    }
}
