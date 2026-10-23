//! Review comments pinned to lines in a diff or editable workspace file,
//! staged on the composer and folded into the next prompt as plain text.
//!
//! [`with_comments`] appends them; `zeron_proto::badges` reads the same block
//! back out for the transcript. There is no second data model.

pub use zeron_proto::comments::{
    COMMENT_ONLY_TEXT, CommentSide, CommentSource, ReviewComment, with_comments,
};

pub const CARD_PAD_V: f32 = 20.0;
pub const CARD_HEADER_HEIGHT: f32 = 22.0;
pub const CARD_LINE_HEIGHT: f32 = 18.0;
pub const DRAFT_CARD_HEIGHT: f32 = 116.0;
pub const COMMENT_ADDER_SIZE: f32 = 16.0;
const CARD_GAP: f32 = 6.0;
/// 12px Geist averages about 5.65px a character; the rest covers words that wrap early.
const CARD_CHAR_WIDTH: f32 = 6.4;
pub const CARD_BODY_MAX_WIDTH: f32 = 480.0;
const CARD_MAX_LINES: usize = 8;
pub const CARD_BODY_MAX_HEIGHT: f32 = CARD_MAX_LINES as f32 * CARD_LINE_HEIGHT;

/// Wraps are guessed, not measured: the fold tween and the editor overlay need a
/// height before layout. Cards laid out in a list size themselves.
pub fn card_body_lines(body: &str, width: f32) -> usize {
    let columns = ((width.min(CARD_BODY_MAX_WIDTH) / CARD_CHAR_WIDTH) as usize).max(1);
    body.lines()
        .map(|line| line.chars().count().div_ceil(columns).max(1))
        .sum::<usize>()
        .clamp(1, CARD_MAX_LINES)
}

pub fn card_height_at(body: &str, width: f32) -> f32 {
    CARD_PAD_V
        + CARD_HEADER_HEIGHT
        + card_body_lines(body, width) as f32 * CARD_LINE_HEIGHT
        + CARD_GAP
}

pub fn card_height(body: &str) -> f32 {
    card_height_at(body, CARD_BODY_MAX_WIDTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_height_grows_with_body_lines() {
        assert!(card_height("two\nlines") > card_height("one"));
        assert_eq!(card_height(""), card_height("one"));
    }

    #[test]
    fn long_lines_are_charged_for_their_soft_wraps() {
        let columns = (CARD_BODY_MAX_WIDTH / CARD_CHAR_WIDTH) as usize;
        assert_eq!(card_body_lines("short", CARD_BODY_MAX_WIDTH), 1);
        assert_eq!(
            card_body_lines(&"x".repeat(columns), CARD_BODY_MAX_WIDTH),
            1
        );
        assert_eq!(
            card_body_lines(&"x".repeat(columns + 1), CARD_BODY_MAX_WIDTH),
            2
        );
        assert_eq!(card_body_lines(&"x".repeat(70), 288.0), 2);
        assert_eq!(
            card_body_lines(&"line\n".repeat(200), CARD_BODY_MAX_WIDTH),
            CARD_MAX_LINES
        );
    }
}
