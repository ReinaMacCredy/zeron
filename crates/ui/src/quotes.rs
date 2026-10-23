pub use zeron_proto::quotes::*;

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(badges[0].label.as_str(), "1 comment");
        assert_eq!(badges[1].label.as_str(), "1 selection");
    }
}
