use crate::badges::{COMMENT_BLOCK_HEADER, REVIEW_COMMENT_BLOCK_HEADER, side_marker};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommentSide {
    Old,
    New,
}

impl CommentSide {
    pub fn tag(self) -> &'static str {
        match self {
            Self::Old => "L",
            Self::New => "R",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommentSource {
    Diff {
        side: CommentSide,
        old_path: Option<String>,
    },
    File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewComment {
    pub id: String,
    pub path: String,
    pub line: u32,
    pub body: String,
    pub source: CommentSource,
}

impl ReviewComment {
    pub fn new(
        path: impl Into<String>,
        side: CommentSide,
        line: u32,
        body: impl Into<String>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            path: path.into(),
            line,
            body: body.into(),
            source: CommentSource::Diff {
                side,
                old_path: None,
            },
        }
    }

    pub fn file(path: impl Into<String>, line: u32, body: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            path: path.into(),
            line,
            body: body.into(),
            source: CommentSource::File,
        }
    }

    pub fn renamed_from(mut self, old_path: Option<impl Into<String>>) -> Self {
        if let CommentSource::Diff {
            old_path: current, ..
        } = &mut self.source
        {
            *current = old_path.map(Into::into);
        }
        self
    }

    pub fn diff_anchor(&self) -> Option<(CommentSide, u32)> {
        match self.source {
            CommentSource::Diff { side, .. } => Some((side, self.line)),
            CommentSource::File => None,
        }
    }

    pub fn is_file(&self) -> bool {
        matches!(self.source, CommentSource::File)
    }

    pub fn side(&self) -> Option<CommentSide> {
        match self.source {
            CommentSource::Diff { side, .. } => Some(side),
            CommentSource::File => None,
        }
    }

    /// An `Old`-side line only exists in the pre-rename file, so citing `path` there names the wrong file.
    pub fn cite_path(&self) -> &str {
        match &self.source {
            CommentSource::Diff {
                side: CommentSide::Old,
                old_path,
            } => old_path.as_deref().unwrap_or(&self.path),
            CommentSource::Diff { .. } | CommentSource::File => &self.path,
        }
    }

    pub fn location(&self) -> String {
        format!("{}:{}", self.cite_path(), self.line)
    }
}

pub const COMMENT_ONLY_TEXT: &str = "Address the review comments below.";

pub fn with_comments(text: &str, comments: &[ReviewComment]) -> String {
    if comments.is_empty() {
        return text.to_string();
    }
    let bullets: Vec<String> = comments
        .iter()
        .map(|comment| {
            let body = comment.body.trim().replace('\n', "\n  ");
            let separator = comment
                .side()
                .map(|side| side_marker(side.tag()))
                .unwrap_or_else(|| ": ".to_string());
            format!("- {}{separator}{body}", comment.location())
        })
        .collect();
    let body = if text.is_empty() {
        COMMENT_ONLY_TEXT
    } else {
        text
    };
    let header = if comments.iter().all(|comment| !comment.is_file()) {
        COMMENT_BLOCK_HEADER
    } else {
        REVIEW_COMMENT_BLOCK_HEADER
    };
    format!("{body}\n\n{header}\n{}", bullets.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::badges::{comment_label as chip_label, extract_comment_badge as extract_badge};

    fn comment(path: &str, side: CommentSide, line: u32, body: &str) -> ReviewComment {
        ReviewComment::new(path, side, line, body)
    }

    #[test]
    fn empty_set_leaves_the_prompt_untouched() {
        assert_eq!(with_comments("ship it", &[]), "ship it");
    }

    #[test]
    fn comments_append_as_located_bullets() {
        let staged = vec![
            comment("src/main.rs", CommentSide::New, 42, "early-return here"),
            comment("src/lib.rs", CommentSide::Old, 7, "why was this dropped?"),
        ];
        let out = with_comments("look at these", &staged);
        assert!(out.starts_with("look at these\n\n"));
        assert!(out.contains("- src/main.rs:42 (R): early-return here"));
        assert!(out.contains("- src/lib.rs:7 (L): why was this dropped?"));
    }

    #[test]
    fn comment_only_send_gets_a_body() {
        let staged = vec![comment("a.rs", CommentSide::New, 1, "fix")];
        assert!(with_comments("", &staged).starts_with(COMMENT_ONLY_TEXT));
    }

    #[test]
    fn multiline_bodies_indent_under_their_bullet() {
        let staged = vec![comment("a.rs", CommentSide::New, 3, "first\nsecond")];
        assert!(with_comments("x", &staged).contains("- a.rs:3 (R): first\n  second"));
    }

    #[test]
    fn file_comments_use_current_workspace_locations_without_diff_tags() {
        let staged = vec![ReviewComment::file(
            "crates/ui/src/files/preview.rs",
            1494,
            "Keep this branch explicit",
        )];
        let out = with_comments("review this", &staged);
        assert!(out.contains(REVIEW_COMMENT_BLOCK_HEADER));
        assert!(out.contains("- crates/ui/src/files/preview.rs:1494: Keep this branch explicit"));
        assert!(!out.contains("(R)"));

        let (text, badge) = extract_badge(&out).unwrap();
        assert_eq!(text, "review this");
        assert_eq!(
            badge.details[0].location.as_str(),
            "crates/ui/src/files/preview.rs:1494"
        );
        assert_eq!(badge.details[0].tag, None);
        assert_eq!(badge.details[0].body.as_str(), "Keep this branch explicit");
    }

    #[test]
    fn mixed_editor_and_diff_comments_share_one_review_block() {
        let staged = vec![
            comment("a.rs", CommentSide::Old, 3, "why removed?"),
            ReviewComment::file("odd:path.rs", 8, "change this: please"),
        ];
        let (text, badge) = extract_badge(&with_comments("x", &staged)).unwrap();
        assert_eq!(text, "x");
        assert_eq!(badge.details.len(), 2);
        assert_eq!(badge.details[0].tag.as_deref(), Some("L"));
        assert_eq!(badge.details[1].location.as_str(), "odd:path.rs:8");
        assert_eq!(badge.details[1].body.as_str(), "change this: please");
    }

    #[test]
    fn file_comment_body_may_quote_a_diff_side_marker() {
        let staged = vec![ReviewComment::file("a.rs", 2, "compare with (L): old code")];
        let (_, badge) = extract_badge(&with_comments("x", &staged)).unwrap();
        assert_eq!(badge.details[0].location.as_str(), "a.rs:2");
        assert_eq!(badge.details[0].tag, None);
        assert_eq!(badge.details[0].body.as_str(), "compare with (L): old code");
    }

    #[test]
    fn chip_label_pluralizes() {
        assert_eq!(chip_label(1), "1 comment");
        assert_eq!(chip_label(2), "2 comments");
        assert_eq!(chip_label(0), "0 comments");
    }

    #[test]
    fn a_body_quoting_a_side_marker_survives_the_round_trip() {
        let staged = vec![comment(
            "a.rs",
            CommentSide::New,
            5,
            "see (L): the other one",
        )];
        let (text, badge) = extract_badge(&with_comments("x", &staged)).unwrap();
        assert_eq!(text, "x");
        assert_eq!(badge.details.len(), 1);
        assert_eq!(badge.details[0].location.as_str(), "a.rs:5");
        assert_eq!(badge.details[0].tag.as_deref(), Some("R"));
        assert_eq!(badge.details[0].body.as_str(), "see (L): the other one");
    }

    #[test]
    fn a_renamed_file_cites_the_side_the_line_lives_in() {
        let old = comment("new_name.rs", CommentSide::Old, 7, "why dropped?")
            .renamed_from(Some("old_name.rs"));
        let new =
            comment("new_name.rs", CommentSide::New, 12, "nit").renamed_from(Some("old_name.rs"));
        assert_eq!(old.path, "new_name.rs");
        assert_eq!(new.path, "new_name.rs");
        let out = with_comments("x", &[old, new]);
        assert!(out.contains("- old_name.rs:7 (L): why dropped?"));
        assert!(out.contains("- new_name.rs:12 (R): nit"));
    }

    #[test]
    fn an_unrenamed_file_cites_its_only_path_on_both_sides() {
        let staged = vec![
            comment("a.rs", CommentSide::Old, 3, "gone"),
            comment("a.rs", CommentSide::New, 4, "here"),
        ];
        let out = with_comments("x", &staged);
        assert!(out.contains("- a.rs:3 (L): gone"));
        assert!(out.contains("- a.rs:4 (R): here"));
    }
}
