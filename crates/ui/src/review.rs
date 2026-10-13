use serde::Deserialize;

use crate::changes::{FileDiff, line_anchor};
use crate::comments::{CommentSide, ReviewComment};

pub const REQUEST_PREFIX: &str = "zeron-review:";
const FENCE: &str = "```zeron-review";
const MAX_PATCH_BYTES: usize = 120_000;
const REANCHOR_WINDOW: u32 = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReviewScope {
    Uncommitted,
    Branch,
    LatestTurn,
}

impl ReviewScope {
    pub const ALL: [ReviewScope; 3] = [Self::Branch, Self::Uncommitted, Self::LatestTurn];

    pub fn default_for(worktree: bool) -> Self {
        if worktree {
            Self::Branch
        } else {
            Self::Uncommitted
        }
    }

    pub fn mode(self) -> &'static str {
        match self {
            Self::Uncommitted => "workingTree",
            Self::Branch => "branch",
            Self::LatestTurn => "turn",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Uncommitted => "uncommitted",
            Self::Branch => "branch",
            Self::LatestTurn => "turn",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|scope| scope.key() == key)
    }

    pub fn label(self, base: Option<&str>) -> String {
        match self {
            Self::Uncommitted => "Uncommitted".into(),
            Self::Branch => format!("Branch vs {}", base.unwrap_or("base")),
            Self::LatestTurn => "Latest turn".into(),
        }
    }
}

pub fn request_id(scope: ReviewScope, nonce: &str) -> String {
    format!("{REQUEST_PREFIX}{}:{nonce}", scope.key())
}

pub fn request_scope(request_id: &str) -> Option<ReviewScope> {
    let rest = request_id.strip_prefix(REQUEST_PREFIX)?;
    ReviewScope::from_key(rest.split(':').next()?)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Bug,
    Risk,
    Nit,
}

impl Severity {
    pub fn word(self) -> &'static str {
        match self {
            Self::Bug => "Bug",
            Self::Risk => "Risk",
            Self::Nit => "Nit",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Fresh,
    Outdated,
    Unanchored,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub id: String,
    pub path: String,
    pub side: CommentSide,
    pub line: u32,
    pub text: Option<String>,
    pub severity: Severity,
    pub title: String,
    pub body: String,
    pub placement: Placement,
}

impl Finding {
    pub fn location(&self) -> String {
        format!("{}:{}", self.path, self.line)
    }

    pub fn anchor(&self) -> Option<(CommentSide, u32)> {
        (self.placement != Placement::Unanchored).then_some((self.side, self.line))
    }

