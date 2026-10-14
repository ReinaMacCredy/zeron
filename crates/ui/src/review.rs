use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use zeron_engine::registry::HarnessDescriptor;
use zeron_proto::{AgentProfile, HarnessId, Model, ReasoningLevel};
use zeron_rpc::methods;

use crate::changes::{FileDiff, line_anchor};
use crate::comments::{CommentSide, ReviewComment};
use crate::composer::harness_display_name;

pub const REQUEST_PREFIX: &str = "zeron-review:";
pub const CARD_PREFIX: &str = "review-finding:";
const FENCE: &str = "```zeron-review";
const MAX_PATCH_BYTES: usize = 120_000;
const REANCHOR_WINDOW: u32 = 40;

#[derive(Debug, Clone, Default)]
pub struct ChatReview {
    pub scope: Option<ReviewScope>,
    pub base: Option<String>,
    pub stats: HashMap<ReviewScope, Result<(u32, u32), String>>,
    pub stats_key: Option<String>,
    pub run: Option<ReviewRun>,
    pub reviewer: Option<ReviewerChoice>,
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
            .flat_map(|run| &run.findings)
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
        let (staged, kept): (Vec<_>, Vec<_>) = run.findings.drain(..).partition(|finding| {
            finding.placement != Placement::Outdated && only.is_none_or(|id| finding.id == id)
        });
        run.findings = kept;
        staged.iter().map(Finding::comment).collect()
    }

    pub fn dismiss(&mut self, id: &str) {
        if let Some(run) = self.run.as_mut() {
            run.findings.retain(|finding| finding.id != id);
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ReviewerChoice {
    #[default]
    Auto,
    Harness {
        harness: HarnessId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning: Option<ReasoningLevel>,
    },
    Profile {
        id: String,
    },
}

#[derive(Debug, Clone, Default)]
pub struct ReviewerOptions {
    pub offered: Vec<HarnessId>,
    pub profiles: Vec<AgentProfile>,
    pub levels: HashMap<HarnessId, Vec<ReasoningLevel>>,
}

impl ReviewerOptions {
    pub fn label(&self, choice: &ReviewerChoice) -> String {
        match choice {
            ReviewerChoice::Auto => "Auto".to_string(),
            ReviewerChoice::Harness { harness, model, .. } => match model {
                Some(model) => format!("{} · {model}", harness_display_name(*harness)),
                None => harness_display_name(*harness).to_string(),
            },
            ReviewerChoice::Profile { id } => self
                .profiles
                .iter()
                .find(|profile| &profile.id == id)
                .map_or_else(
                    || "Missing profile".to_string(),
                    |profile| profile.name.clone(),
                ),
        }
    }

    pub fn brand(
        &self,
        choice: &ReviewerChoice,
        chat: Option<HarnessId>,
    ) -> (&'static str, Option<gpui::Hsla>) {
        let (harness, model) = match choice {
            ReviewerChoice::Harness { harness, model, .. } => (Some(*harness), model.clone()),
            ReviewerChoice::Profile { id }
                if let Some(profile) = self.profiles.iter().find(|profile| &profile.id == id) =>
            {
                (Some(profile.harness), profile.model.clone())
            }
            _ => {
                let reviewer = resolve_reviewer(choice, chat, self);
                (reviewer.harness, reviewer.model)
            }
        };
        crate::pickers::agent_brand_icon(harness, model.as_deref())
    }

    pub fn choices(&self) -> Vec<ReviewerChoice> {
        std::iter::once(ReviewerChoice::Auto)
            .chain(self.offered.iter().map(|harness| ReviewerChoice::Harness {
                harness: *harness,
                model: None,
                reasoning: None,
            }))
            .chain(self.profiles.iter().map(|profile| ReviewerChoice::Profile {
                id: profile.id.clone(),
            }))
            .collect()
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

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reviewer {
    pub harness: Option<HarnessId>,
    pub model: Option<String>,
    pub reasoning: Option<ReasoningLevel>,
    pub profile: Option<AgentProfile>,
    pub note: Option<String>,
}

impl Reviewer {
    pub fn name(&self) -> String {
        match (&self.profile, self.harness) {
            (Some(profile), _) => profile.name.clone(),
            (None, Some(harness)) => harness_display_name(harness).to_string(),
            (None, None) => "the chat's agent".to_string(),
        }
    }
}

pub fn other_family(chat: Option<HarnessId>, offered: &[HarnessId]) -> Option<HarnessId> {
    let wanted: &[HarnessId] = match chat {
        Some(HarnessId::ClaudeCode) => &[HarnessId::Codex],
        Some(HarnessId::Codex) => &[HarnessId::ClaudeCode],
        _ => &[HarnessId::ClaudeCode, HarnessId::Codex],
    };
    wanted.iter().copied().find(|id| offered.contains(id))
}

pub fn resolve_reviewer(
    choice: &ReviewerChoice,
    chat: Option<HarnessId>,
    options: &ReviewerOptions,
) -> Reviewer {
    let usable = |profile: &AgentProfile| options.offered.contains(&profile.harness);
    let from_profile = |profile: &AgentProfile| Reviewer {
        harness: Some(profile.harness),
        model: profile.model.clone(),
        reasoning: profile.reasoning,
        profile: Some(profile.clone()),
        note: None,
    };
    let skipped = match choice {
        ReviewerChoice::Auto => None,
        ReviewerChoice::Harness {
            harness,
            model,
            reasoning,
        } if options.offered.contains(harness) => {
            return Reviewer {
                harness: Some(*harness),
                model: model.clone(),
                reasoning: *reasoning,
                ..Reviewer::default()
            };
        }
        ReviewerChoice::Harness { harness, .. } => {
            Some(format!("{} is unavailable", harness_display_name(*harness)))
        }
        ReviewerChoice::Profile { id } => match options.profiles.iter().find(|p| &p.id == id) {
            Some(profile) if usable(profile) => return from_profile(profile),
            Some(profile) => Some(format!("{} is unavailable", profile.name)),
            None => Some("The chosen profile was deleted".to_string()),
        },
    };
    let named = options
        .profiles
        .iter()
        .find(|profile| profile.name.trim().eq_ignore_ascii_case("reviewer") && usable(profile));
    let mut reviewer = match (named, other_family(chat, &options.offered)) {
        (Some(profile), _) => from_profile(profile),
        (None, Some(harness)) => Reviewer {
            harness: Some(harness),
            ..Reviewer::default()
        },
        (None, None) => {
            let own = Reviewer {
                harness: chat,
                ..Reviewer::default()
            };
            Reviewer {
                note: Some(format!(
                    "No other agent is available, so {} reviews its own work",
                    own.name()
                )),
                ..own
            }
        }
    };
    if let Some(skipped) = skipped {
        reviewer.note = Some(format!("{skipped}, so {} reviews instead", reviewer.name()));
    }
    reviewer
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
        _ => ReadOutcome::Read(parse_findings(summary)),
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
        }
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
            .filter(|finding| finding.placement == Placement::Outdated)
            .count()
    }
}

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
        let mut body = format!("{}: {}", self.severity.word(), self.title.trim());
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

#[derive(Deserialize)]
struct RawFinding {
    path: String,
    #[serde(default)]
    side: Option<String>,
    line: serde_json::Value,
    #[serde(default)]
    text: Option<String>,
    severity: String,
    title: String,
    #[serde(default)]
    body: String,
}

fn raw_finding(value: serde_json::Value) -> Option<RawFinding> {
    serde_json::from_value(value).ok()
}

fn finding_from(ix: usize, raw: RawFinding) -> Option<Finding> {
    let line = match &raw.line {
        serde_json::Value::Number(n) => u32::try_from(n.as_u64()?).ok()?,
        serde_json::Value::String(s) => s.trim().parse().ok()?,
        _ => return None,
    };
    let severity = match raw.severity.trim().to_ascii_lowercase().as_str() {
        "bug" => Severity::Bug,
        "risk" => Severity::Risk,
        "nit" => Severity::Nit,
        _ => return None,
    };
    Some(Finding {
        id: format!("finding-{ix}"),
        path: raw.path.trim().trim_start_matches("./").to_string(),
        side: match raw.side.as_deref().map(str::trim) {
            Some("L" | "l" | "old") => CommentSide::Old,
            _ => CommentSide::New,
        },
        line,
        text: raw.text.filter(|text| !text.trim().is_empty()),
        severity,
        title: raw.title,
        body: raw.body,
        placement: Placement::Fresh,
    })
}

pub fn parse_findings(result: &str) -> Option<Vec<Finding>> {
    let start = result.rfind(FENCE)? + FENCE.len();
    let rest = result[start..].split_once('\n')?.1;
    let end = if rest.starts_with("```") {
        0
    } else {
        rest.find("\n```")?
    };
    let items: Vec<serde_json::Value> = serde_json::from_str(rest[..end].trim()).ok()?;
    let total = items.len();
    let findings: Vec<Finding> = items
        .into_iter()
        .filter_map(raw_finding)
        .enumerate()
        .filter_map(|(ix, raw)| finding_from(ix, raw))
        .collect();
    (total == 0 || !findings.is_empty()).then_some(findings)
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
    fn one_bad_item_does_not_sink_the_block() {
        let found = parse_findings(&reply(
            "[{\"path\":\"a\",\"line\":\"4\",\"severity\":\"Bug\",\"title\":\"x\",\"text\":\"/// ```rust\"},\
             {\"path\":\"b\",\"line\":2,\"severity\":\"high\",\"title\":\"y\"}]",
        ))
        .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].severity), (4, Severity::Bug));
        assert_eq!(
            parse_findings(&reply(
                "[{\"path\":\"b\",\"line\":2,\"severity\":\"high\",\"title\":\"y\"}]"
            )),
            None
        );
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
        assert_eq!(
            review.run.unwrap().findings[0].placement,
            Placement::Outdated
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

    fn profile(id: &str, name: &str, harness: HarnessId) -> AgentProfile {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "name": name,
            "harness": harness,
        }))
        .unwrap()
    }

    #[test]
    fn the_reviewer_resolves_by_choice_then_falls_back_with_a_note() {
        let options = ReviewerOptions {
            offered: vec![HarnessId::ClaudeCode, HarnessId::Codex],
            profiles: vec![profile("p1", "Reviewer", HarnessId::Codex)],
            levels: HashMap::new(),
        };
        let picked = resolve_reviewer(
            &ReviewerChoice::Harness {
                harness: HarnessId::ClaudeCode,
                model: None,
                reasoning: Some(ReasoningLevel::High),
            },
            Some(HarnessId::ClaudeCode),
            &options,
        );
        assert_eq!(
            (picked.harness, picked.reasoning),
            (Some(HarnessId::ClaudeCode), Some(ReasoningLevel::High))
        );
        let auto = resolve_reviewer(&ReviewerChoice::Auto, Some(HarnessId::Codex), &options);
        assert_eq!(auto.profile.map(|p| p.id), Some("p1".to_string()));
        assert_eq!(auto.note, None);

        let no_profiles = ReviewerOptions {
            profiles: Vec::new(),
            ..options.clone()
        };
        let auto = resolve_reviewer(&ReviewerChoice::Auto, Some(HarnessId::Codex), &no_profiles);
        assert_eq!(auto.harness, Some(HarnessId::ClaudeCode));

        let alone = ReviewerOptions {
            offered: vec![HarnessId::Codex],
            profiles: Vec::new(),
            levels: HashMap::new(),
        };
        let own = resolve_reviewer(&ReviewerChoice::Auto, Some(HarnessId::Codex), &alone);
        assert_eq!(own.harness, Some(HarnessId::Codex));
        assert!(own.note.is_some());

        let gone = resolve_reviewer(
            &ReviewerChoice::Profile {
                id: "deleted".into(),
            },
            Some(HarnessId::ClaudeCode),
            &no_profiles,
        );
        assert_eq!(gone.harness, Some(HarnessId::Codex));
        assert!(gone.note.unwrap().contains("deleted"));

        let unavailable = resolve_reviewer(
            &ReviewerChoice::Harness {
                harness: HarnessId::Mock,
                model: None,
                reasoning: None,
            },
            Some(HarnessId::ClaudeCode),
            &no_profiles,
        );
        assert_eq!(unavailable.harness, Some(HarnessId::Codex));
        assert!(unavailable.note.is_some());
    }

    #[test]
    fn the_request_id_carries_the_scope() {
        let id = request_id(ReviewScope::Branch, "abc");
        assert!(id.starts_with(REQUEST_PREFIX));
        assert_eq!(request_scope(&id), Some(ReviewScope::Branch));
        assert_eq!(request_scope("other:branch:x"), None);
    }
}
