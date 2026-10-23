use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::comments::{CommentSide, ReviewComment};
use crate::patch::{FileDiff, line_anchor};
use crate::{
    AgentProfile, Chat, CheckoutDiff, Delegation, FindingOutcome, FindingOutcomeKind, HarnessId,
    ReasoningLevel,
};

pub const REQUEST_PREFIX: &str = "zeron-review:";
pub const CARD_PREFIX: &str = "review-finding:";
pub const STAGED_PREFIX: &str = "review-staged:";
const FENCE: &str = "```zeron-review";
const REANCHOR_WINDOW: u32 = 40;

pub fn is_commit_id(rev: &str) -> bool {
    (4..=64).contains(&rev.len()) && rev.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReviewScope {
    Uncommitted,
    Branch,
    LatestTurn,
    Commit,
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
            Self::Commit => "commit",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Uncommitted => "uncommitted",
            Self::Branch => "branch",
            Self::LatestTurn => "turn",
            Self::Commit => "commit",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .chain([Self::Commit])
            .find(|scope| scope.key() == key)
    }

    pub fn label(self, base: Option<&str>) -> String {
        match self {
            Self::Uncommitted => "Uncommitted".into(),
            Self::Branch => format!("Branch vs {}", base.unwrap_or("base")),
            Self::LatestTurn => "Latest turn".into(),
            Self::Commit => "Commit".into(),
        }
    }
}

pub fn request_id(scope: ReviewScope, commit: Option<&str>, nonce: &str) -> String {
    match commit {
        Some(sha) => format!("{REQUEST_PREFIX}{}:{nonce}:{sha}", scope.key()),
        None => format!("{REQUEST_PREFIX}{}:{nonce}", scope.key()),
    }
}

pub fn is_review(delegation: &Delegation) -> bool {
    delegation
        .client_request_id
        .as_deref()
        .is_some_and(|id| id.starts_with(REQUEST_PREFIX))
}

