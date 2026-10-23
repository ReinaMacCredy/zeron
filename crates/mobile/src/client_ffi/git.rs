use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use zeron_client as zc;
use zeron_proto as zp;
use zp::change_request_assessment::{CiState, Facts};
use zp::pull_request::{actions, board, chat::PrChatContext, detail, handoff};
use zp::{git_graph, git_tab, patch};

use super::{CoreClient, CoreError, CoreResult, on_runtime};

const DIFF_LINE_LIMIT: usize = 600;
const HISTORY_PAGE: usize = 60;

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PrTone {
    Muted,
    Positive,
    Warning,
    Danger,
    Merged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PrGlyph {
    PullRequest,
    CloseCircle,
    Document,
    Check,
    DangerTriangle,
    ClockCircle,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PrChip {
    pub label: String,
    pub glyph: PrGlyph,
    pub tone: PrTone,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PrNotice {
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DiffLineKind {
    Context,
    Add,
    Del,
    Meta,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DiffLineView {
    pub kind: DiffLineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub text: String,
    pub threads: Vec<PrThread>,
    pub findings: Vec<super::review::DiffFindingCard>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DiffHunkView {
    pub header: String,
    pub lines: Vec<DiffLineView>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DiffFileView {
    pub path: String,
    pub old_path: Option<String>,
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
    pub binary: bool,
    pub notices: Vec<String>,
    pub hunks: Vec<DiffHunkView>,
}

fn chip((label, glyph, tone): (String, detail::StatusGlyph, detail::StatusTone)) -> PrChip {
    PrChip {
        label,
        glyph: match glyph {
            detail::StatusGlyph::PullRequest => PrGlyph::PullRequest,
            detail::StatusGlyph::CloseCircle => PrGlyph::CloseCircle,
            detail::StatusGlyph::Document => PrGlyph::Document,
            detail::StatusGlyph::Check => PrGlyph::Check,
            detail::StatusGlyph::DangerTriangle => PrGlyph::DangerTriangle,
            detail::StatusGlyph::ClockCircle => PrGlyph::ClockCircle,
        },
        tone: match tone {
            detail::StatusTone::Neutral => PrTone::Muted,
            detail::StatusTone::Positive => PrTone::Positive,
            detail::StatusTone::Warning => PrTone::Warning,
            detail::StatusTone::Negative => PrTone::Danger,
            detail::StatusTone::Merged => PrTone::Merged,
        },
    }
}

fn board_tone(tone: board::Tone) -> PrTone {
    match tone {
        board::Tone::Muted => PrTone::Muted,
        board::Tone::Danger => PrTone::Danger,
        board::Tone::Warning => PrTone::Warning,
        board::Tone::Success => PrTone::Positive,
    }
}

fn ci_tone(state: CiState) -> PrTone {
    match state {
        CiState::Failed => PrTone::Danger,
        CiState::Pending => PrTone::Warning,
        CiState::Passed => PrTone::Positive,
        CiState::NoChecks | CiState::Skipped | CiState::Unknown => PrTone::Muted,
    }
}

fn changes(additions: u64, deletions: u64) -> String {
    format!(
        "+{} \u{2212}{}",
        board::format_compact_count(additions),
        board::format_compact_count(deletions)
    )
}

pub(super) fn ago(stamp: &str) -> String {
    DateTime::parse_from_rfc3339(stamp)
        .map(|at| board::compact_relative_time(at.with_timezone(&Utc), Utc::now()))
        .unwrap_or_default()
}

pub(super) fn diff_files(
    text: &str,
    threads: &[zp::ChangeRequestReviewThread],
    cards: &[zp::comments::ReviewComment],
) -> Vec<DiffFileView> {
    patch::parse_patch(text)
        .into_iter()
        .map(|mut file| {
            patch::truncate_file_lines(&mut file, DIFF_LINE_LIMIT);
            let anchored = |line: &patch::DiffLine| -> Vec<PrThread> {
                threads
                    .iter()
                    .filter(|thread| thread.path == file.path && !thread.is_outdated)
                    .filter(|thread| {
                        let at = thread.line.or(thread.original_line);
                        match thread.diff_side.as_str() {
                            "LEFT" => at.is_some_and(|at| line.old_no == Some(at as u32)),
                            _ => at.is_some_and(|at| line.new_no == Some(at as u32)),
                        }
                    })
                    .map(thread_view)
                    .collect()
            };
            let placed = |line: &patch::DiffLine| -> Vec<super::review::DiffFindingCard> {
                let anchor = patch::line_anchor(line);
                cards
                    .iter()
                    .filter(|card| card.path == file.path && anchor.is_some())
                    .filter(|card| card.diff_anchor() == anchor)
                    .filter_map(super::review::DiffFindingCard::of)
                    .collect()
            };
            DiffFileView {
                status: match file.status {
                    patch::FileStatus::Added => "Added",
                    patch::FileStatus::Deleted => "Deleted",
                    patch::FileStatus::Modified => "Modified",
                    patch::FileStatus::Renamed => "Renamed",
                }
                .into(),
                notices: patch::file_notices(&file),
                hunks: file
                    .hunks
                    .iter()
                    .map(|hunk| DiffHunkView {
                        header: hunk.header.clone(),
                        lines: hunk
                            .lines
                            .iter()
                            .map(|line| DiffLineView {
                                kind: match line.kind {
                                    patch::LineKind::Context => DiffLineKind::Context,
                                    patch::LineKind::Add => DiffLineKind::Add,
                                    patch::LineKind::Del => DiffLineKind::Del,
                                    patch::LineKind::Meta => DiffLineKind::Meta,
                                },
                                old_no: line.old_no,
                                new_no: line.new_no,
                                text: line.text.clone(),
                                threads: anchored(line),
                                findings: placed(line),
                            })
                            .collect(),
                    })
                    .collect(),
                path: file.path.clone(),
                old_path: file.old_path.clone(),
                additions: file.additions,
                deletions: file.deletions,
                binary: file.binary,
            }
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PrFilter {
    Authored,
    Reviewing,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PrPersonalView {
    Everything,
    NeedsYou,
    Starred,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PrSortField {
    Updated,
    Opened,
    Changes,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PrDevice {
    pub id: String,
    pub name: String,
    pub online: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PrRepository {
    pub repository: String,
    pub project_id: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PrRow {
    pub url: String,
    pub star_key: String,
    pub starred: bool,
    pub number: u64,
    pub title: String,
    pub author: String,
    pub status: String,
    pub status_tone: PrTone,
    pub glyph_tone: PrTone,
    pub ci_label: String,
    pub ci_tone: PrTone,
    pub changes: String,
    pub updated: String,
    pub accessibility: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PrGroup {
    pub key: String,
    pub label: String,
    pub rows: Vec<PrRow>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PrBoardView {
    pub device_id: String,
    pub repository: String,
    pub filter: PrFilter,
    pub view: PrPersonalView,
    pub sort_field: PrSortField,
    pub ascending: bool,
    pub query: String,
    pub groups: Vec<PrGroup>,
    pub empty: Option<String>,
    pub error: Option<PrNotice>,
    pub can_load_more: bool,
    pub summary: String,
}

struct BoardState {
    repository: String,
    filter: zp::ChangeRequestFilter,
    view: board::PersonalView,
    sort: board::Sort,
    query: String,
    stars: Vec<String>,
    items: Vec<zp::ChangeRequestListItem>,
    next_cursor: Option<String>,
    total: Option<u64>,
    fetched_at: Option<DateTime<Utc>>,
    error: Option<board::PageError>,
}

#[derive(uniffi::Object)]
pub struct PullRequestBoard {
    client: zc::Client,
    device_id: String,
    state: Mutex<BoardState>,
}

fn device_name(client: &zc::Client, device_id: &str) -> String {
    client
        .workspace()
        .devices
        .iter()
        .find(|device| device.id == device_id)
        .map(|device| device.name.clone())
        .unwrap_or_else(|| device_id.to_owned())
}

fn page_error(client: &zc::Client, device_id: &str, error: zc::ClientError) -> board::PageError {
    match error {
        zc::ClientError::HostError(code) => board::PageError::from_capability(&code),
        zc::ClientError::HostUnavailable(_) => {
            board::PageError::RemoteOffline(device_name(client, device_id))
        }
        zc::ClientError::Unsupported(_) => {
            board::PageError::UpdateRequired(device_name(client, device_id))
        }
        _ => board::PageError::Network,
    }
}

impl PullRequestBoard {
    async fn fetch(&self, more: bool, refresh: bool) -> PrBoardView {
        let (repository, filter, after) = {
            let state = lock(&self.state);
            let after = more.then(|| state.next_cursor.clone()).flatten();
            (state.repository.clone(), state.filter, after)
        };
        if more && after.is_none() {
            return self.view();
        }
        let client = self.client.clone();
        let device = self.device_id.clone();
        let reply = zc::runtime::run(async move {
            client
                .pull_request_page(&device, &repository, filter, after, refresh)
                .await
        })
        .await;
        let mut state = lock(&self.state);
        match reply {
            Ok(page) => {
                if !more {
                    state.items.clear();
                }
                for item in page.items {
                    if !state.items.iter().any(|known| known.url == item.url) {
                        state.items.push(item);
                    }
                }
                state.next_cursor = page.next_cursor;
                state.total = page.total_count;
                state.fetched_at = page.fetched_at.or(Some(Utc::now()));
                state.error = None;
            }
            Err(error) => state.error = Some(page_error(&self.client, &self.device_id, error)),
        }
        drop(state);
        self.view()
    }
}

#[uniffi::export]
impl PullRequestBoard {
    pub async fn load(&self, refresh: bool) -> PrBoardView {
        self.fetch(false, refresh).await
    }

    pub async fn load_more(&self) -> PrBoardView {
        self.fetch(true, false).await
    }

    pub async fn set_filter(&self, filter: PrFilter) -> PrBoardView {
        lock(&self.state).filter = match filter {
            PrFilter::Authored => zp::ChangeRequestFilter::Authored,
            PrFilter::Reviewing => zp::ChangeRequestFilter::Reviewing,
            PrFilter::All => zp::ChangeRequestFilter::All,
        };
        self.fetch(false, false).await
    }

    pub async fn set_repository(&self, repository: String) -> PrBoardView {
        if !board::valid_repository_filter(&repository) {
            return self.view();
        }
        lock(&self.state).repository = repository;
        self.fetch(false, false).await
    }

    pub fn set_view(&self, view: PrPersonalView) -> PrBoardView {
        lock(&self.state).view = match view {
            PrPersonalView::Everything => board::PersonalView::Everything,
            PrPersonalView::NeedsYou => board::PersonalView::NeedsYou,
            PrPersonalView::Starred => board::PersonalView::Starred,
        };
        self.view()
    }

    pub fn set_sort(&self, field: PrSortField, ascending: bool) -> PrBoardView {
        lock(&self.state).sort = board::Sort {
            field: match field {
                PrSortField::Updated => board::SortField::Updated,
                PrSortField::Opened => board::SortField::Opened,
                PrSortField::Changes => board::SortField::Changes,
            },
            direction: if ascending {
                board::SortDirection::Ascending
            } else {
                board::SortDirection::Descending
            },
        };
        self.view()
    }

    pub fn set_query(&self, query: String) -> PrBoardView {
        lock(&self.state).query = query;
        self.view()
    }

    pub fn set_stars(&self, stars: Vec<String>) -> PrBoardView {
        lock(&self.state).stars = stars;
        self.view()
    }

    pub fn view(&self) -> PrBoardView {
        let state = lock(&self.state);
        let now = Utc::now();
        let mut items: Vec<_> = state
            .items
            .iter()
            .filter(|item| state.view.shows(item, &state.stars))
            .filter(|item| board::matches_query(item, &state.query))
            .cloned()
            .collect();
        board::sort(&mut items, state.sort);
        let row = |item: &zp::ChangeRequestListItem| {
            let star_key = board::star_key(&item.url);
            PrRow {
                starred: state.stars.contains(&star_key),
                star_key,
                url: item.url.clone(),
                number: item.number,
                title: zp::view::single_line(&item.title),
                author: if item.author.login.is_empty() {
                    "Unknown author".into()
                } else {
                    item.author.login.clone()
                },
                status: board::row_status(item),
                status_tone: board_tone(board::status_tone(item)),
                glyph_tone: board_tone(board::glyph_tone(item)),
                ci_label: item.ci.state.label().into(),
                ci_tone: ci_tone(item.ci.state),
                changes: changes(item.additions, item.deletions),
                updated: board::compact_relative_time(item.updated_at, now),
                accessibility: format!(
                    "{}: {} #{}. {}",
                    item.title,
                    item.repository,
                    item.number,
                    board::status_description(item)
                ),
            }
        };
        let groups: Vec<_> = board::grouped(&items)
            .into_iter()
            .map(|(group, members)| PrGroup {
                key: group.key().into(),
                label: group.label().into(),
                rows: members
                    .into_iter()
                    .map(|index| row(&items[index]))
                    .collect(),
            })
            .collect();
        let shown = items.len();
        let summary = match (state.total, state.fetched_at) {
            (Some(total), Some(at)) => format!(
                "{shown} of {total} · updated {}",
                board::relative_time(at, now)
            ),
            (None, Some(at)) => format!("{shown} · updated {}", board::relative_time(at, now)),
            _ => String::new(),
        };
        PrBoardView {
            device_id: self.device_id.clone(),
            repository: state.repository.clone(),
            filter: match state.filter {
                zp::ChangeRequestFilter::Authored => PrFilter::Authored,
                zp::ChangeRequestFilter::Reviewing => PrFilter::Reviewing,
                zp::ChangeRequestFilter::All => PrFilter::All,
            },
            view: match state.view {
                board::PersonalView::Everything => PrPersonalView::Everything,
                board::PersonalView::NeedsYou => PrPersonalView::NeedsYou,
                board::PersonalView::Starred => PrPersonalView::Starred,
            },
            sort_field: match state.sort.field {
                board::SortField::Updated => PrSortField::Updated,
                board::SortField::Opened => PrSortField::Opened,
                board::SortField::Changes => PrSortField::Changes,
            },
            ascending: state.sort.direction == board::SortDirection::Ascending,
            query: state.query.clone(),
            empty: (groups.is_empty() && state.error.is_none() && state.fetched_at.is_some())
                .then(|| state.view.empty_copy().into()),
            error: state.error.as_ref().map(|error| {
                let (title, body) = error.copy();
                PrNotice { title, body }
            }),
            can_load_more: state.next_cursor.is_some(),
            groups,
            summary,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PrActivityKind {
    Comment,
    Review,
    Thread,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PrComment {
    pub id: String,
    pub author: String,
    pub body: String,
    pub when: String,
    pub mine: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PrThread {
    pub id: String,
    pub path: String,
    pub line: Option<u64>,
    pub resolved: bool,
    pub outdated: bool,
    pub comments: Vec<PrComment>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PrActivity {
    pub id: String,
    pub kind: PrActivityKind,
    pub comment: PrComment,
    pub state: Option<PrChip>,
    pub thread: Option<PrThread>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PrCheckRow {
    pub name: String,
    pub chip: PrChip,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PrHandoffKind {
    Review,
    AddressFeedback,
    FixChecks,
    ResolveConflicts,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PrHandoff {
    pub kind: PrHandoffKind,
    pub label: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PrDetailView {
    pub url: String,
    pub repository: String,
    pub number: u64,
    pub title: String,
    pub body: String,
    pub author: String,
    pub state: PrChip,
    pub review: Option<PrChip>,
    pub ci: PrChip,
    pub branch: String,
    pub changes: String,
    pub file_count: u32,
    pub blockers: Vec<String>,
    pub attention: Vec<String>,
    pub missing: Vec<String>,
    pub checks: Vec<PrCheckRow>,
    pub checks_empty: Option<String>,
    pub activity: Vec<PrActivity>,
    pub open_threads: u32,
    pub handoffs: Vec<PrHandoff>,
    pub can_judge: bool,
    pub is_open: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PrCodeView {
    pub files: Vec<DiffFileView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PrMergeMethod {
    Merge,
    Squash,
    Rebase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PrReviewEvent {
    Comment,
    Approve,
    RequestChanges,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PrMergeOption {
    pub method: PrMergeMethod,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PrLabel {
    pub name: String,
    pub color: Option<String>,
    pub applied: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PrReviewer {
    pub login: String,
    pub team: bool,
    pub requested: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PrControlsView {
    pub merge_methods: Vec<PrMergeOption>,
    pub can_write: bool,
    pub can_triage: bool,
    pub labels: Vec<PrLabel>,
    pub reviewers: Vec<PrReviewer>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PrConfirm {
    pub title: String,
    pub message: String,
    pub action: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PrAskContext {
    pub title: String,
    pub placeholder: String,
    pub chips: Vec<String>,
    pub block: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PrLink {
    pub url: String,
    pub repository: String,
    pub number: u64,
}

#[derive(Default)]
struct PageState {
    detail: Option<zp::ChangeRequestDetail>,
    patch: Option<String>,
}

#[derive(uniffi::Object)]
pub struct PullRequestPage {
    client: zc::Client,
    device_id: String,
    url: String,
    state: Mutex<PageState>,
}

fn merge_method(method: PrMergeMethod) -> zp::ChangeRequestMergeMethod {
    match method {
        PrMergeMethod::Merge => zp::ChangeRequestMergeMethod::Merge,
        PrMergeMethod::Squash => zp::ChangeRequestMergeMethod::Squash,
        PrMergeMethod::Rebase => zp::ChangeRequestMergeMethod::Rebase,
    }
}

fn handoff_kind(kind: PrHandoffKind) -> handoff::Handoff {
    match kind {
        PrHandoffKind::Review => handoff::Handoff::Review,
        PrHandoffKind::AddressFeedback => handoff::Handoff::AddressFeedback,
        PrHandoffKind::FixChecks => handoff::Handoff::FixChecks,
        PrHandoffKind::ResolveConflicts => handoff::Handoff::ResolveConflicts,
    }
}

fn readable(error: CoreError, hint: &str) -> CoreError {
    match error {
        CoreError::HostError { message } => CoreError::HostError {
            message: actions::readable(&message, hint),
        },
        other => other,
    }
}

fn comment_view(comment: &zp::ChangeRequestComment) -> PrComment {
    PrComment {
        id: comment.id.clone(),
        author: comment.author.login.clone(),
        body: comment.body.clone(),
        when: ago(if comment.submitted_at.is_empty() {
            &comment.created_at
        } else {
            &comment.submitted_at
        }),
        mine: comment.viewer_did_author,
    }
}

fn thread_view(thread: &zp::ChangeRequestReviewThread) -> PrThread {
    PrThread {
        id: thread.id.clone(),
        path: thread.path.clone(),
        line: thread.line.or(thread.original_line),
        resolved: thread.is_resolved,
        outdated: thread.is_outdated,
        comments: thread.comments.iter().map(comment_view).collect(),
    }
}

fn detail_view(pr: &zp::ChangeRequestDetail) -> PrDetailView {
    let assessment = Facts::from_detail(pr).assess();
    let threads = pr.review_threads.clone().unwrap_or_default();
    let stamp = |comment: &zp::ChangeRequestComment| {
        if comment.submitted_at.is_empty() {
            comment.created_at.clone()
        } else {
            comment.submitted_at.clone()
        }
    };
    let mut activity: Vec<(String, PrActivity)> = pr
        .comments
        .iter()
        .map(|comment| {
            (
                stamp(comment),
                PrActivity {
                    id: comment.id.clone(),
                    kind: PrActivityKind::Comment,
                    comment: comment_view(comment),
                    state: None,
                    thread: None,
                },
            )
        })
        .chain(
            pr.reviews
                .iter()
                .filter(|review| !review.body.trim().is_empty() || !review.state.is_empty())
                .map(|review| {
                    (
                        stamp(review),
                        PrActivity {
                            id: review.id.clone(),
                            kind: PrActivityKind::Review,
                            comment: comment_view(review),
                            state: (!review.state.is_empty())
                                .then(|| chip(detail::status_style(&review.state))),
                            thread: None,
                        },
                    )
                }),
        )
        .chain(threads.iter().filter_map(|thread| {
            let first = thread.comments.first()?;
            Some((
                stamp(first),
                PrActivity {
                    id: thread.id.clone(),
                    kind: PrActivityKind::Thread,
                    comment: comment_view(first),
                    state: None,
                    thread: Some(thread_view(thread)),
                },
            ))
        }))
        .collect();
    activity.sort_by(|a, b| a.0.cmp(&b.0));
    let state = if pr.is_draft && pr.state.eq_ignore_ascii_case("open") {
        "DRAFT"
    } else {
        pr.state.as_str()
    };
    PrDetailView {
        url: pr.url.clone(),
        repository: handoff::repository(&pr.url),
        number: pr.number,
        title: zp::view::single_line(&pr.title),
        body: pr.body.clone(),
        author: pr.author.login.clone(),
        state: chip(detail::status_style(state)),
        review: (!pr.review_decision.is_empty())
            .then(|| chip(detail::status_style(&pr.review_decision))),
        ci: chip(detail::status_style(detail::ci_status(pr))),
        branch: format!("{} \u{2192} {}", pr.head_ref_name, pr.base_ref_name),
        changes: changes(pr.additions, pr.deletions),
        file_count: pr.files.len() as u32,
        blockers: assessment
            .blockers
            .iter()
            .map(|b| b.label().into())
            .collect(),
        attention: assessment
            .attention
            .iter()
            .map(|a| a.label().into())
            .collect(),
        missing: assessment
            .missing
            .iter()
            .map(|m| m.label().into())
            .collect(),
        checks: pr
            .status_check_rollup
            .iter()
            .map(|check| {
                let raw = [&check.conclusion, &check.state, &check.status]
                    .into_iter()
                    .find(|value| !value.is_empty())
                    .cloned()
                    .unwrap_or_default();
                let url = [&check.details_url, &check.target_url]
                    .into_iter()
                    .find(|value| !value.is_empty())
                    .cloned();
                PrCheckRow {
                    name: if check.name.is_empty() {
                        check.context.clone()
                    } else {
                        check.name.clone()
                    },
                    chip: chip(detail::status_style(&raw)),
                    url,
                }
            })
            .collect(),
        checks_empty: pr
            .status_check_rollup
            .is_empty()
            .then(|| detail::checks_empty_copy(pr).into()),
        open_threads: threads.iter().filter(|t| !t.is_resolved).count() as u32,
        activity: activity.into_iter().map(|(_, item)| item).collect(),
        handoffs: handoff::handoffs(pr)
            .into_iter()
            .map(|kind| PrHandoff {
                kind: match kind {
                    handoff::Handoff::Review => PrHandoffKind::Review,
                    handoff::Handoff::AddressFeedback => PrHandoffKind::AddressFeedback,
                    handoff::Handoff::FixChecks => PrHandoffKind::FixChecks,
                    handoff::Handoff::ResolveConflicts => PrHandoffKind::ResolveConflicts,
                },
                label: kind.label().into(),
                summary: kind.summary().into(),
            })
            .collect(),
        can_judge: pr.viewer_did_author != Some(true),
        is_open: pr.state.eq_ignore_ascii_case("open"),
    }
}

impl PullRequestPage {
    fn detail(&self) -> CoreResult<zp::ChangeRequestDetail> {
        lock(&self.state)
            .detail
            .clone()
            .ok_or_else(|| CoreError::NotFound {
                message: "Load the pull request first.".into(),
            })
    }

    async fn controls_view(&self) -> CoreResult<PrControlsView> {
        let (client, device, url) = (
            self.client.clone(),
            self.device_id.clone(),
            self.url.clone(),
        );
        let controls =
            on_runtime(async move { client.pull_request_controls(&device, &url).await }).await?;
        Ok(PrControlsView {
            merge_methods: controls
                .merge_methods
                .iter()
                .map(|method| PrMergeOption {
                    method: match method {
                        zp::ChangeRequestMergeMethod::Merge => PrMergeMethod::Merge,
                        zp::ChangeRequestMergeMethod::Squash => PrMergeMethod::Squash,
                        zp::ChangeRequestMergeMethod::Rebase => PrMergeMethod::Rebase,
                    },
                    label: actions::method_entry(*method).0.into(),
                })
                .collect(),
            can_write: controls.can_write,
            can_triage: controls.can_triage,
            labels: controls
                .labels
                .into_iter()
                .map(|label| PrLabel {
                    name: label.name,
                    color: label.color,
                    applied: label.applied,
                })
                .collect(),
            reviewers: controls
                .reviewers
                .into_iter()
                .map(|reviewer| PrReviewer {
                    login: reviewer.login,
                    team: reviewer.team,
                    requested: reviewer.requested,
                })
                .collect(),
        })
    }
}

#[uniffi::export]
impl PullRequestPage {
    pub fn url(&self) -> String {
        self.url.clone()
    }

    pub async fn load(&self, refresh: bool) -> CoreResult<PrDetailView> {
        let (client, device, url) = (
            self.client.clone(),
            self.device_id.clone(),
            self.url.clone(),
        );
        let pr = on_runtime(async move { client.pull_request(&device, &url, refresh).await })
            .await
            .map_err(|error| match error {
                CoreError::HostError { message } => CoreError::HostError {
                    message: detail::failure_reason(&message).into(),
                },
                other => other,
            })?;
        let view = detail_view(&pr);
        let mut state = lock(&self.state);
        if state.detail.as_ref().map(|d| &d.head_ref_oid) != Some(&pr.head_ref_oid) {
            state.patch = None;
        }
        state.detail = Some(pr);
        Ok(view)
    }

    pub async fn code(&self, refresh: bool) -> CoreResult<PrCodeView> {
        let pr = self.detail()?;
        let cached = lock(&self.state).patch.clone().filter(|_| !refresh);
        let text = match cached {
            Some(text) => text,
            None => {
                let (client, device, for_fetch) =
                    (self.client.clone(), self.device_id.clone(), pr.clone());
                let text = on_runtime(async move {
                    client.pull_request_diff(&device, &for_fetch, refresh).await
                })
                .await?;
                lock(&self.state).patch = Some(text.clone());
                text
            }
        };
        let threads = pr.review_threads.unwrap_or_default();
        Ok(PrCodeView {
            files: diff_files(&text, &threads, &[]),
        })
    }

    pub async fn controls(&self) -> CoreResult<PrControlsView> {
        self.controls_view().await
    }

    pub async fn comment(&self, body: String) -> CoreResult<PrDetailView> {
        let (client, device, url) = (
            self.client.clone(),
            self.device_id.clone(),
            self.url.clone(),
        );
        on_runtime(async move {
            client
                .post_pull_request_comment(&device, &url, &body)
                .await
                .map(drop)
        })
        .await
        .map_err(|e| readable(e, actions::REFUSED))?;
        self.load(true).await
    }

    pub async fn reply(&self, thread_id: String, body: String) -> CoreResult<PrDetailView> {
        let (client, device, url) = (
            self.client.clone(),
            self.device_id.clone(),
            self.url.clone(),
        );
        on_runtime(async move {
            client
                .reply_to_pull_request_thread(&device, &url, &thread_id, &body)
                .await
        })
        .await
        .map_err(|e| readable(e, actions::REFUSED))?;
        self.load(true).await
    }

    pub async fn resolve(&self, thread_id: String, resolved: bool) -> CoreResult<PrDetailView> {
        let (client, device, url) = (
            self.client.clone(),
            self.device_id.clone(),
            self.url.clone(),
        );
        on_runtime(async move {
            client
                .set_pull_request_thread_resolved(&device, &url, &thread_id, resolved)
                .await
        })
        .await
        .map_err(|e| readable(e, actions::REFUSED))?;
        self.load(true).await
    }

    pub async fn review(&self, event: PrReviewEvent, body: String) -> CoreResult<PrDetailView> {
        let (client, device, url) = (
            self.client.clone(),
            self.device_id.clone(),
            self.url.clone(),
        );
        let event = match event {
            PrReviewEvent::Comment => zp::ChangeRequestReviewEvent::Comment,
            PrReviewEvent::Approve => zp::ChangeRequestReviewEvent::Approve,
            PrReviewEvent::RequestChanges => zp::ChangeRequestReviewEvent::RequestChanges,
        };
        on_runtime(async move {
            client
                .submit_pull_request_review(&device, &url, event, &body)
                .await
        })
        .await
        .map_err(|e| readable(e, actions::REFUSED))?;
        self.load(true).await
    }

    pub async fn set_label(&self, name: String, add: bool) -> CoreResult<PrControlsView> {
        let (client, device, url) = (
            self.client.clone(),
            self.device_id.clone(),
            self.url.clone(),
        );
        on_runtime(async move {
            client
                .set_pull_request_label(&device, &url, &name, add)
                .await
        })
        .await
        .map_err(|e| readable(e, actions::REFUSED))?;
        self.controls_view().await
    }

    pub async fn set_reviewer(
        &self,
        login: String,
        team: bool,
        add: bool,
    ) -> CoreResult<PrControlsView> {
        let (client, device, url) = (
            self.client.clone(),
            self.device_id.clone(),
            self.url.clone(),
        );
        on_runtime(async move {
            client
                .set_pull_request_reviewer(&device, &url, &login, team, add)
                .await
        })
        .await
        .map_err(|e| readable(e, actions::REFUSED))?;
        self.controls_view().await
    }

    pub fn merge_confirm(&self, method: PrMergeMethod) -> PrConfirm {
        let (number, base) = lock(&self.state)
            .detail
            .as_ref()
            .map(|d| (d.number, d.base_ref_name.clone()))
            .unwrap_or_default();
        let (title, message, action) = actions::merge_confirm(merge_method(method), number, &base);
        PrConfirm {
            title,
            message,
            action: action.into(),
        }
    }

    pub async fn merge(&self, method: PrMergeMethod) -> CoreResult<PrDetailView> {
        let (client, device, url) = (
            self.client.clone(),
            self.device_id.clone(),
            self.url.clone(),
        );
        let method = merge_method(method);
        on_runtime(async move { client.merge_pull_request(&device, &url, method).await })
            .await
            .map_err(|e| readable(e, actions::MERGE_HINT))?;
        self.load(true).await
    }

    pub fn handoff_prompt(&self, kind: PrHandoffKind) -> CoreResult<String> {
        Ok(handoff::prompt(
            handoff_kind(kind),
            &self.detail()?,
            &self.url,
            handoff::VerificationPlatform::Relevant,
        ))
    }

    pub async fn handoff_project(&self) -> Option<String> {
        let repository = handoff::repository(&self.url);
        let projects: Vec<(String, String)> = self
            .client
            .workspace()
            .projects
            .iter()
            .filter(|p| p.device_id == self.device_id && p.git_detected)
            .map(|p| (p.id.clone(), p.path.clone()))
            .collect();
        for (id, path) in projects {
            let (client, device, repo) = (
                self.client.clone(),
                self.device_id.clone(),
                repository.clone(),
            );
            let matched = on_runtime(async move {
                client
                    .checkout_repository(&device, &path, Some(&repo))
                    .await
            })
            .await;
            if matches!(matched, Ok(Some(_))) {
                return Some(id);
            }
        }
        None
    }

    pub fn ask_context(&self) -> CoreResult<PrAskContext> {
        let state = lock(&self.state);
        let context = PrChatContext::new(
            &self.url,
            Some(self.device_id.clone()),
            state.detail.clone(),
            state.patch.as_deref(),
        )
        .ok_or_else(|| CoreError::InvalidArgument {
            message: "Not a pull request link.".into(),
        })?;
        Ok(PrAskContext {
            title: context.promoted_title(),
            placeholder: context.placeholder().into(),
            chips: context.chips().into_iter().map(|(_, text)| text).collect(),
            block: context.block(&BTreeSet::new()).unwrap_or_default(),
        })
    }
}

#[uniffi::export]
pub fn pull_request_link(url: String) -> Option<PrLink> {
    let context = PrChatContext::new(&url, None, None, None)?;
    context
        .index_key()
        .to_ascii_lowercase()
        .starts_with("github.com/")
        .then(|| PrLink {
            repository: context.repository.clone(),
            number: context.number,
            url: format!(
                "https://github.com/{}/pull/{}",
                context.repository, context.number
            ),
        })
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct GitFileRow {
    pub path: String,
    pub letter: String,
    pub additions: u32,
    pub deletions: u32,
    pub staged: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct GitCheckoutOption {
    /// `None` is the project's main checkout.
    pub path: Option<String>,
    pub label: String,
    pub detail: String,
    pub live: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct GitNotice {
    pub text: String,
    pub error: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct GitTreeView {
    pub branch: String,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub clean: Option<String>,
    pub staged: Vec<GitFileRow>,
    pub changes: Vec<GitFileRow>,
    pub stashes: Vec<String>,
    pub agent_banner: Option<String>,
    pub pull_refusal: Option<String>,
    pub push_refusal: Option<String>,
    pub push_prompt: Option<String>,
    pub commit_refusal: Option<String>,
    pub can_amend: bool,
    pub subject_limit: u32,
    pub notice: Option<GitNotice>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum GitSegmentShape {
    Through,
    Incoming,
    Outgoing,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct GitSegment {
    pub from_lane: u32,
    pub to_lane: u32,
    pub color: u32,
    pub shape: GitSegmentShape,
    pub dashed: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct GitRefChip {
    pub label: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct GitHistoryRow {
    pub sha: String,
    pub short_sha: String,
    pub subject: String,
    pub author: String,
    pub when: String,
    pub refs: Vec<GitRefChip>,
    pub is_head: bool,
    pub lane: u32,
    pub color: u32,
    pub segments: Vec<GitSegment>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct GitHistoryView {
    pub rows: Vec<GitHistoryRow>,
    pub lane_count: u32,
    pub can_load_more: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct GitCommitView {
    pub sha: String,
    pub short_sha: String,
    pub subject: String,
    pub author: String,
    pub when: String,
    pub files: Vec<DiffFileView>,
}

#[derive(Default)]
struct GitState {
    tree: Option<zp::WorkingTree>,
    notice: Option<GitNotice>,
    history: Vec<zp::GitHistoryCommit>,
    next_cursor: Option<usize>,
    head_sha: Option<String>,
}

#[derive(uniffi::Object)]
pub struct GitCheckout {
    client: zc::Client,
    device_id: String,
    cwd: String,
    target: zp::WorkspaceTarget,
    state: Mutex<GitState>,
}

impl GitCheckout {
    fn supports(&self, capability: &str) -> bool {
        self.client
            .workspace()
            .devices
            .iter()
            .find(|device| device.id == self.device_id)
            .is_some_and(|device| device.capabilities.iter().any(|c| c == capability))
    }

    fn tree_view(&self) -> GitTreeView {
        let state = lock(&self.state);
        let syncs = self.supports(zp::capabilities::SOURCE_CONTROL_SYNC_V1);
        let commits = self.supports(zp::capabilities::SOURCE_CONTROL_COMMITS_V1);
        let policy = zp::GitAgentBusyPolicy::default();
        let Some(tree) = state.tree.as_ref() else {
            return GitTreeView {
                branch: String::new(),
                upstream: None,
                ahead: 0,
                behind: 0,
                clean: None,
                staged: Vec::new(),
                changes: Vec::new(),
                stashes: Vec::new(),
                agent_banner: None,
                pull_refusal: None,
                push_refusal: None,
                push_prompt: None,
                commit_refusal: None,
                can_amend: false,
                subject_limit: git_tab::SUBJECT_LIMIT as u32,
                notice: state.notice.clone(),
            };
        };
        let staged = git_tab::staged_files(tree);
        let changed = git_tab::changed_files(tree);
        let row = |file: &zp::WorkingTreeFile, staged: bool| {
            let (state, delta) = if staged {
                (file.index, file.staged)
            } else {
                (file.worktree, file.unstaged)
            };
            let delta = delta.unwrap_or_default();
            GitFileRow {
                path: file.path.clone(),
                letter: git_tab::status_letter(state).into(),
                additions: delta.additions,
                deletions: delta.deletions,
                staged,
            }
        };
        let commit_refusal = if !commits {
            Some("Zeron on this device is too old to commit. Update it first.".into())
        } else if staged.is_empty() {
            Some("Stage a change to commit.".into())
        } else {
            None
        };
        GitTreeView {
            branch: tree
                .branch
                .clone()
                .unwrap_or_else(|| "detached HEAD".into()),
            upstream: tree.upstream.clone(),
            ahead: tree.ahead,
            behind: tree.behind,
            clean: (staged.is_empty() && changed.is_empty()).then(|| git_tab::clean_status(tree)),
            staged: staged.iter().map(|file| row(file, true)).collect(),
            changes: changed.iter().map(|file| row(file, false)).collect(),
            stashes: tree.stashes.iter().map(|s| s.message.clone()).collect(),
            agent_banner: tree
                .agent_active
                .then(|| git_tab::agent_banner(policy).into()),
            pull_refusal: git_tab::sync_refusal(
                git_tab::SyncKind::Pull,
                tree,
                policy,
                syncs,
                false,
            ),
            push_refusal: git_tab::sync_refusal(
                git_tab::SyncKind::Push,
                tree,
                policy,
                syncs,
                false,
            ),
            push_prompt: git_tab::push_prompt(tree),
            commit_refusal,
            can_amend: commits && tree.head_sha.is_some(),
            subject_limit: git_tab::SUBJECT_LIMIT as u32,
            notice: state.notice.clone(),
        }
    }

    async fn read_tree(&self) -> CoreResult<()> {
        let (client, device, target) = (
            self.client.clone(),
            self.device_id.clone(),
            self.target.clone(),
        );
        let tree = on_runtime(async move { client.working_tree(&device, target).await }).await?;
        lock(&self.state).tree = Some(tree);
        Ok(())
    }

    async fn act(&self, action: zp::WorkingTreeAction) -> CoreResult<GitTreeView> {
        let revision = lock(&self.state)
            .tree
            .as_ref()
            .map(|tree| tree.revision.clone())
            .unwrap_or_default();
        let (client, device, target) = (
            self.client.clone(),
            self.device_id.clone(),
            self.target.clone(),
        );
        let reply = on_runtime(async move {
            client
                .run_working_tree_action(&device, target, revision, action)
                .await
        })
        .await;
        let notice = match reply {
            Ok(reply) => reply.commit.map(|commit| GitNotice {
                text: format!(
                    "Committed {} {}",
                    git_tab::short_sha(&commit.sha),
                    commit.subject
                ),
                error: false,
            }),
            Err(CoreError::HostError { message }) | Err(CoreError::InvalidArgument { message }) => {
                Some(GitNotice {
                    text: message,
                    error: true,
                })
            }
            Err(error) => return Err(error),
        };
        lock(&self.state).notice = notice;
        self.read_tree().await?;
        Ok(self.tree_view())
    }

    async fn sync(&self, kind: git_tab::SyncKind) -> CoreResult<GitTreeView> {
        self.read_tree().await?;
        let (tree, refusal) = {
            let state = lock(&self.state);
            let tree = state.tree.clone();
            let refusal = tree.as_ref().and_then(|tree| {
                git_tab::sync_refusal(
                    kind,
                    tree,
                    zp::GitAgentBusyPolicy::default(),
                    self.supports(zp::capabilities::SOURCE_CONTROL_SYNC_V1),
                    false,
                )
            });
            (tree, refusal)
        };
        let Some(tree) = tree else {
            return Ok(self.tree_view());
        };
        if let Some(text) = refusal {
            lock(&self.state).notice = Some(GitNotice { text, error: true });
            return Ok(self.tree_view());
        }
        let (direction, destination) = match kind {
            git_tab::SyncKind::Push => (
                git_tab::push_direction(&tree),
                git_tab::publish_destination(&tree),
            ),
            git_tab::SyncKind::Pull => (zp::SyncDirection::Pull, None),
        };
        let head = tree.head_sha.clone().unwrap_or_default();
        let (client, device, target) = (
            self.client.clone(),
            self.device_id.clone(),
            self.target.clone(),
        );
        let reply =
            on_runtime(async move { client.sync_branch(&device, target, head, direction).await })
                .await;
        let branch = tree.branch.clone().unwrap_or_default();
        lock(&self.state).notice = Some(match reply {
            Ok(reply) => GitNotice {
                text: git_tab::sync_note(kind, &branch, destination.as_deref(), &reply),
                error: false,
            },
            Err(CoreError::HostError { message }) => GitNotice {
                text: message,
                error: true,
            },
            Err(error) => return Err(error),
        });
        self.read_tree().await?;
        Ok(self.tree_view())
    }
}

#[uniffi::export]
impl GitCheckout {
    pub fn path(&self) -> String {
        self.cwd.clone()
    }

    pub async fn refresh(&self) -> CoreResult<GitTreeView> {
        self.read_tree().await?;
        Ok(self.tree_view())
    }

    pub fn dismiss_notice(&self) -> GitTreeView {
        lock(&self.state).notice = None;
        self.tree_view()
    }

    pub async fn stage(&self, paths: Vec<String>) -> CoreResult<GitTreeView> {
        self.act(zp::WorkingTreeAction::Stage { paths }).await
    }

    pub async fn unstage(&self, paths: Vec<String>) -> CoreResult<GitTreeView> {
        self.act(zp::WorkingTreeAction::Unstage { paths }).await
    }

    pub async fn stage_all(&self) -> CoreResult<GitTreeView> {
        self.act(zp::WorkingTreeAction::StageAll).await
    }

    pub async fn unstage_all(&self) -> CoreResult<GitTreeView> {
        self.act(zp::WorkingTreeAction::UnstageAll).await
    }

    pub async fn commit(&self, message: String, amend: bool, all: bool) -> CoreResult<GitTreeView> {
        self.act(zp::WorkingTreeAction::Commit {
            message,
            amend,
            all,
        })
        .await
    }

    pub async fn generate_message(&self) -> CoreResult<String> {
        let (client, device, target) = (
            self.client.clone(),
            self.device_id.clone(),
            self.target.clone(),
        );
        on_runtime(async move { client.generate_commit_message(&device, target).await })
            .await
            .map(|generated| generated.message)
    }

    pub async fn file_diff(&self, path: String, staged: bool) -> CoreResult<Vec<DiffFileView>> {
        let (client, device, target) = (
            self.client.clone(),
            self.device_id.clone(),
            self.target.clone(),
        );
        let diff = on_runtime(async move {
            client
                .working_tree_file_diff(&device, target, path, staged)
                .await
        })
        .await?;
        Ok(diff_files(&diff.patch, &[], &[]))
    }

    pub async fn pull(&self) -> CoreResult<GitTreeView> {
        self.sync(git_tab::SyncKind::Pull).await
    }

    /// Run only after the reader confirmed `push_prompt`.
    pub async fn push(&self) -> CoreResult<GitTreeView> {
        self.sync(git_tab::SyncKind::Push).await
    }

    pub async fn history(&self, more: bool) -> CoreResult<GitHistoryView> {
        let cursor = if more {
            match lock(&self.state).next_cursor {
                Some(cursor) => cursor,
                None => return Ok(self.history_view()),
            }
        } else {
            0
        };
        let (client, device, cwd) = (
            self.client.clone(),
            self.device_id.clone(),
            self.cwd.clone(),
        );
        let page = on_runtime(async move {
            client
                .git_history(&device, &cwd, cursor, HISTORY_PAGE)
                .await
        })
        .await?;
        let mut state = lock(&self.state);
        if !more {
            state.history.clear();
        }
        state.history.extend(page.commits);
        state.next_cursor = page.next_cursor;
        state.head_sha = page.head_sha;
        drop(state);
        Ok(self.history_view())
    }

    pub async fn commit_detail(&self, sha: String) -> CoreResult<GitCommitView> {
        let commit = lock(&self.state)
            .history
            .iter()
            .find(|commit| commit.sha == sha)
            .cloned();
        let (client, device, cwd, wanted) = (
            self.client.clone(),
            self.device_id.clone(),
            self.cwd.clone(),
            sha.clone(),
        );
        let diff =
            on_runtime(async move { client.commit_diff(&device, &cwd, &wanted).await }).await?;
        Ok(GitCommitView {
            short_sha: git_tab::short_sha(&sha),
            subject: commit
                .as_ref()
                .map(|c| c.subject.clone())
                .unwrap_or_default(),
            author: commit
                .as_ref()
                .map(|c| c.author_name.clone())
                .unwrap_or_default(),
            when: commit
                .as_ref()
                .map(|c| ago(&c.authored_at))
                .unwrap_or_default(),
            sha,
            files: diff_files(&diff.patch, &[], &[]),
        })
    }
}

impl GitCheckout {
    fn history_view(&self) -> GitHistoryView {
        let state = lock(&self.state);
        let layout = git_graph::layout_graph(&state.history, state.head_sha.as_deref());
        let rows = state
            .history
            .iter()
            .zip(layout.rows)
            .map(|(commit, row)| GitHistoryRow {
                short_sha: git_tab::short_sha(&commit.sha),
                sha: commit.sha.clone(),
                subject: commit.subject.clone(),
                author: commit.author_name.clone(),
                when: ago(&commit.authored_at),
                refs: commit
                    .refs
                    .iter()
                    .map(|r| GitRefChip {
                        label: r.label.clone(),
                        kind: match r.kind {
                            zp::GitHistoryRefKind::Branch => "branch",
                            zp::GitHistoryRefKind::Remote => "remote",
                            zp::GitHistoryRefKind::Tag => "tag",
                        }
                        .into(),
                    })
                    .collect(),
                is_head: row.is_head,
                lane: row.node_lane as u32,
                color: row.node_color_id as u32,
                segments: row
                    .segments
                    .iter()
                    .map(|segment| GitSegment {
                        from_lane: segment.from_lane as u32,
                        to_lane: segment.to_lane as u32,
                        color: segment.color_id as u32,
                        shape: match segment.shape {
                            git_graph::SegmentShape::Through => GitSegmentShape::Through,
                            git_graph::SegmentShape::Incoming => GitSegmentShape::Incoming,
                            git_graph::SegmentShape::Outgoing => GitSegmentShape::Outgoing,
                        },
                        dashed: segment.dashed,
                    })
                    .collect(),
            })
            .collect();
        GitHistoryView {
            rows,
            lane_count: layout.max_lane_count as u32,
            can_load_more: state.next_cursor.is_some(),
        }
    }
}

#[uniffi::export]
impl CoreClient {
    pub fn pull_request_devices(&self) -> Vec<PrDevice> {
        self.client
            .workspace()
            .devices
            .iter()
            .filter(|device| {
                device
                    .capabilities
                    .iter()
                    .any(|c| c == zp::capabilities::PULL_REQUESTS_PAGE_V1)
            })
            .map(|device| PrDevice {
                id: device.id.clone(),
                name: device.name.clone(),
                online: device.online,
            })
            .collect()
    }

    pub async fn pull_request_repositories(&self, device_id: String) -> Vec<PrRepository> {
        let client = self.client.clone();
        on_runtime(async move {
            Ok::<_, zc::ClientError>(client.pull_request_repositories(&device_id).await)
        })
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(repository, project_id)| PrRepository {
            repository,
            project_id,
        })
        .collect()
    }

    pub fn pull_request_board(
        &self,
        device_id: String,
        repository: String,
        stars: Vec<String>,
    ) -> Arc<PullRequestBoard> {
        Arc::new(PullRequestBoard {
            client: self.client.clone(),
            device_id,
            state: Mutex::new(BoardState {
                repository,
                filter: zp::ChangeRequestFilter::Authored,
                view: board::PersonalView::Everything,
                sort: board::Sort::DEFAULT,
                query: String::new(),
                stars,
                items: Vec::new(),
                next_cursor: None,
                total: None,
                fetched_at: None,
                error: None,
            }),
        })
    }

    pub fn pull_request_page(&self, device_id: String, url: String) -> Arc<PullRequestPage> {
        Arc::new(PullRequestPage {
            client: self.client.clone(),
            device_id,
            url,
            state: Mutex::new(PageState::default()),
        })
    }

    pub fn git_checkouts(&self, project_id: String) -> Vec<GitCheckoutOption> {
        let workspace = self.client.workspace();
        let Some(project) = workspace.project(&project_id) else {
            return Vec::new();
        };
        let chats = workspace.sessions.values().filter_map(|row| {
            if row.archived
                || row.device_id != project.device_id
                || row.project.as_ref().map(|p| p.id.as_str()) != Some(project.id.as_str())
            {
                return None;
            }
            Some(git_tab::CheckoutChat {
                cwd: row.cwd.as_deref()?,
                branch: row.branch.as_deref(),
                title: row.has_title.then_some(row.title.as_str()),
                live: matches!(
                    row.indicator,
                    zp::ChatIndicator::Working | zp::ChatIndicator::AwaitingInput
                ),
            })
        });
        git_tab::checkout_options(&project.path, chats)
            .into_iter()
            .map(|option| GitCheckoutOption {
                path: option.path,
                label: option.label,
                detail: option.detail,
                live: option.live,
            })
            .collect()
    }

    pub fn git_checkout(
        &self,
        project_id: String,
        checkout_path: Option<String>,
    ) -> CoreResult<Arc<GitCheckout>> {
        let workspace = self.client.workspace();
        let project = workspace
            .project(&project_id)
            .ok_or_else(|| CoreError::NotFound {
                message: project_id.clone(),
            })?;
        Ok(Arc::new(GitCheckout {
            client: self.client.clone(),
            device_id: project.device_id.clone(),
            cwd: checkout_path
                .clone()
                .unwrap_or_else(|| project.path.clone()),
            target: zp::WorkspaceTarget {
                chat_id: None,
                space_id: Some(project.id.clone()),
                checkout_path,
            },
            state: Mutex::new(GitState::default()),
        }))
    }
}