    pub fn comment(&self) -> ReviewComment {
        let mut body = format!("{}: {}", self.severity.word(), self.title.trim());
        if !self.body.trim().is_empty() {
            body.push('\n');
            body.push_str(self.body.trim());
        }
        ReviewComment::new(self.path.clone(), self.side, self.line, body)
    }
}

#[derive(Deserialize)]
struct RawFinding {
    path: String,
    #[serde(default)]
    side: Option<String>,
    line: u32,
    #[serde(default)]
    text: Option<String>,
    severity: Severity,
    title: String,
    #[serde(default)]
    body: String,
}

pub fn parse_findings(result: &str) -> Option<Vec<Finding>> {
    let start = result.rfind(FENCE)? + FENCE.len();
    let rest = result[start..].split_once('\n')?.1;
    let end = rest.find("```")?;
    let raw: Vec<RawFinding> = serde_json::from_str(rest[..end].trim()).ok()?;
    Some(
        raw.into_iter()
            .enumerate()
            .map(|(ix, raw)| Finding {
                id: format!("finding-{ix}"),
                path: raw.path.trim().trim_start_matches("./").to_string(),
                side: match raw.side.as_deref().map(str::trim) {
                    Some("L" | "l" | "old") => CommentSide::Old,
                    _ => CommentSide::New,
                },
                line: raw.line,
                text: raw.text.filter(|text| !text.trim().is_empty()),
                severity: raw.severity,
                title: raw.title,
                body: raw.body,
                placement: Placement::Fresh,
            })
            .collect(),
    )
}

fn same_text(a: &str, b: &str) -> bool {
    a.trim() == b.trim()
}

fn locate(files: &[FileDiff], finding: &Finding) -> Option<(u32, String)> {
    let file = files.iter().find(|file| {
        file.path == finding.path || file.old_path.as_deref() == Some(finding.path.as_str())
    })?;
    let lines =
        file.hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .filter_map(|line| match line_anchor(line) {
                Some((side, no)) if side == finding.side => Some((no, line.text.as_str())),
                _ => None,
            });
    match &finding.text {
        None => lines
            .into_iter()
            .find(|(no, _)| *no == finding.line)
            .map(|(no, text)| (no, text.to_string())),
        Some(want) => lines
            .filter(|(no, text)| {
                same_text(text, want) && no.abs_diff(finding.line) <= REANCHOR_WINDOW
            })
            .min_by_key(|(no, _)| no.abs_diff(finding.line))
            .map(|(no, text)| (no, text.to_string())),
    }
}

pub fn place_new(findings: &mut [Finding], files: &[FileDiff]) {
    for finding in findings {
        match locate(files, finding) {
            Some((line, text)) => {
                finding.line = line;
                finding.text = Some(text);
                finding.placement = Placement::Fresh;
            }
            None => finding.placement = Placement::Unanchored,
        }
    }
}

pub fn refresh(findings: &mut [Finding], files: &[FileDiff]) -> bool {
    let mut changed = false;
    for finding in findings
        .iter_mut()
        .filter(|finding| finding.placement != Placement::Unanchored)
    {
        let next = match locate(files, finding) {
            Some((line, _)) => {
                changed |= finding.line != line;
                finding.line = line;
                Placement::Fresh
            }
            None => Placement::Outdated,
        };
        changed |= finding.placement != next;
        finding.placement = next;
    }
    changed
}

pub struct PromptInput<'a> {
    pub scope: ReviewScope,
    pub base: Option<&'a str>,
    pub patch: &'a str,
    pub truncated: bool,
    pub files: &'a [String],
}