pub fn parse_request(request_id: &str) -> Option<(ReviewScope, Option<String>)> {
    let mut parts = request_id.strip_prefix(REQUEST_PREFIX)?.split(':');
    let scope = ReviewScope::from_key(parts.next()?)?;
    let commit = parts
        .nth(1)
        .filter(|sha| is_commit_id(sha))
        .map(str::to_string);
    Some((scope, commit))
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ReviewSettings {
    pub reviewer: ReviewerChoice,
    pub instructions: String,
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
                Some(model) => format!("{} · {model}", harness.display_name()),
                None => harness.display_name().to_string(),
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

    pub fn reasoning_stops(
        &self,
        harness: HarnessId,
        model_levels: Option<&[ReasoningLevel]>,
    ) -> Vec<ReasoningLevel> {
        model_levels
            .filter(|levels| !levels.is_empty())
            .map(<[ReasoningLevel]>::to_vec)
            .or_else(|| self.levels.get(&harness).cloned())
            .unwrap_or_default()
    }
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
            (None, Some(harness)) => harness.display_name().to_string(),
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
            Some(format!("{} is unavailable", harness.display_name()))
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

pub fn default_base_ref(branches: &[String], current: Option<&str>) -> Option<String> {
    let first = branches.first()?;
    if current != Some(first.as_str()) {
        return Some(first.clone());
    }
    for candidate in ["main", "master"] {
        if branches.iter().any(|b| b == candidate) {
            return Some(candidate.to_string());
        }
    }
    branches
        .iter()
        .find(|b| current != Some(b.as_str()))
        .or(Some(first))
        .cloned()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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

    pub fn meta(&self) -> String {
        match self.placement {
            Placement::Outdated => format!("{} · {} · outdated", self.id, self.location()),
            _ => format!("{} · {}", self.id, self.location()),
        }
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
        ReviewComment {
            id: format!("{STAGED_PREFIX}{}", self.id),
            ..ReviewComment::new(self.path.clone(), self.side, self.line, body)
        }
    }

    pub fn card(&self) -> ReviewComment {
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
        id: format!("F{}", ix + 1),
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
        .enumerate()
        .filter_map(|(ix, value)| {
            finding_from(ix, serde_json::from_value::<RawFinding>(value).ok()?)
        })
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

pub fn read_task_view(view: &serde_json::Value) -> ReadOutcome {
    let field = |key: &str| view.get(key).and_then(|v| v.as_str()).unwrap_or_default();
    read_outcome(field("status"), field("summary"))
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

    pub fn settled(self) -> bool {
        matches!(self, Self::Fixed | Self::NoChange)
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
    pub withdrawn: BTreeSet<String>,
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
            withdrawn: BTreeSet::new(),
        }
    }

    pub fn outcome(&self, id: &str) -> Option<&FindingOutcome> {
        self.delegation.as_ref()?.finding_outcomes.get(id)
    }

    pub fn state(&self, id: &str) -> FindingState {
        let handed = self.handed_here.contains(id)
            || !self.withdrawn.contains(id)
                && self
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

    pub fn state_label(&self, id: &str) -> String {
        let state = self.state(id);
        match self
            .outcome(id)
            .and_then(|outcome| outcome.commit.as_deref())
        {
            Some(sha) if state == FindingState::Fixed => format!("Fixed · {}", short_sha(sha)),
            _ => state.label().to_string(),
        }
    }

    pub fn reason(&self, id: &str) -> Option<String> {
        self.outcome(id)
            .filter(|_| {
                matches!(
                    self.state(id),
                    FindingState::Skipped | FindingState::NoChange
                )
            })
            .and_then(|outcome| outcome.note.clone())
    }

    pub fn fixable(&self) -> impl Iterator<Item = &Finding> {
        self.findings.iter().filter(|finding| {
            finding.placement != Placement::Outdated
                && self.state(&finding.id) == FindingState::Open
                && !self.handed(&finding.id)
        })
    }

    fn handed(&self, id: &str) -> bool {
        self.handed_here.contains(id)
            || !self.withdrawn.contains(id)
                && self
                    .delegation
                    .as_ref()
                    .is_some_and(|d| d.findings_handed.contains(id))
    }

    pub fn withdraw(&mut self, id: &str) {
        self.handed_here.remove(id);
        self.withdrawn.insert(id.to_owned());
    }

    pub fn unsent_withdrawals(&self) -> Vec<String> {
        self.withdrawn
            .iter()
            .filter(|id| {
                self.delegation
                    .as_ref()
                    .is_some_and(|d| d.findings_handed.contains(*id))
            })
            .cloned()
            .collect()
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

    pub fn stoppable(&self) -> bool {
        self.status == RunStatus::Running && self.task_id.is_some()
    }

    pub fn apply_read(&mut self, outcome: ReadOutcome, reviewed: Option<&str>, restored: bool) {
        match outcome {
            ReadOutcome::Stopped => self.status = RunStatus::Stopped,
            ReadOutcome::Failed(message) => self.status = RunStatus::Failed(message),
            ReadOutcome::Read(None) => self.status = RunStatus::Unreadable,
            ReadOutcome::Read(Some(mut findings)) => {
                let files = crate::patch::parse_patch(reviewed.unwrap_or_default());
                if restored {
                    refresh(&mut findings, &files);
                } else {
                    place_new(&mut findings, &files);
                }
                self.reported = findings.len();
                self.findings = findings;
                self.status = RunStatus::Done;
            }
        }
    }

    pub fn apply_start(
        &mut self,
        started: Result<StartedReview, String>,
        previous: Option<ReviewRun>,
    ) {
        if self.status != RunStatus::Starting {
            return;
        }
        match started {
            Ok(started) => {
                self.task_id = Some(started.task_id);
                self.reviewed = Some(started.patch);
                self.note = started.note;
                self.status = RunStatus::Running;
            }
            Err(message) => match previous {
                Some(mut previous) => {
                    previous.retry_error = Some(message);
                    *self = previous;
                }
                None => self.status = RunStatus::Failed(message),
            },
        }
    }

    pub fn sync_child(&mut self, delegation: Option<Delegation>, owner_working: bool) -> bool {
        let changed = self.delegation != delegation || self.owner_working != owner_working;
        self.delegation = delegation;
        self.owner_working = owner_working;
        changed
    }

    pub fn next_hand(&self) -> Option<(Vec<String>, bool)> {
        let withdrawals = self.unsent_withdrawals();
        if withdrawals.is_empty() {
            Some((self.unsent_hands(), false)).filter(|(ids, _)| !ids.is_empty())
        } else {
            Some((withdrawals, true))
        }
    }

    pub fn hand_failed(&mut self, ids: &[String], withdraw: bool) {
        let sent = if withdraw {
            &mut self.withdrawn
        } else {
            &mut self.handed_here
        };
        sent.retain(|id| !ids.contains(id));
    }

    pub fn outdated(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| {
                finding.placement == Placement::Outdated && self.outcome(&finding.id).is_none()
            })
            .count()
    }

    pub fn outcome_counts(&self) -> String {
        let count = |states: &[FindingState]| {
            self.findings
                .iter()
                .filter(|finding| states.contains(&self.state(&finding.id)))
                .count()
        };
        [
            (count(&[FindingState::Fixed]), "fixed"),
            (count(&[FindingState::Skipped]), "skipped"),
            (count(&[FindingState::NoChange]), "no change"),
            (
                count(&[FindingState::Open, FindingState::AgentOnIt]),
                "open",
            ),
        ]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, word)| format!("{n} {word}"))
        .collect::<Vec<_>>()
        .join(" · ")
    }

    pub fn pill_suffix(&self) -> Option<String> {
        let suffix = match self.status {
            RunStatus::Unreadable => "couldn’t read findings".to_string(),
            RunStatus::Stopped => "stopped".to_string(),
            RunStatus::Failed(_) => "failed".to_string(),
            RunStatus::Done if self.reported == 0 => "no issues".to_string(),
            RunStatus::Done if self.findings.is_empty() => format!("{} handled", self.reported),
            RunStatus::Done => {
                let mut text = self.outcome_counts();
                if self.outdated() > 0 {
                    text.push_str(&format!(" · {} outdated", self.outdated()));
                }
                text
            }
            _ if self.note.is_some() => "working".to_string(),
            _ => return None,
        };
        Some(match self.note {
            Some(_) => format!("{suffix} · fallback"),
            None => suffix,
        })
    }
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

pub fn reviewer_child<'a>(
    chats: &'a [Chat],
    owner: &str,
    task_id: Option<&str>,
) -> Option<&'a Chat> {
    chats
        .iter()
        .filter(|chat| {
            chat.delegation
                .as_ref()
                .is_some_and(|d| d.delegated_by_chat_id == owner && is_review(d))
        })
        .filter(|chat| task_id.is_none_or(|id| chat.id == id))
        .max_by_key(|chat| chat.delegation.as_ref().map(|d| d.started_at))
}

#[derive(Debug, Clone, PartialEq)]
pub struct FinishedRead {
    pub task_id: String,
    pub scope: ReviewScope,
    pub reviewed: Option<String>,
    pub commit: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StartedReview {
    pub task_id: String,
    pub patch: String,
    pub note: Option<String>,
}

pub fn start_params(
    owner: &str,
    scope: ReviewScope,
    commit: Option<&str>,
    choice: Option<&ReviewerChoice>,
) -> serde_json::Value {
    serde_json::json!({
        "ownerChatId": owner,
        "scope": scope.key(),
        "commitSha": commit,
        "reviewer": choice,
    })
}

pub fn diff_base(
    scope: ReviewScope,
    commit: Option<&str>,
    branches: &[String],
    current: Option<&str>,
) -> Result<Option<String>, String> {
    match scope {
        ReviewScope::Commit if commit.is_none() => Err("Pick a commit".to_string()),
        ReviewScope::Branch => default_base_ref(branches, current)
            .map(Some)
            .ok_or_else(|| "No base branch to compare against".to_string()),
        _ => Ok(None),
    }
}

pub fn checkout_diff_params(
    cwd: &str,
    chat_id: &str,
    scope: ReviewScope,
    commit: Option<&str>,
    base: Option<&str>,
) -> serde_json::Value {
    let mut params = serde_json::json!({
        "cwd": cwd,
        "mode": scope.mode(),
        "chatId": chat_id,
        "commitSha": commit,
    });
    if let Some(base) = base {
        params["baseRef"] = serde_json::Value::String(base.to_string());
    }
    params
}

pub fn hand_params(
    owner: &str,
    task_id: &str,
    ids: &[String],
    withdraw: bool,
) -> serde_json::Value {
    serde_json::json!({ "ownerChatId": owner, "taskId": task_id, "ids": ids, "withdraw": withdraw })
}

impl ChatReview {
    pub fn adopt(&mut self, chats: &[Chat], owner: &str) -> bool {
        let current = self.run.as_ref().map(|run| run.task_id.clone());
        let started_at = |chat: &Chat| chat.delegation.as_ref().map(|d| d.started_at);
        let Some(child) = reviewer_child(chats, owner, None) else {
            return false;
        };
        let adoptable = match &current {
            None => true,
            Some(None) => false,
            Some(Some(task)) => {
                task != &child.id
                    && reviewer_child(chats, owner, Some(task))
                        .is_none_or(|own| started_at(child) > started_at(own))
            }
        };
        let Some((scope, commit)) = child
            .delegation
            .as_ref()
            .and_then(|d| d.client_request_id.as_deref())
            .and_then(parse_request)
            .filter(|_| adoptable)
        else {
            return false;
        };
        let mut run = ReviewRun::new(scope);
        run.task_id = Some(child.id.clone());
        run.commit = commit.clone();
        run.status = RunStatus::Running;
        if current.is_some() {
            self.scope = Some(scope);
            if let Some(sha) = commit {
                self.commit = Some(CommitPick {
                    subject: sha.clone(),
                    sha,
                });
            }
        }
        self.run = Some(run);
        true
    }

    pub fn begin(&mut self, scope: ReviewScope, choice: ReviewerChoice) -> Option<ReviewRun> {
        let mut run = ReviewRun::new(scope);
        run.choice = Some(choice);
        run.commit = self
            .commit
            .as_ref()
            .filter(|_| scope == ReviewScope::Commit)
            .map(|pick| pick.sha.clone());
        self.run
            .replace(run)
            .filter(|run| run.status == RunStatus::Done && !run.findings.is_empty())
    }

    pub fn finished(&self, chats: &[Chat], owner: &str) -> Option<FinishedRead> {
        let run = self
            .run
            .as_ref()
            .filter(|run| run.status == RunStatus::Running)?;
        let task_id = run.task_id.clone()?;
        reviewer_child(chats, owner, Some(&task_id))
            .and_then(|child| child.delegation.as_ref())
            .filter(|d| !d.is_running())?;
        Some(FinishedRead {
            task_id,
            scope: run.scope,
            reviewed: run.reviewed.clone(),
            commit: run.commit.clone(),
        })
    }

    pub fn apply_stats(
        &mut self,
        scope: ReviewScope,
        result: Result<(CheckoutDiff, Option<String>), String>,
    ) {
        let (diff, base) = match result {
            Ok(fetched) => fetched,
            Err(message) => {
                self.stats.insert(scope, Err(message));
                return;
            }
        };
        self.stats
            .insert(scope, Ok((diff.additions, diff.deletions)));
        if base.is_some() {
            self.base = base;
        }
        if let Some(run) = self
            .run
            .as_mut()
            .filter(|run| run.scope == scope && run.status == RunStatus::Done)
        {
            refresh(&mut run.findings, &crate::patch::parse_patch(&diff.patch));
        }
    }

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
                        && !run.handed(id)
                })
                .cloned()
                .collect(),
        };
        for finding in &staged {
            run.withdrawn.remove(&finding.id);
            run.handed_here.insert(finding.id.clone());
        }
        staged.iter().map(Finding::comment).collect()
    }

    pub fn withdraw_comment(&mut self, comment_id: &str) {
        if let Some(finding) = comment_id.strip_prefix(STAGED_PREFIX)
            && let Some(run) = self.run.as_mut()
        {
            run.withdraw(finding);
        }
    }

    pub fn dismiss(&mut self, id: &str) {
        if let Some(run) = self.run.as_mut() {
            run.findings.retain(|finding| finding.id != id);
        }
    }

    pub fn start_label(&self, scope: ReviewScope) -> String {
        match (scope, &self.commit) {
            (ReviewScope::Commit, Some(pick)) => format!("Review commit {}", pick.short()),
            _ => format!(
                "Review {}",
                scope.label(self.base.as_deref()).to_lowercase()
            ),
        }
    }

    pub fn look(&self, scope: ReviewScope) -> Option<Look> {
        let counts = self.changes(scope).filter(|(add, del)| add + del > 0);
        let idle = || match counts {
            Some((add, del)) => Some(Look::Idle(add, del)),
            None => self.scope.is_some().then(|| {
                Look::Empty(match (self.changes(scope), self.unavailable(scope)) {
                    (Some(_), _) => "No changes".to_string(),
                    (None, Some(message)) => capitalize(message),
                    (None, None) => "…".to_string(),
                })
            }),
        };
        let Some(run) = &self.run else {
            return idle();
        };
        let open = run
            .findings
            .iter()
            .filter(|finding| run.outcome(&finding.id).is_none())
            .count();
        let look = match &run.status {
            status if run.busy() && *status != RunStatus::Done => Look::Busy,
            RunStatus::Unreadable => Look::Unreadable,
            RunStatus::Stopped => {
                let (add, del) = counts.unwrap_or_default();
                Look::Stopped(add, del)
            }
            RunStatus::Failed(message) => Look::Failed(message.clone()),
            _ if run.outdated() > 0 => {
                let (add, del) = counts.unwrap_or_default();
                Look::Again(add, del)
            }
            _ if open > 0 => Look::Findings(open),
            _ if run.reported > 0 || counts.is_none() => return idle(),
            _ => Look::Clean,
        };
        Some(look)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Look {
    Idle(u32, u32),
    Empty(String),
    Busy,
    Findings(usize),
    Again(u32, u32),
    Stopped(u32, u32),
    Clean,
    Unreadable,
    Failed(String),
}

impl Look {
    pub fn label(&self, scope: ReviewScope, picked: Option<&str>) -> String {
        match self {
            Self::Idle(..) | Self::Empty(_) => match (scope, picked) {
                (ReviewScope::Commit, Some(short)) => format!("Review {short}"),
                _ => "Review".to_string(),
            },
            Self::Busy => "Reviewing".to_string(),
            Self::Findings(n) => format!("{n} finding{}", if *n == 1 { "" } else { "s" }),
            Self::Again(..) | Self::Stopped(..) => "Review again".to_string(),
            Self::Clean => "No issues".to_string(),
            Self::Unreadable => "Couldn’t read findings".to_string(),
            Self::Failed(_) => "Review failed".to_string(),
        }
    }

    pub fn tip(&self, scope: ReviewScope) -> String {
        match self {
            Self::Idle(..) => format!(
                "Review {} with a fresh agent",
                scope.label(None).to_lowercase()
            ),
            Self::Empty(_) => format!(
                "Nothing to review in {}. Pick another scope from the menu.",
                scope.label(None).to_lowercase()
            ),
            Self::Busy => "The reviewer is working. Its progress is in the agents pill.".into(),
            Self::Findings(_) => "Open the findings".into(),
            Self::Again(..) => {
                "The code changed since the review. Some findings are outdated.".into()
            }
            Self::Stopped(..) => "The review was stopped. Click to run it again.".into(),
            Self::Clean => "The reviewer found no problems. Click to review again.".into(),
            Self::Unreadable => "The reviewer did not end with a findings block. Click to review again; its thread is in the agents pill.".into(),
            Self::Failed(message) => message.clone(),
        }
    }

    pub fn counts(&self) -> Option<(u32, u32)> {
        match self {
            Self::Idle(add, del) | Self::Again(add, del) | Self::Stopped(add, del) => {
                Some((*add, *del))
            }
            _ => None,
        }
    }
}

pub fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScopeRow {
    pub scope: ReviewScope,
    pub label: String,
    pub counts: Option<(u32, u32)>,
    pub note: Option<String>,
    pub empty: bool,
}

impl ChatReview {
    pub fn scope_rows(&self, branch: Option<&str>) -> Vec<ScopeRow> {
        let on_base = branch.is_some() && branch == self.base.as_deref();
        ReviewScope::ALL
            .into_iter()
            .filter(|scope| !(on_base && *scope == ReviewScope::Branch))
            .map(|scope| {
                let counts = self.changes(scope);
                let note = self.unavailable(scope).map(capitalize);
                ScopeRow {
                    scope,
                    label: scope.label(self.base.as_deref()),
                    empty: counts.is_some_and(|(add, del)| add + del == 0) || note.is_some(),
                    counts,
                    note,
                }
            })
            .collect()
    }

    pub fn effective_reviewer(&self, settings: Option<&ReviewSettings>) -> ReviewerChoice {
        self.reviewer
            .clone()
            .or_else(|| settings.map(|settings| settings.reviewer.clone()))
            .unwrap_or_default()
    }

    pub fn reviewer_changed(&self, effective: &ReviewerChoice) -> bool {
        self.run
            .as_ref()
            .and_then(|run| run.choice.as_ref())
            .is_some_and(|last| last != effective)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReviewerRow {
    pub choice: ReviewerChoice,
    pub harness: Option<HarnessId>,
    pub label: String,
    pub detail: Option<String>,
    pub selected: bool,
}

fn same_slot(a: &ReviewerChoice, b: &ReviewerChoice) -> bool {
    match (a, b) {
        (
            ReviewerChoice::Harness { harness: a, .. },
            ReviewerChoice::Harness { harness: b, .. },
        ) => a == b,
        _ => a == b,
    }
}

pub fn reviewer_rows(
    options: Option<&ReviewerOptions>,
    effective: &ReviewerChoice,
    chat: Option<HarnessId>,
    model_label: impl Fn(HarnessId, &str) -> String,
) -> Vec<ReviewerRow> {
    let fallback = ReviewerOptions::default();
    let all = options.unwrap_or(&fallback);
    let auto_name =
        options.map(|options| resolve_reviewer(&ReviewerChoice::Auto, chat, options).name());
    let mut choices = all.choices();
    if !choices.iter().any(|choice| same_slot(choice, effective)) {
        choices.insert(1, effective.clone());
    }
    choices
        .into_iter()
        .map(|choice| {
            let harness = match &choice {
                ReviewerChoice::Harness { harness, .. } => Some(*harness),
                _ => None,
            };
            let selected = same_slot(&choice, effective);
            let label = match harness {
                Some(harness) => all.label(&ReviewerChoice::Harness {
                    harness,
                    model: None,
                    reasoning: None,
                }),
                None => all.label(&choice),
            };
            let detail = match (&choice, effective) {
                (ReviewerChoice::Auto, _) => auto_name.clone(),
                (
                    _,
                    ReviewerChoice::Harness {
                        harness: h,
                        model,
                        reasoning,
                    },
                ) if selected
                    && harness == Some(*h)
                    && (model.is_some() || reasoning.is_some()) =>
                {
                    let parts: Vec<String> = model
                        .as_deref()
                        .map(|model| model_label(*h, model))
                        .into_iter()
                        .chain(reasoning.map(|level| level.label().to_string()))
                        .collect();
                    Some(parts.join(" · "))
                }
                _ => None,
            };
            ReviewerRow {
                choice,
                harness,
                label,
                detail,
                selected,
            }
        })
        .collect()
}

pub fn choice_agent(
    options: &ReviewerOptions,
    choice: &ReviewerChoice,
    chat: Option<HarnessId>,
) -> (Option<HarnessId>, Option<String>) {
    match choice {
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
    }
}

pub fn choice_name(
    options: Option<&ReviewerOptions>,
    choice: &ReviewerChoice,
    chat: Option<HarnessId>,
) -> String {
    match (choice, options) {
        (ReviewerChoice::Auto, Some(options)) => resolve_reviewer(choice, chat, options).name(),
        (_, Some(options)) => options.label(choice),
        (_, None) => ReviewerOptions::default().label(choice),
    }
}

pub fn commit_matches(subject: &str, sha: &str, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty() || subject.to_lowercase().contains(&query) || sha.starts_with(&query)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::parse_patch;

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

    fn profile(id: &str, name: &str, harness: HarnessId) -> AgentProfile {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "name": name,
            "harness": harness,
        }))
        .unwrap()
    }

    fn delegation() -> Delegation {
        serde_json::from_value(serde_json::json!({
            "delegatedByChatId": "owner",
            "depth": 1,
            "anchor": "task",
            "clientRequestId": "zeron-review:uncommitted:n",
            "wake": "never",
            "status": "completed",
            "startedAt": "2026-10-17T10:00:00Z",
        }))
        .unwrap()
    }

    fn outcome(kind: FindingOutcomeKind) -> FindingOutcome {
        FindingOutcome {
            outcome: kind,
            note: Some("why".into()),
            commit: None,
        }
    }

    #[test]
    fn parses_the_last_block_and_an_empty_array_means_clean() {
        let text = format!(
            "{}\nSecond pass:\n{}",
            reply("[{\"path\":\"a\",\"line\":1,\"severity\":\"nit\",\"title\":\"x\"}]"),
            reply("[]")
        );
        assert_eq!(parse_findings(&text), Some(Vec::new()));
        let found = parse_findings(&reply(
            "[{\"path\":\"./src/a.rs\",\"side\":\"L\",\"line\":3,\"severity\":\"bug\",\"title\":\"t\",\"body\":\"b\"}]",
        ))
        .unwrap()
        .remove(0);
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
    fn findings_are_numbered_from_one_in_reply_order() {
        let found = parse_findings(&reply(
            "[{\"path\":\"a\",\"line\":1,\"severity\":\"bug\",\"title\":\"first\"},\
             {\"path\":\"b\",\"line\":2,\"severity\":\"high\",\"title\":\"dropped\"},\
             {\"path\":\"c\",\"line\":3,\"severity\":\"nit\",\"title\":\"third\"},\
             {\"path\":\"d\",\"line\":4,\"severity\":\"nit\"},\
             {\"path\":\"e\",\"line\":5,\"severity\":\"nit\",\"title\":\"fifth\"}]",
        ))
        .unwrap();
        let ids: Vec<(&str, &str)> = found
            .iter()
            .map(|f| (f.id.as_str(), f.title.as_str()))
            .collect();
        assert_eq!(ids, [("F1", "first"), ("F3", "third"), ("F5", "fifth")]);
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
    fn the_request_id_carries_the_scope_and_commit() {
        let id = request_id(ReviewScope::Branch, None, "abc");
        assert!(id.starts_with(REQUEST_PREFIX));
        assert_eq!(parse_request(&id), Some((ReviewScope::Branch, None)));
        let id = request_id(ReviewScope::Commit, Some("a1b2c3d"), "abc");
        assert_eq!(
            parse_request(&id),
            Some((ReviewScope::Commit, Some("a1b2c3d".to_string())))
        );
        assert_eq!(parse_request("other:branch:x"), None);
        let forged = request_id(ReviewScope::Commit, Some("abc123é"), "abc");
        assert_eq!(parse_request(&forged), Some((ReviewScope::Commit, None)));
    }

    #[test]
    fn saved_review_settings_round_trip_and_default_to_auto() {
        let settings: ReviewSettings = serde_json::from_value(serde_json::json!({
            "reviewer": {"kind": "harness", "harness": "claude-code", "reasoning": "high"},
            "instructions": "Be terse."
        }))
        .unwrap();
        assert_eq!(settings.instructions, "Be terse.");
        let back: ReviewSettings =
            serde_json::from_value(serde_json::to_value(&settings).unwrap()).unwrap();
        assert_eq!(back, settings);
        assert_eq!(
            serde_json::from_value::<ReviewSettings>(serde_json::json!({})).unwrap(),
            ReviewSettings::default()
        );
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
        assert_eq!(findings[0].meta(), "F1 · src/session.rs:42 · outdated");
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
        let mut delegation = delegation();
        delegation.finding_outcomes.insert(
            "F1".into(),
            FindingOutcome {
                commit: Some("a1b2c3d4e5".into()),
                ..outcome(FindingOutcomeKind::Fixed)
            },
        );
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
        assert_eq!(run.state_label("F1"), "Fixed · a1b2c3d");
        assert_eq!(run.reason("F1"), None);
        assert_eq!(run.reason("F2").as_deref(), Some("why"));
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
        assert!(
            review.take_fixable(Some("F3")).is_empty(),
            "a second Fix never stages a handed row again"
        );
        assert_eq!(staged[0].id, format!("{STAGED_PREFIX}F3"));
        let run = review.run.as_mut().unwrap();
        run.delegation
            .as_mut()
            .unwrap()
            .findings_handed
            .insert("F3".into());
        run.withdraw("F3");
        assert_eq!(run.unsent_withdrawals(), ["F3"]);
        assert_eq!(
            review.take_fixable(Some("F3")).len(),
            1,
            "removing the staged comment hands the row back"
        );
        assert!(review.run.as_ref().unwrap().unsent_withdrawals().is_empty());
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

    #[test]
    fn a_picked_scope_with_nothing_to_review_keeps_the_chip() {
        let turn = ReviewScope::LatestTurn;
        let picked = |stat: Option<Result<(u32, u32), String>>| ChatReview {
            scope: Some(turn),
            stats: stat.into_iter().map(|stat| (turn, stat)).collect(),
            ..ChatReview::default()
        };
        for review in [
            picked(Some(Ok((0, 0)))),
            picked(Some(Err("no turn recorded".into()))),
            picked(None),
        ] {
            assert!(matches!(review.look(turn), Some(Look::Empty(_))));
        }
        let mut clean = picked(Some(Ok((0, 0))));
        let mut run = ReviewRun::new(turn);
        run.status = RunStatus::Done;
        clean.run = Some(run);
        assert!(matches!(clean.look(turn), Some(Look::Empty(_))));

        let untouched = ChatReview {
            stats: [(ReviewScope::Uncommitted, Ok((0, 0)))].into(),
            ..ChatReview::default()
        };
        assert!(untouched.look(ReviewScope::Uncommitted).is_none());
    }

    #[test]
    fn the_pill_counts_outcomes_and_leaves_out_the_empty_ones() {
        let reply = (1..=5)
            .map(|n| {
                format!("{{\"path\":\"a\",\"line\":{n},\"severity\":\"bug\",\"title\":\"t\"}}")
            })
            .collect::<Vec<_>>()
            .join(",");
        let ReadOutcome::Read(Some(findings)) =
            read_outcome("completed", &format!("```zeron-review\n[{reply}]\n```"))
        else {
            panic!("unread");
        };
        let mut delegation = delegation();
        for (id, kind) in [
            ("F1", FindingOutcomeKind::Fixed),
            ("F2", FindingOutcomeKind::Fixed),
            ("F3", FindingOutcomeKind::Skipped),
            ("F4", FindingOutcomeKind::NoChange),
        ] {
            delegation.finding_outcomes.insert(
                id.into(),
                FindingOutcome {
                    outcome: kind,
                    note: None,
                    commit: None,
                },
            );
        }
        let mut run = ReviewRun::new(ReviewScope::Uncommitted);
        run.status = RunStatus::Done;
        run.reported = 5;
        run.findings = findings;
        run.delegation = Some(delegation.clone());
        assert_eq!(
            run.outcome_counts(),
            "2 fixed · 1 skipped · 1 no change · 1 open"
        );
        assert_eq!(
            run.pill_suffix().as_deref(),
            Some("2 fixed · 1 skipped · 1 no change · 1 open")
        );
        delegation.finding_outcomes.remove("F3");
        delegation.finding_outcomes.remove("F4");
        run.delegation = Some(delegation);
        assert_eq!(run.outcome_counts(), "2 fixed · 3 open");
    }

    #[test]
    fn scope_rows_drop_branch_on_the_base_and_flag_empty_scopes() {
        let mut review = ChatReview {
            base: Some("main".into()),
            ..ChatReview::default()
        };
        review.stats.insert(ReviewScope::Uncommitted, Ok((4, 1)));
        review
            .stats
            .insert(ReviewScope::LatestTurn, Err("no turn yet".into()));
        let scopes = |rows: Vec<ScopeRow>| rows.iter().map(|row| row.scope).collect::<Vec<_>>();
        assert!(!scopes(review.scope_rows(Some("main"))).contains(&ReviewScope::Branch));
        let rows = review.scope_rows(Some("fix/colors"));
        assert_eq!(scopes(rows.clone()), ReviewScope::ALL.to_vec());
        let row = |scope| rows.iter().find(|row| row.scope == scope).unwrap();
        assert!(!row(ReviewScope::Uncommitted).empty);
        assert_eq!(
            row(ReviewScope::LatestTurn).note.as_deref(),
            Some("No turn yet")
        );
        assert!(row(ReviewScope::LatestTurn).empty);
        assert_eq!(row(ReviewScope::Branch).counts, None);
        assert!(!row(ReviewScope::Branch).empty);
        assert_eq!(row(ReviewScope::Branch).label, "Branch vs main");
    }

    #[test]
    fn reviewer_rows_mark_the_effective_slot_and_name_auto() {
        let options = ReviewerOptions {
            offered: vec![HarnessId::ClaudeCode, HarnessId::Codex],
            ..ReviewerOptions::default()
        };
        let effective = ReviewerChoice::Harness {
            harness: HarnessId::ClaudeCode,
            model: Some("opus".into()),
            reasoning: Some(ReasoningLevel::High),
        };
        let rows = reviewer_rows(
            Some(&options),
            &effective,
            Some(HarnessId::Codex),
            |_, m| m.to_uppercase(),
        );
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].detail.as_deref(), Some("Claude Code"));
        let claude = &rows[1];
        assert!(claude.selected && claude.harness == Some(HarnessId::ClaudeCode));
        assert_eq!(claude.label, "Claude Code");
        assert_eq!(
            claude.detail,
            Some(format!("OPUS · {}", ReasoningLevel::High.label()))
        );
        assert!(!rows[2].selected && rows[2].detail.is_none());

        let missing = ReviewerChoice::Profile { id: "gone".into() };
        let rows = reviewer_rows(Some(&options), &missing, None, |_, m| m.into());
        assert_eq!(rows[1].choice, missing);
        assert!(rows[1].selected);
        assert_eq!(rows[1].label, "Missing profile");
    }

    #[test]
    fn commit_matches_subject_words_or_sha_prefix() {
        assert!(commit_matches("Fix the tab strip", "4e1c9a7", ""));
        assert!(commit_matches("Fix the tab strip", "4e1c9a7", " TAB "));
        assert!(commit_matches("Fix the tab strip", "4e1c9a7", "4e1c"));
        assert!(!commit_matches("Fix the tab strip", "4e1c9a7", "c9a7"));
    }
}
