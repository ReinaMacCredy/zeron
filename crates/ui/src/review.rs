use std::collections::{BTreeSet, HashMap};

use zeron_engine::registry::HarnessDescriptor;
use zeron_engine::review::{FindingSide, ReviewFinding};
pub use zeron_engine::review::{
    REQUEST_PREFIX, ReviewScope, ReviewSettings, ReviewerChoice, ReviewerOptions, is_review,
    parse_request, resolve_reviewer,
};
use zeron_proto::{
    Delegation, FindingOutcome, FindingOutcomeKind, HarnessId, Model, ReasoningLevel,
};
use zeron_rpc::methods;

use crate::changes::{FileDiff, line_anchor};
use crate::comments::{CommentSide, ReviewComment};

pub const CARD_PREFIX: &str = "review-finding:";
const REANCHOR_WINDOW: u32 = 40;

pub fn brand(
    options: &ReviewerOptions,
    choice: &ReviewerChoice,
    chat: Option<HarnessId>,
) -> (&'static str, Option<gpui::Hsla>) {
    let (harness, model) = match choice {
        ReviewerChoice::Harness { harness, model, .. } => (Some(*harness), model.clone()),
        ReviewerChoice::Profile { id }
            if let Some(profile) = options.profiles.iter().find(|profile| &profile.id == id) =>
        {
            (Some(profile.harness), profile.model.clone())
        }
        _ => {
            let reviewer = resolve_reviewer(choice, chat, options);
            (reviewer.harness, reviewer.model)
        }
    };
    crate::pickers::agent_brand_icon(harness, model.as_deref())
}

pub async fn review_settings(engine: &crate::state::EngineHandle) -> ReviewSettings {
    engine
        .client()
        .call_as::<ReviewSettings>(methods::GET_REVIEW_SETTINGS, serde_json::json!({}))
        .await
        .unwrap_or_default()
}

#[derive(Debug, Clone, Default)]
pub struct ChatReview {
    pub scope: Option<ReviewScope>,
    pub base: Option<String>,
    pub stats: HashMap<ReviewScope, Result<(u32, u32), String>>,
    pub stats_key: Option<String>,
    pub run: Option<ReviewRun>,
    pub reviewer: Option<ReviewerChoice>,
    pub commit: Option<CommitPick>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommitPick {
    pub sha: String,
    pub subject: String,
}

impl CommitPick {
    pub fn short(&self) -> &str {
        short_sha(&self.sha)
    }
}

pub fn short_sha(sha: &str) -> &str {
    sha.char_indices()
        .nth(7)
        .map_or(sha, |(end, _)| &sha[..end])
}

impl ChatReview {
    pub fn changes(&self, scope: ReviewScope) -> Option<(u32, u32)> {
        self.stats.get(&scope)?.as_ref().ok().copied()
    }

    pub fn unavailable(&self, scope: ReviewScope) -> Option<&str> {
        self.stats.get(&scope)?.as_ref().err().map(String::as_str)
    }

    pub fn cards(&self) -> Vec<ReviewComment> {
        self.run
            .iter()
            .filter(|run| run.status == RunStatus::Done)
            .flat_map(|run| {
                run.findings
                    .iter()
                    .filter(|finding| run.outcome(&finding.id).is_none())
            })
            .filter(|finding| finding.placement == Placement::Fresh)
            .map(Finding::card)
            .collect()
    }

    pub fn finding(&self, id: &str) -> Option<&Finding> {
        self.run
            .as_ref()?
            .findings
            .iter()
            .find(|finding| finding.id == id)
    }

    pub fn take_fixable(&mut self, only: Option<&str>) -> Vec<ReviewComment> {
        let Some(run) = self.run.as_mut() else {
            return Vec::new();
        };
        let staged: Vec<Finding> = match only {
            None => run.fixable().cloned().collect(),
            Some(id) => run
                .findings
                .iter()
                .filter(|finding| {
                    finding.id == id
                        && finding.placement != Placement::Outdated
                        && run.outcome(id).is_none()
                })
                .cloned()
                .collect(),
        };
        run.handed_here
            .extend(staged.iter().map(|finding| finding.id.clone()));
        staged.iter().map(Finding::comment).collect()
    }