pub fn prompt(input: PromptInput) -> String {
    let scope = input.scope.label(input.base);
    let mut out = format!(
        "Review these code changes ({scope}). Report problems only: never edit, create, or delete files, and never commit. You may read files in this checkout for context.\n\n"
    );
    let fits = !input.truncated && input.patch.len() <= MAX_PATCH_BYTES;
    if fits {
        out.push_str("```diff\n");
        out.push_str(input.patch.trim_end());
        out.push_str("\n```\n\n");
    } else {
        let how = match input.scope {
            ReviewScope::Uncommitted => {
                "Read it with `git diff HEAD`, plus `git status --porcelain` for new files.".into()
            }
            ReviewScope::Branch => format!(
                "Read it with `git diff $(git merge-base {} HEAD)`, plus `git status --porcelain` for new files.",
                input.base.unwrap_or("main")
            ),
            ReviewScope::LatestTurn => "Read the changed files listed below.".into(),
        };
        out.push_str(&format!(
            "The diff is too large to include. {how}\nChanged files:\n"
        ));
        for file in input.files {
            out.push_str(&format!("- {file}\n"));
        }
        out.push('\n');
    }
    out.push_str(concat!(
        "Look for correctness bugs, regressions, security problems, and risky assumptions. ",
        "Skip style preferences unless they hide a bug.\n\n",
        "End your reply with exactly one fenced code block whose info string is zeron-review, ",
        "holding a JSON array of findings (an empty array when you found nothing):\n\n",
        "```zeron-review\n",
        "[{\"path\": \"src/lib.rs\", \"side\": \"R\", \"line\": 42, \"text\": \"let run = snapshot.run.unwrap();\", ",
        "\"severity\": \"bug\", \"title\": \"One line summary\", \"body\": \"Why it is wrong and how to fix it.\"}]\n",
        "```\n\n",
        "path is the file path exactly as the diff shows it. side is \"R\" for an added or unchanged line ",
        "(line = its number in the changed file) and \"L\" for a removed line (line = its number in the ",
        "original file). text is that line's exact content without the +/- marker. ",
        "severity is \"bug\", \"risk\", or \"nit\"."
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changes::parse_patch;

    const PATCH: &str = "diff --git a/src/session.rs b/src/session.rs
--- a/src/session.rs
+++ b/src/session.rs
@@ -40,4 +40,5 @@
     let snapshot = store.load(id)?;
-    let run = snapshot.run.clone().unwrap();
+    let run = snapshot.run.clone().unwrap_or_default();
+    self.pending.remove(&id);
     Ok(run)
";

    fn reply(block: &str) -> String {
        format!("I looked at the diff.\n\n```zeron-review\n{block}\n```\n")
    }

    fn one(json: &str) -> Finding {
        parse_findings(&reply(json)).unwrap().remove(0)
    }

    #[test]
    fn parses_the_last_block_and_an_empty_array_means_clean() {
        let text = format!(
            "{}\nSecond pass:\n{}",
            reply("[{\"path\":\"a\",\"line\":1,\"severity\":\"nit\",\"title\":\"x\"}]"),
            reply("[]")
        );
        assert_eq!(parse_findings(&text), Some(Vec::new()));
        let found = one(
            "[{\"path\":\"./src/a.rs\",\"side\":\"L\",\"line\":3,\"severity\":\"bug\",\"title\":\"t\",\"body\":\"b\"}]",
        );
        assert_eq!(found.path, "src/a.rs");
        assert_eq!(found.side, CommentSide::Old);
        assert_eq!(found.severity, Severity::Bug);
    }

    #[test]
    fn a_missing_or_garbled_block_is_unreadable_not_clean() {
        assert_eq!(parse_findings("No issues found."), None);
        assert_eq!(parse_findings(&reply("[{\"path\": ")), None);
        assert_eq!(parse_findings("```zeron-review\n[]"), None);
    }

    #[test]
    fn findings_anchor_by_text_near_the_cited_line() {
        let files = parse_patch(PATCH);
        let mut findings = vec![
            one(
                "[{\"path\":\"src/session.rs\",\"line\":43,\"text\":\"self.pending.remove(&id);\",\"severity\":\"bug\",\"title\":\"t\"}]",
            ),
            one(
                "[{\"path\":\"src/session.rs\",\"side\":\"L\",\"line\":41,\"severity\":\"risk\",\"title\":\"t\"}]",
            ),
            one("[{\"path\":\"src/other.rs\",\"line\":1,\"severity\":\"nit\",\"title\":\"t\"}]"),
        ];
        place_new(&mut findings, &files);
        assert_eq!(
            (findings[0].line, findings[0].placement),
            (42, Placement::Fresh)
        );
        assert_eq!(
            findings[1].text.as_deref(),
            Some("    let run = snapshot.run.clone().unwrap();")
        );
        assert_eq!(findings[2].placement, Placement::Unanchored);
    }

    #[test]
    fn an_edited_line_goes_outdated_and_a_moved_one_follows() {
        let mut findings = vec![
            one(
                "[{\"path\":\"src/session.rs\",\"line\":42,\"text\":\"self.pending.remove(&id);\",\"severity\":\"bug\",\"title\":\"t\"}]",
            ),
            one(
                "[{\"path\":\"src/session.rs\",\"line\":41,\"text\":\"let run = snapshot.run.clone().unwrap_or_default();\",\"severity\":\"risk\",\"title\":\"t\"}]",
            ),
        ];
        place_new(&mut findings, &parse_patch(PATCH));
        let edited = PATCH
            .replace(
                "+    self.pending.remove(&id);",
                "+    self.persist(&snapshot)?;",
            )
            .replace("@@ -40,4 +40,5 @@", "@@ -40,4 +40,6 @@\n+    // moved down");
        assert!(refresh(&mut findings, &parse_patch(&edited)));
        assert_eq!(findings[0].placement, Placement::Outdated);
        assert_eq!(
            (findings[1].line, findings[1].placement),
            (42, Placement::Fresh)
        );
    }

    #[test]
    fn a_staged_finding_leads_with_its_severity() {
        let finding = one(
            "[{\"path\":\"src/a.rs\",\"line\":7,\"severity\":\"risk\",\"title\":\"Leaks\",\"body\":\"Close it.\"}]",
        );
        let comment = finding.comment();
        assert_eq!(comment.body, "Risk: Leaks\nClose it.");
        assert_eq!(comment.location(), "src/a.rs:7");
    }

    #[test]
    fn the_request_id_carries_the_scope() {
        let id = request_id(ReviewScope::Branch, "abc");
        assert!(id.starts_with(REQUEST_PREFIX));
        assert_eq!(request_scope(&id), Some(ReviewScope::Branch));
        assert_eq!(request_scope("other:branch:x"), None);
    }
}
