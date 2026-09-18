//! Transcript text the user selected and added to the chat, staged on the
//! composer and folded into the next prompt as a quoted block.
//!
//! [`with_quotes`] appends them; [`extract_badge`] reads the same block back
//! out for the transcript. There is no second data model.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quote {
    pub id: String,
    pub text: String,
}

impl Quote {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            text: text.into(),
        }
    }
}

pub const QUOTE_ONLY_TEXT: &str = "Refer to the selected text below.";

pub const QUOTE_BLOCK_HEADER: &str = "Selected text from this conversation (quoted):";

const DETAIL_LOCATION: &str = "Selected text";

fn quote_block(text: &str) -> String {
    text.trim()
        .lines()
        .map(|line| {
            if line.is_empty() {
                ">".to_string()
            } else {
                format!("> {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn with_quotes(text: &str, quotes: &[Quote]) -> String {
    if quotes.is_empty() {
        return text.to_string();
    }
    let blocks: Vec<String> = quotes.iter().map(|q| quote_block(&q.text)).collect();
    let body = if text.is_empty() {
        QUOTE_ONLY_TEXT
    } else {
        text
    };
    format!("{body}\n\n{QUOTE_BLOCK_HEADER}\n{}", blocks.join("\n\n"))
}

pub fn extract_badge(text: &str) -> Option<(String, crate::badges::MessageBadge)> {
    let marker = format!("\n\n{QUOTE_BLOCK_HEADER}\n");
    let at = text.rfind(&marker)?;
    let block = &text[at + marker.len()..];
    if block.is_empty()
        || !block
            .lines()
            .all(|line| line.is_empty() || line.starts_with('>'))
    {
        return None;
    }
    let details: Vec<crate::badges::BadgeDetail> = block
        .split("\n\n")
        .filter(|chunk| !chunk.is_empty())
        .map(|chunk| {
            let body = chunk
                .lines()
                .map(|line| {
                    line.strip_prefix("> ")
                        .or_else(|| line.strip_prefix('>'))
                        .unwrap_or(line)
                })
                .collect::<Vec<_>>()
                .join("\n");
            crate::badges::BadgeDetail {
                location: DETAIL_LOCATION.into(),
                tag: None,
                body: body.into(),
            }
        })
        .collect();
    if details.is_empty() {
        return None;
    }
    Some((
        text[..at].to_string(),
        crate::badges::MessageBadge {
            icon: crate::icons::WRAP_TEXT,
            label: chip_label(details.len()).into(),
            details,
        },
    ))
}

pub fn chip_label(count: usize) -> String {
    if count == 1 {
        "1 selection".to_string()
    } else {
        format!("{count} selections")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_set_leaves_the_prompt_untouched() {
        assert_eq!(with_quotes("hello", &[]), "hello");
    }

    #[test]
    fn quotes_append_as_quoted_blocks() {
        let quotes = vec![Quote::new("first line\nsecond line"), Quote::new("other")];
        assert_eq!(
            with_quotes("explain this", &quotes),
            format!("explain this\n\n{QUOTE_BLOCK_HEADER}\n> first line\n> second line\n\n> other")
        );
    }

    #[test]
    fn quote_only_send_gets_a_body() {
        let out = with_quotes("", &[Quote::new("x")]);
        assert!(out.starts_with(QUOTE_ONLY_TEXT));
    }

    #[test]
    fn blank_lines_inside_a_quote_stay_inside_it() {
        let quotes = vec![Quote::new("a\n\nb")];
        let sent = with_quotes("q", &quotes);
        assert!(sent.ends_with("> a\n>\n> b"));
        let (rest, badge) = extract_badge(&sent).expect("badge");
        assert_eq!(rest, "q");
        assert_eq!(badge.details.len(), 1);
        assert_eq!(badge.details[0].body.as_ref(), "a\n\nb");
    }

    #[test]
    fn the_block_round_trips_through_the_badge() {
        let quotes = vec![Quote::new("first line\nsecond line"), Quote::new("other")];
        let sent = with_quotes("explain this", &quotes);
        let (rest, badge) = extract_badge(&sent).expect("badge");
        assert_eq!(rest, "explain this");
        assert_eq!(badge.label.as_ref(), "2 selections");
        assert_eq!(badge.details[0].body.as_ref(), "first line\nsecond line");
        assert_eq!(badge.details[1].body.as_ref(), "other");
    }

    #[test]
    fn a_prompt_without_the_block_is_left_alone() {
        assert!(extract_badge("plain prompt\n\n> a quote the user typed").is_none());
    }

    #[test]
    fn quotes_and_comments_share_one_prompt() {
        use crate::comments::{CommentSide, ReviewComment};
        let quoted = with_quotes("do it", &[Quote::new("sel")]);
        let sent = crate::comments::with_comments(
            &quoted,
            &[ReviewComment::new(
                "src/main.rs",
                CommentSide::New,
                4,
                "fix",
            )],
        );
        let (rest, badges) = crate::badges::split(&sent);
        assert_eq!(rest, "do it");
        assert_eq!(badges.len(), 2);
        assert_eq!(badges[0].label.as_ref(), "1 comment");
        assert_eq!(badges[1].label.as_ref(), "1 selection");
    }

    #[test]
    fn chip_label_pluralizes() {
        assert_eq!(chip_label(1), "1 selection");
        assert_eq!(chip_label(3), "3 selections");
    }
}
