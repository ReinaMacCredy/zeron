use zeron_client as zc;
use zeron_proto::comments::ReviewComment;
use zeron_proto::review::{
    self, ChipAction, FindingState, Look, ReviewScope, ReviewerChoice, Severity,
};
use zeron_proto::{HarnessId, ReasoningLevel};

use super::git::{DiffFileView, diff_files};
use super::{CoreError, CoreResult, SessionHandle, on_runtime};

fn wire<T: serde::Serialize>(value: T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn parse<T: serde::de::DeserializeOwned>(text: &str) -> CoreResult<T> {
    serde_json::from_value(serde_json::Value::String(text.to_owned())).map_err(|_| {
        CoreError::InvalidArgument {
            message: format!("unknown value {text}"),
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ReviewTone {
    Plain,
    Muted,
    Danger,
    Success,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ReviewChipAction {
    OpenFindings,
    Start,
    Inert,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ReviewSeverity {
    Bug,
    Risk,
    Nit,
}

impl From<Severity> for ReviewSeverity {
    fn from(severity: Severity) -> Self {
        match severity {
            Severity::Bug => Self::Bug,
            Severity::Risk => Self::Risk,
            Severity::Nit => Self::Nit,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ReviewFindingState {
    Open,
    AgentOnIt,
    Fixed,
    Skipped,
    NoChange,
}

impl From<FindingState> for ReviewFindingState {
    fn from(state: FindingState) -> Self {
        match state {
            FindingState::Open => Self::Open,
            FindingState::AgentOnIt => Self::AgentOnIt,
            FindingState::Fixed => Self::Fixed,
            FindingState::Skipped => Self::Skipped,
            FindingState::NoChange => Self::NoChange,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ReviewChipView {
    pub label: String,
    pub tip: String,
    pub tone: ReviewTone,
    pub filled: bool,
    pub dashed: bool,
    pub busy: bool,
    pub stoppable: bool,
    pub stop_label: String,
    pub stop_tip: String,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
    pub note: Option<String>,
    pub next_reviewer: Option<String>,
    pub next_brand: Option<String>,
    pub action: ReviewChipAction,
    pub refresh: bool,
    pub clean: bool,
    pub menu_label: String,
}

impl From<&zc::session::ReviewChip> for ReviewChipView {
    fn from(chip: &zc::session::ReviewChip) -> Self {
        let look = &chip.look;
        let tone = match look {
            Look::Idle(..) => ReviewTone::Plain,
            Look::Findings(_) | Look::Failed(_) => ReviewTone::Danger,
            Look::Clean => ReviewTone::Success,
            Look::Again(..)
            | Look::Stopped(..)
            | Look::Empty(_)
            | Look::Busy
            | Look::Unreadable => ReviewTone::Muted,
        };
        let counts = look.counts();
        Self {
            label: look.label(chip.scope, chip.picked.as_deref()),
            tip: look.tip(chip.scope),
            tone,
            filled: matches!(look, Look::Findings(_)),
            dashed: matches!(look, Look::Again(..) | Look::Stopped(..)),
            busy: matches!(look, Look::Busy),
            stoppable: chip.stoppable,
            stop_label: review::STOP_LABEL.into(),
            stop_tip: review::STOP_TIP.into(),
            additions: counts.map(|(add, _)| add),
            deletions: counts.map(|(_, del)| del),
            note: match look {
                Look::Empty(note) => Some(note.clone()),
                _ => None,
            },
            next_reviewer: chip.next_reviewer.as_ref().map(|next| next.name.clone()),
            next_brand: chip
                .next_reviewer
                .as_ref()
                .and_then(|next| next.brand)
                .map(wire),
            action: match look.action() {
                ChipAction::OpenFindings => ReviewChipAction::OpenFindings,
                ChipAction::Start => ReviewChipAction::Start,
                ChipAction::Inert => ReviewChipAction::Inert,
            },
            refresh: matches!(look, Look::Again(..) | Look::Stopped(..)),
            clean: matches!(look, Look::Clean),
            menu_label: review::REVIEW_CHANGES.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct StagedCommentChip {
    pub id: String,
    pub severity: Option<ReviewSeverity>,
    pub text: String,
    pub location: String,
}

pub(super) fn staged_chips(comments: &[ReviewComment]) -> Vec<StagedCommentChip> {
    comments
        .iter()
        .map(|comment| {
            let line = review::staged_line(comment);
            StagedCommentChip {
                id: comment.id.clone(),
                severity: line.severity.map(Into::into),
                text: line.text,
                location: line.location,
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct StagedCommentsLabels {
    pub title: String,
    pub clear: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DiffFindingCard {
    pub id: String,
    pub severity: ReviewSeverity,
    pub severity_word: String,
    pub title: String,
    pub detail: String,
    pub fix_label: String,
    pub dismiss_label: String,
}

impl DiffFindingCard {
    pub(super) fn of(comment: &ReviewComment) -> Option<Self> {
        let id = comment.id.strip_prefix(review::CARD_PREFIX)?;
        let (severity, title, detail) = review::split_finding(&comment.body);
        let severity = severity?;
        Some(Self {
            id: id.to_owned(),
            severity: severity.into(),
            severity_word: severity.word().into(),
            title: title.to_owned(),
            detail: detail.to_owned(),
            fix_label: review::FIX.into(),
            dismiss_label: review::DISMISS.into(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ChatChangesView {
    pub title: String,
    pub files: String,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ReviewFindingRow {
    pub id: String,
    pub severity: ReviewSeverity,
    pub severity_word: String,
    pub title: String,
    pub reason: Option<String>,
    pub meta: String,
    pub state: ReviewFindingState,
    pub state_label: String,
    pub dimmed: bool,
    pub dismissable: bool,
    pub dismiss_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ReviewFindingsView {
    pub task_id: String,
    pub summary: Option<String>,
    pub note: Option<String>,
    pub retry_error: Option<String>,
    pub rows: Vec<ReviewFindingRow>,
    pub fix_label: Option<String>,
    pub open_changes: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum ReviewerPick {
    Auto,
    Harness {
        harness: String,
        model: Option<String>,
        reasoning: Option<String>,
    },
    Profile {
        id: String,
    },
}

impl From<&ReviewerChoice> for ReviewerPick {
    fn from(choice: &ReviewerChoice) -> Self {
        match choice {
            ReviewerChoice::Auto => Self::Auto,
            ReviewerChoice::Harness {
                harness,
                model,
                reasoning,
            } => Self::Harness {
                harness: wire(harness),
                model: model.clone(),
                reasoning: reasoning.map(wire),
            },
            ReviewerChoice::Profile { id } => Self::Profile { id: id.clone() },
        }
    }
}

impl TryFrom<ReviewerPick> for ReviewerChoice {
    type Error = CoreError;

    fn try_from(pick: ReviewerPick) -> CoreResult<Self> {
        Ok(match pick {
            ReviewerPick::Auto => Self::Auto,
            ReviewerPick::Harness {
                harness,
                model,
                reasoning,
            } => Self::Harness {
                harness: parse(&harness)?,
                model,
                reasoning: reasoning.as_deref().map(parse).transpose()?,
            },
            ReviewerPick::Profile { id } => Self::Profile { id },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ReviewScopeRow {
    pub key: String,
    pub label: String,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
    pub note: Option<String>,
    pub selected: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ReviewerRowView {
    pub pick: ReviewerPick,
    pub label: String,
    pub detail: Option<String>,
    pub selected: bool,
    pub brand: Option<String>,
    pub models_harness: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ReviewOptionsView {
    pub title: String,
    pub scope_heading: String,
    pub scopes: Vec<ReviewScopeRow>,
    pub commit_label: String,
    pub commit_picked: Option<String>,
    pub commit_selected: bool,
    pub reviewer_heading: String,
    pub reviewers: Vec<ReviewerRowView>,
    pub start_label: String,
    pub busy: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ReviewCommitRow {
    pub sha: String,
    pub subject: String,
    pub detail: String,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ReviewCommitsView {
    pub loading: bool,
    pub rows: Vec<ReviewCommitRow>,
    pub empty: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ReviewerOptionRow {
    pub label: String,
    pub pick: ReviewerPick,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ReviewerModelsView {
    pub title: String,
    pub loading: bool,
    pub models: Vec<ReviewerOptionRow>,
    pub thinking_heading: String,
    pub thinking: Vec<ReviewerOptionRow>,
}

fn model_label(snapshot: &zc::session::ReviewSnapshot, harness: HarnessId, id: &str) -> String {
    snapshot
        .models
        .get(&harness)
        .and_then(|models| models.iter().find(|m| m.id == id))
        .map_or_else(|| id.to_owned(), |m| m.label.clone())
}

#[uniffi::export]
impl SessionHandle {
    pub fn review_findings(&self, task_id: Option<String>) -> Option<ReviewFindingsView> {
        let snapshot = self.inner.review_snapshot()?;
        let run = snapshot
            .review
            .run
            .as_ref()
            .filter(|run| task_id.is_none() || run.task_id.as_deref() == task_id.as_deref())?;
        let listed = run.findings_listed();
        if !listed && run.note.is_none() {
            return None;
        }
        let fixable = run.fixable().count();
        Some(ReviewFindingsView {
            task_id: run.task_id.clone().unwrap_or_default(),
            summary: run.pill_suffix(),
            note: run.note.clone(),
            retry_error: run.retry_error.as_deref().map(review::retry_failed),
            rows: if listed {
                run.findings
                    .iter()
                    .map(|finding| {
                        let state = run.state(&finding.id);
                        ReviewFindingRow {
                            id: finding.id.clone(),
                            severity: finding.severity.into(),
                            severity_word: finding.severity.word().into(),
                            title: finding.title.clone(),
                            reason: run.reason(&finding.id),
                            meta: finding.meta(),
                            state: state.into(),
                            state_label: run.state_label(&finding.id),
                            dimmed: finding.placement == review::Placement::Outdated
                                || state.settled(),
                            dismissable: state == FindingState::Open,
                            dismiss_label: review::DISMISS.into(),
                        }
                    })
                    .collect()
            } else {
                Vec::new()
            },
            fix_label: (listed && fixable > 0).then(|| review::fix_remaining(fixable)),
            open_changes: review::OPEN_CHANGES.into(),
        })
    }

    pub fn review_options(&self) -> Option<ReviewOptionsView> {
        let snapshot = self.inner.review_snapshot()?;
        let review = &snapshot.review;
        let selected = snapshot.scope;
        let options = snapshot.options.clone().unwrap_or_default();
        let reviewers = review::reviewer_rows(
            snapshot.options.as_ref(),
            &snapshot.effective,
            snapshot.harness,
            |harness, model| model_label(&snapshot, harness, model),
        )
        .into_iter()
        .map(|row| {
            let (harness, model) = review::choice_agent(&options, &row.choice, snapshot.harness);
            ReviewerRowView {
                pick: (&row.choice).into(),
                label: row.label,
                detail: row.detail,
                selected: row.selected,
                brand: HarnessId::brand_of(harness, model.as_deref()).map(wire),
                models_harness: row.harness.map(wire),
            }
        })
        .collect();
        Some(ReviewOptionsView {
            title: review::REVIEW_OPTIONS.into(),
            scope_heading: review::SCOPE_HEADING.into(),
            scopes: review
                .scope_rows(snapshot.branch.as_deref())
                .into_iter()
                .map(|row| ReviewScopeRow {
                    key: row.scope.key().into(),
                    label: row.label,
                    additions: row.counts.map(|(add, _)| add),
                    deletions: row.counts.map(|(_, del)| del),
                    enabled: row.note.is_none(),
                    note: row.note,
                    selected: row.scope == selected,
                })
                .collect(),
            commit_label: review::COMMIT_LABEL.into(),
            commit_picked: review.commit.as_ref().map(|pick| pick.short().to_owned()),
            commit_selected: selected == ReviewScope::Commit,
            reviewer_heading: review::REVIEWER_HEADING.into(),
            reviewers,
            start_label: review.start_label(selected),
            busy: review.run.as_ref().is_some_and(review::ReviewRun::busy),
        })
    }

    pub fn open_review_options(&self) -> CoreResult<()> {
        self.inner.open_review_options().map_err(Into::into)
    }

    pub fn set_review_scope(&self, key: String) -> CoreResult<()> {
        let scope = ReviewScope::from_key(&key).ok_or_else(|| CoreError::InvalidArgument {
            message: format!("unknown review scope {key}"),
        })?;
        self.inner.set_review_scope(scope);
        Ok(())
    }

    pub fn set_reviewer(&self, pick: ReviewerPick) -> CoreResult<()> {
        self.inner.set_reviewer(pick.try_into()?);
        Ok(())
    }

    pub fn load_review_commits(&self) -> CoreResult<()> {
        self.inner.load_review_commits().map_err(Into::into)
    }

    pub fn review_commits(&self, query: String) -> ReviewCommitsView {
        let snapshot = self.inner.review_snapshot();
        let current = snapshot
            .as_ref()
            .and_then(|s| s.review.commit.as_ref().map(|pick| pick.sha.clone()));
        let commits = snapshot.and_then(|s| s.commits);
        ReviewCommitsView {
            loading: commits.is_none(),
            rows: commits
                .unwrap_or_default()
                .into_iter()
                .filter(|commit| review::commit_matches(&commit.subject, &commit.sha, &query))
                .map(|commit| ReviewCommitRow {
                    detail: format!(
                        "{} · {}",
                        review::short_sha(&commit.sha),
                        super::git::ago(&commit.authored_at)
                    ),
                    selected: current.as_deref() == Some(commit.sha.as_str()),
                    sha: commit.sha,
                    subject: commit.subject,
                })
                .collect(),
            empty: review::NO_MATCHING_COMMITS.into(),
        }
    }

    pub fn pick_review_commit(&self, sha: String, subject: String) -> CoreResult<()> {
        self.inner
            .pick_review_commit(&sha, &subject)
            .map_err(Into::into)
    }

    pub fn load_reviewer_models(&self, harness: String) -> CoreResult<()> {
        self.inner
            .load_reviewer_models(parse(&harness)?)
            .map_err(Into::into)
    }

    pub fn reviewer_models(&self, harness: String) -> CoreResult<ReviewerModelsView> {
        let id: HarnessId = parse(&harness)?;
        let snapshot = self
            .inner
            .review_snapshot()
            .ok_or_else(|| CoreError::InvalidArgument {
                message: "no review here".into(),
            })?;
        let (current, reasoning) = match &snapshot.effective {
            ReviewerChoice::Harness {
                harness: h,
                model,
                reasoning,
            } if *h == id => (Some(model.clone()), *reasoning),
            _ => (None, None),
        };
        let pick = |model: Option<String>, reasoning: Option<ReasoningLevel>| {
            ReviewerPick::from(&ReviewerChoice::Harness {
                harness: id,
                model,
                reasoning,
            })
        };
        let models = snapshot.models.get(&id);
        let model_rows = std::iter::once(("Default".to_owned(), None))
            .chain(
                models
                    .into_iter()
                    .flatten()
                    .map(|m| (m.label.clone(), Some(m.id.clone()))),
            )
            .map(|(label, model)| ReviewerOptionRow {
                selected: current.as_ref() == Some(&model),
                pick: pick(model, reasoning),
                label,
            })
            .collect();
        let model = current.clone().flatten();
        let levels = model
            .as_deref()
            .and_then(|m| models.and_then(|list| list.iter().find(|entry| entry.id == m)))
            .map(zc::session::model_levels);
        let stops = snapshot
            .options
            .as_ref()
            .map(|options| options.reasoning_stops(id, levels.as_deref()))
            .unwrap_or_default();
        let thinking = if stops.is_empty() {
            Vec::new()
        } else {
            std::iter::once(("Default".to_owned(), None))
                .chain(
                    stops
                        .iter()
                        .map(|level| (level.label().to_owned(), Some(*level))),
                )
                .map(|(label, level)| ReviewerOptionRow {
                    selected: reasoning == level,
                    pick: pick(model.clone(), level),
                    label,
                })
                .collect()
        };
        Ok(ReviewerModelsView {
            title: options_label(&snapshot, id),
            loading: models.is_none(),
            models: model_rows,
            thinking_heading: review::THINKING_HEADING.into(),
            thinking,
        })
    }

    pub async fn start_review(&self) -> CoreResult<()> {
        let handle = self.inner.clone();
        let scope = handle
            .review_snapshot()
            .map(|snapshot| snapshot.scope)
            .ok_or_else(|| CoreError::InvalidArgument {
                message: "no review here".into(),
            })?;
        on_runtime(async move { handle.start_review(scope).await }).await
    }

    pub async fn stop_review(&self) -> CoreResult<()> {
        let handle = self.inner.clone();
        on_runtime(async move { handle.stop_review().await }).await
    }

    pub async fn review_changes(&self) -> CoreResult<Vec<DiffFileView>> {
        let handle = self.inner.clone();
        let patch = on_runtime(async move { handle.review_changes().await }).await?;
        let cards = self
            .inner
            .review_snapshot()
            .map(|snapshot| snapshot.review.cards())
            .unwrap_or_default();
        Ok(diff_files(&patch, &[], &cards))
    }

    pub async fn chat_changes(&self) -> CoreResult<ChatChangesView> {
        let files = self.review_changes().await?;
        Ok(ChatChangesView {
            title: zeron_proto::badges::CHANGES_TITLE.into(),
            files: zeron_proto::badges::files_label(files.len()),
            additions: files.iter().map(|file| file.additions).sum(),
            deletions: files.iter().map(|file| file.deletions).sum(),
        })
    }

    pub fn fix_review_findings(&self, only: Option<String>) -> u32 {
        self.inner.fix_review_findings(only.as_deref()) as u32
    }

    pub fn dismiss_review_finding(&self, id: String) {
        self.inner.dismiss_review_finding(&id);
    }

    pub fn remove_staged_comment(&self, id: String) {
        self.inner.remove_staged_comment(&id);
    }

    pub fn clear_staged_comments(&self) {
        for comment in self.inner.composer().staged_comments.iter() {
            self.inner.remove_staged_comment(&comment.id);
        }
    }

    pub fn staged_comments_labels(&self) -> StagedCommentsLabels {
        StagedCommentsLabels {
            title: zeron_proto::badges::comment_label(self.inner.composer().staged_comments.len()),
            clear: review::CLEAR_COMMENTS.into(),
        }
    }
}

fn options_label(snapshot: &zc::session::ReviewSnapshot, harness: HarnessId) -> String {
    snapshot
        .options
        .clone()
        .unwrap_or_default()
        .label(&ReviewerChoice::Harness {
            harness,
            model: None,
            reasoning: None,
        })
}