    pub fn dismiss(&mut self, id: &str) {
        if let Some(run) = self.run.as_mut() {
            run.findings.retain(|finding| finding.id != id);
        }
    }
}

pub async fn list_models(engine: &crate::state::EngineHandle, harness: HarnessId) -> Vec<Model> {
    engine
        .client()
        .call(
            methods::LIST_MODELS,
            serde_json::json!({ "harness": harness, "force": false }),
        )
        .await
        .ok()
        .and_then(|value| serde_json::from_value::<Vec<Model>>(value["models"].clone()).ok())
        .unwrap_or_default()
}

pub async fn reviewer_options(engine: &crate::state::EngineHandle) -> ReviewerOptions {
    let descriptors = engine
        .client()
        .call_as::<Vec<HarnessDescriptor>>(methods::LIST_HARNESSES, serde_json::json!({}))
        .await
        .map(|list| crate::pickers::offered_harnesses(&list))
        .unwrap_or_default();
    let profiles = crate::agent_profiles::list(engine, None)
        .await
        .unwrap_or_default();
    ReviewerOptions {
        offered: descriptors.iter().map(|descriptor| descriptor.id).collect(),
        levels: descriptors
            .into_iter()
            .map(|descriptor| (descriptor.id, descriptor.reasoning_levels))
            .collect(),
        profiles,
    }
}

pub fn reasoning_stops(
    options: &ReviewerOptions,
    harness: HarnessId,
    model: Option<&Model>,
) -> Vec<ReasoningLevel> {
    model
        .map(|model| model.reasoning_levels.clone())
        .filter(|levels| !levels.is_empty())
        .or_else(|| options.levels.get(&harness).cloned())
        .unwrap_or_default()
}
#[derive(Debug, Clone, PartialEq)]
pub enum RunStatus {
    Starting,
    Running,
    Reading,
    Done,
    Unreadable,
    Stopped,
    Failed(String),
}

#[derive(Debug, PartialEq)]
pub enum ReadOutcome {
    Stopped,
    Failed(String),
    Read(Option<Vec<Finding>>),
}

pub fn read_outcome(status: &str, summary: &str) -> ReadOutcome {
    match status {
        "cancelled" | "interrupted" => ReadOutcome::Stopped,
        "failed" => ReadOutcome::Failed(summary.to_string()),
        _ => ReadOutcome::Read(
            zeron_engine::review::parse_findings(summary)
                .map(|findings| findings.into_iter().map(Finding::from).collect()),
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingState {
    Open,
    AgentOnIt,
    Fixed,
    Skipped,
    NoChange,
}

impl FindingState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::AgentOnIt => "Agent on it",
            Self::Fixed => "Fixed",
            Self::Skipped => "Skipped",
            Self::NoChange => "No change",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReviewRun {
    pub scope: ReviewScope,
    pub task_id: Option<String>,
    pub status: RunStatus,
    pub findings: Vec<Finding>,
    pub reported: usize,
    pub reviewed: Option<String>,
    pub note: Option<String>,
    pub choice: Option<ReviewerChoice>,
    pub retry_error: Option<String>,
    pub commit: Option<String>,
    pub delegation: Option<Delegation>,
    pub owner_working: bool,
    pub handed_here: BTreeSet<String>,
}

impl ReviewRun {
    pub fn new(scope: ReviewScope) -> Self {
        Self {
            scope,
            task_id: None,
            status: RunStatus::Starting,
            findings: Vec::new(),
            reported: 0,
            reviewed: None,
            note: None,
            choice: None,
            retry_error: None,
            commit: None,
            delegation: None,
            owner_working: false,
            handed_here: BTreeSet::new(),
        }
    }

    pub fn outcome(&self, id: &str) -> Option<&FindingOutcome> {
        self.delegation.as_ref()?.finding_outcomes.get(id)
    }

    pub fn state(&self, id: &str) -> FindingState {
        let handed = self.handed_here.contains(id)
            || self
                .delegation
                .as_ref()
                .is_some_and(|d| d.finding_handed(id));
        match self.outcome(id).map(|outcome| outcome.outcome) {
            Some(FindingOutcomeKind::Fixed) => FindingState::Fixed,
            Some(FindingOutcomeKind::Skipped) => FindingState::Skipped,
            Some(FindingOutcomeKind::NoChange) => FindingState::NoChange,
            None if handed && self.owner_working => FindingState::AgentOnIt,
            None => FindingState::Open,
        }
    }

    pub fn fixable(&self) -> impl Iterator<Item = &Finding> {
        self.findings.iter().filter(|finding| {
            finding.placement != Placement::Outdated
                && self.state(&finding.id) == FindingState::Open
                && !self.handed_here.contains(&finding.id)
                && !self
                    .delegation
                    .as_ref()
                    .is_some_and(|d| d.findings_handed.contains(&finding.id))
        })
    }

    pub fn unsent_hands(&self) -> Vec<String> {
        self.handed_here
            .iter()
            .filter(|id| {
                !self
                    .delegation
                    .as_ref()
                    .is_some_and(|d| d.findings_handed.contains(*id))
            })
            .cloned()
            .collect()
    }

    pub fn busy(&self) -> bool {
        matches!(
            self.status,
            RunStatus::Starting | RunStatus::Running | RunStatus::Reading
        )
    }

    pub fn outdated(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| {
                finding.placement == Placement::Outdated && self.outcome(&finding.id).is_none()
            })
            .count()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Bug,
    Risk,
    Nit,
}

impl From<zeron_engine::review::Severity> for Severity {
    fn from(severity: zeron_engine::review::Severity) -> Self {
        match severity {
            zeron_engine::review::Severity::Bug => Self::Bug,
            zeron_engine::review::Severity::Risk => Self::Risk,
            zeron_engine::review::Severity::Nit => Self::Nit,
        }
    }
}

impl Severity {
    pub fn word(self) -> &'static str {
        match self {
            Self::Bug => "Bug",
            Self::Risk => "Risk",
            Self::Nit => "Nit",
        }
    }

    pub fn colors(self, theme: &crate::theme::Theme) -> (gpui::Hsla, gpui::Hsla) {
        match self {
            Self::Bug => (theme.danger, theme.danger.opacity(0.14)),
            Self::Risk => (theme.danger.opacity(0.6), theme.danger.opacity(0.08)),
            Self::Nit => (theme.text_muted, crate::theme::ink(0.08)),
        }
    }
}

pub fn split_finding(body: &str) -> (Option<Severity>, &str, &str) {
    let found = [Severity::Bug, Severity::Risk, Severity::Nit]
        .into_iter()
        .find_map(|severity| {
            let rest = body.strip_prefix(severity.word())?.strip_prefix(": ")?;
            Some((severity, rest))
        });
    let Some((severity, rest)) = found else {
        return (None, "", body);
    };
    let (title, detail) = rest.split_once('\n').unwrap_or((rest, ""));
    (Some(severity), title.trim(), detail.trim())
}

pub fn finding_card_height(body: &str) -> f32 {
    match split_finding(body).2 {
        "" => crate::comments::card_height("") - crate::comments::CARD_LINE_HEIGHT,
        detail => crate::comments::card_height(detail),
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
        let mut body = format!(
            "{}: [{}] {}",
            self.severity.word(),
            self.id,
            self.title.trim()
        );
        if !self.body.trim().is_empty() {
            body.push('\n');
            body.push_str(self.body.trim());
        }
        ReviewComment::new(self.path.clone(), self.side, self.line, body)
    }

    fn card(&self) -> ReviewComment {
        ReviewComment {
            id: format!("{CARD_PREFIX}{}", self.id),
            ..self.comment()
        }
    }
}

impl From<ReviewFinding> for Finding {
    fn from(finding: ReviewFinding) -> Self {
        Self {
            id: finding.id,
            path: finding.path,
            side: match finding.side {
                FindingSide::Old => CommentSide::Old,
                FindingSide::New => CommentSide::New,
            },
            line: finding.line,
            text: finding.text,
            severity: finding.severity.into(),
            title: finding.title,
            body: finding.body,
            placement: Placement::Fresh,
        }
    }
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
        let located = locate(files, finding).or_else(|| {
            let cited = Finding {
                text: None,
                ..finding.clone()
            };
            locate(files, &cited)
        });
        match located {
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
        match read_outcome("completed", &reply(json)) {
            ReadOutcome::Read(Some(mut findings)) => findings.remove(0),
            other => panic!("unread: {other:?}"),
        }
    }

    #[test]
    fn short_sha_cuts_on_a_char_boundary() {
        assert_eq!(short_sha("abc123é"), "abc123é");
        assert_eq!(short_sha("abc123éf9"), "abc123é");
        assert_eq!(short_sha("a1b2c3d4e5"), "a1b2c3d");
        assert_eq!(short_sha("abc"), "abc");
    }

    #[test]
    fn a_stopped_reviewer_reads_as_stopped_not_as_a_missing_block() {
        let partial = "Looking at session.rs, the first thing I notice";
        assert_eq!(read_outcome("cancelled", partial), ReadOutcome::Stopped);
        assert_eq!(read_outcome("interrupted", partial), ReadOutcome::Stopped);
        assert_eq!(read_outcome("completed", partial), ReadOutcome::Read(None));
        assert_eq!(
            read_outcome("failed", "boom"),
            ReadOutcome::Failed("boom".to_string())
        );
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
        findings[1].id = "F2".into();
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

        let mut run = ReviewRun::new(ReviewScope::Uncommitted);
        run.status = RunStatus::Done;
        run.findings = findings;
        let mut review = ChatReview {
            run: Some(run),
            ..ChatReview::default()
        };
        let cards = review.cards();
        assert_eq!(cards.len(), 1);
        assert!(cards[0].id.starts_with(CARD_PREFIX));
        let staged = review.take_fixable(None);
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].location(), "src/session.rs:42");
        assert!(
            review.take_fixable(None).is_empty(),
            "a handed row stays put"
        );
        let run = review.run.unwrap();
        assert_eq!(run.findings.len(), 2);
        assert_eq!(run.findings[0].placement, Placement::Outdated);
        assert_eq!(run.unsent_hands(), ["F2"]);
    }

    fn outcome(kind: FindingOutcomeKind) -> FindingOutcome {
        FindingOutcome {
            outcome: kind,
            note: Some("why".into()),
            commit: None,
        }
    }

    #[test]
    fn outcomes_set_each_row_and_only_open_rows_stay_fixable() {
        let mut findings = vec![
            one("[{\"path\":\"a\",\"line\":1,\"severity\":\"bug\",\"title\":\"t\"}]"),
            one("[{\"path\":\"b\",\"line\":2,\"severity\":\"risk\",\"title\":\"t\"}]"),
            one("[{\"path\":\"c\",\"line\":3,\"severity\":\"nit\",\"title\":\"t\"}]"),
        ];
        for (ix, finding) in findings.iter_mut().enumerate() {
            finding.id = format!("F{}", ix + 1);
        }
        let mut delegation: Delegation = serde_json::from_value(serde_json::json!({
            "delegatedByChatId": "owner",
            "depth": 1,
            "anchor": "task",
            "clientRequestId": "zeron-review:uncommitted:n",
            "wake": "always",
            "status": "completed",
            "startedAt": "2026-10-17T10:00:00Z",
        }))
        .unwrap();
        delegation
            .finding_outcomes
            .insert("F1".into(), outcome(FindingOutcomeKind::Fixed));
        delegation
            .finding_outcomes
            .insert("F2".into(), outcome(FindingOutcomeKind::Skipped));
        let mut run = ReviewRun::new(ReviewScope::Uncommitted);
        run.status = RunStatus::Done;
        run.findings = findings;
        run.delegation = Some(delegation);
        let states = |run: &ReviewRun| -> Vec<FindingState> {
            run.findings.iter().map(|f| run.state(&f.id)).collect()
        };
        assert_eq!(
            states(&run),
            [
                FindingState::Fixed,
                FindingState::Skipped,
                FindingState::Open
            ]
        );
        run.owner_working = true;
        assert_eq!(states(&run)[2], FindingState::AgentOnIt);
        assert_eq!(run.fixable().count(), 0, "the agent is on it");
        run.owner_working = false;
        let mut review = ChatReview {
            run: Some(run),
            ..ChatReview::default()
        };
        let cards = review.cards();
        assert_eq!(cards.len(), 1, "resolved findings leave Changes");
        assert!(cards[0].id.ends_with("F3"));
        let staged = review.take_fixable(None);
        assert_eq!(staged.len(), 1);
        assert!(staged[0].body.starts_with("Nit: [F3] "));
        assert!(
            review.take_fixable(Some("F1")).is_empty(),
            "a fixed row is done"
        );
    }

    #[test]
    fn a_staged_finding_leads_with_its_severity() {
        let finding = one(
            "[{\"path\":\"src/a.rs\",\"line\":7,\"severity\":\"risk\",\"title\":\"Leaks\",\"body\":\"Close it.\"}]",
        );
        let comment = finding.comment();
        assert_eq!(comment.body, "Risk: [F1] Leaks\nClose it.");
        assert_eq!(split_finding(&comment.body).1, "[F1] Leaks");
        assert_eq!(comment.location(), "src/a.rs:7");
    }
}
