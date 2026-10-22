pub use zeron_proto::quotes::*;

pub fn extract_badge(text: &str) -> Option<(String, crate::badges::MessageBadge)> {
    badge(extract(text))
}

pub fn extract_terminal_badge(text: &str) -> Option<(String, crate::badges::MessageBadge)> {
    badge(extract_terminal(text))
}

fn badge(
    extracted: Option<(String, QuotedBlock)>,
) -> Option<(String, crate::badges::MessageBadge)> {
    let (rest, block) = extracted?;
    let details = block
        .bodies
        .into_iter()
        .map(|body| crate::badges::BadgeDetail {
            location: block.location.into(),
            tag: None,
            body: body.into(),
        })
        .collect();
    Some((
        rest,
        crate::badges::MessageBadge {
            icon: crate::icons::WRAP_TEXT,
            label: block.label.into(),
            details,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_selections_get_their_own_block_and_badge() {
        let quotes = vec![Quote::terminal("$ make\nerror: boom"), Quote::new("why")];
        let sent = with_quotes("fix it", &quotes);
        assert_eq!(
            sent,
            format!(
                "fix it\n\n{QUOTE_BLOCK_HEADER}\n> why\n\n{TERMINAL_BLOCK_HEADER}\n> $ make\n> error: boom"
            )
        );
        let (rest, badges) = crate::badges::split(&sent);
        assert_eq!(rest, "fix it");
        assert_eq!(badges.len(), 2);
        assert_eq!(badges[0].details[0].location.as_ref(), TERMINAL_LOCATION);
        assert_eq!(badges[0].details[0].body.as_ref(), "$ make\nerror: boom");
        assert_eq!(badges[1].details[0].body.as_ref(), "why");
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
}
