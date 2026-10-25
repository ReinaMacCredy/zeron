use crate::{MessagePart, MessageRole, SessionMessageEntry};

pub fn transcript_markdown<'a>(
    title: &str,
    entries: impl IntoIterator<Item = &'a SessionMessageEntry>,
) -> String {
    let mut out = format!("# {title}\n");
    for entry in entries {
        let text: Vec<&str> = entry
            .parts
            .iter()
            .filter_map(|part| match part {
                MessagePart::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        let Some(first) = text.first() else {
            continue;
        };
        match entry.role {
            MessageRole::User => out.push_str(&format!("\n## You\n\n{}\n", text.concat())),
            MessageRole::Assistant => {
                out.push_str(&format!("\n## Assistant\n\n{}\n", text.concat()))
            }
            MessageRole::System => out.push_str(&format!("\n_{first}_\n")),
        }
    }
    out
}

pub fn export_file_name(title: &str) -> String {
    let name: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{}.md", name.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_keeps_turns_and_notes_in_order() {
        let entry = |role, text: &str| SessionMessageEntry {
            id: text.into(),
            role,
            parts: vec![MessagePart::Text {
                id: "t0".into(),
                text: text.into(),
            }],
            created_at: 0,
            device_id: "d".into(),
            status: None,
            continuation_of: None,
            from_chat_id: None,
            fork_import: false,
            answers_request_id: None,
            duration_ms: None,
        };
        let md = transcript_markdown(
            "Lab",
            &[
                entry(MessageRole::User, "hi"),
                entry(MessageRole::Assistant, "hello"),
                entry(MessageRole::System, "Claude Code /usage"),
            ],
        );
        assert_eq!(
            md,
            "# Lab\n\n## You\n\nhi\n\n## Assistant\n\nhello\n\n_Claude Code /usage_\n"
        );
    }
}
