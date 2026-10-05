//! Pure source-control remote and branch normalization.
//!
//! Process execution and provider calls intentionally live outside this layer so
//! remote parsing and head selector construction remain deterministic and testable.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncReadExt};

use zeron_proto::{
    ChangeRequestAction, ChangeRequestAutoMerge, ChangeRequestLabelCandidate,
    ChangeRequestLabelCandidates, ChangeRequestMergeMethod, ChangeRequestPermissions,
    ChangeRequestReviewerCandidate, ChangeRequestReviewerCandidates, RunChangeRequestActionRequest,
    SetChangeRequestLabelRequest, SetChangeRequestReviewerRequest, UpdateChangeRequestRequest,
};
use zeron_proto::{
    ChangeRequestActivity, ChangeRequestActivityComment, ChangeRequestActivityCommit,
    ChangeRequestCommit, ChangeRequestDiffSlice, ChangeRequestFileStat, ChangeRequestReaction,
    ChangeRequestReactionContent, GetChangeRequestDiffRequest, PostChangeRequestCommentRequest,
    SetChangeRequestReactionRequest,
};
use zeron_proto::{
    ChangeRequestActor, ChangeRequestCheck, ChangeRequestCheckStatus, ChangeRequestChecksState,
    ChangeRequestComment, ChangeRequestCommentKind, ChangeRequestDetail, ChangeRequestDraftFilter,
    ChangeRequestFailure, ChangeRequestInvolvement, ChangeRequestLabel, ChangeRequestListError,
    ChangeRequestListFilters, ChangeRequestListItem, ChangeRequestListState, ChangeRequestListing,
    ChangeRequestMergeability, ChangeRequestRef, ChangeRequestReviewDecision,
    ChangeRequestReviewFilter, ChangeRequestReviewer, ChangeRequestStat, ChangeRequestState,
    ChangeRequestStats, ChangeRequestSummary, ChangeRequestVerdict, GetChangeRequestRequest,
    ListChangeRequestStatsRequest, ListChangeRequestsRequest,
};
use zeron_proto::{ChangeRequestChat, ChangeRequestCheckoutMode, PrepareChangeRequestChatRequest};
use zeron_proto::{
    ChangeRequestReviewEvent, ReplyToChangeRequestThreadRequest,
    SetChangeRequestThreadResolvedRequest, SubmitChangeRequestReviewRequest,
};
use zeron_proto::{CreateChangeRequestRequest, CreatedChangeRequest};

const GIT_TIMEOUT: Duration = Duration::from_secs(10);
const GITHUB_TIMEOUT: Duration = Duration::from_secs(20);
const GITHUB_LIST_TIMEOUT: Duration = Duration::from_secs(30);
const GIT_OUTPUT_LIMIT: usize = 64 * 1024;
const GITHUB_OUTPUT_LIMIT: usize = 1024 * 1024;
const GITHUB_LIST_OUTPUT_LIMIT: usize = 8 * 1024 * 1024;
const GITHUB_RESULT_LIMIT: &str = "20";
const GITHUB_JSON_FIELDS: &str = "number,title,url,state,baseRefName,headRefName,updatedAt,isCrossRepository,headRepositoryOwner";
const GITHUB_DETAIL_FIELDS: &str = "number,title,url,author,headRefName,baseRefName,state,isDraft,mergeable,reviewDecision,createdAt,updatedAt,mergedAt,reviewRequests,labels,additions,deletions,statusCheckRollup,body,changedFiles,closedAt,isCrossRepository,comments,reviews,latestReviews,commits,autoMergeRequest,headRepositoryOwner";
const GITHUB_LIST_FIELDS: &str = "number,title,url,author,headRefName,baseRefName,state,isDraft,mergeable,reviewDecision,createdAt,updatedAt,mergedAt,reviewRequests,labels";
const GITHUB_SEARCH_FIELDS: &str = "number title url author { login } headRefName baseRefName state isDraft mergeable reviewDecision createdAt updatedAt mergedAt repository { nameWithOwner } reviewRequests(first: 20) { nodes { requestedReviewer { ... on User { login } } } } labels(first: 20) { nodes { name color } } commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }";
const GITHUB_DIFF_TIMEOUT: Duration = Duration::from_secs(60);
const DIFF_FILES_PAGE_SIZE: usize = 100;
const REVIEW_THREAD_PAGES: usize = 10;
const REACTION_GROUPS: &str = "reactionGroups { content viewerHasReacted reactors(first: 10) { totalCount nodes { ... on User { login } ... on Bot { login } ... on Organization { login } ... on Mannequin { login } } } }";
const STATS_CHUNK: usize = 25;
const STATS_CONCURRENCY: usize = 4;
const MAX_STATS_REFS: usize = 500;
pub const DEFAULT_LIST_LIMIT: u32 = 99;
const MAX_LIST_LIMIT: u32 = 500;

/// Repository identity extracted from a Git remote URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitRemote {
    pub host: String,
    pub owner: String,
    pub repository: String,
}

/// Normalized branch context used to query a change request provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchHeadContext {
    pub local_branch: String,
    pub upstream_ref: Option<String>,
    pub remote_name: Option<String>,
    pub remote_url: Option<String>,
    pub host: Option<String>,
    pub repository: Option<String>,
    pub owner: Option<String>,
    pub head_branch: String,
    pub head_selectors: Vec<String>,
}

/// Git metadata needed by a provider to resolve a branch's change request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckoutSourceContext {
    pub checkout_root: PathBuf,
    pub branch: BranchHeadContext,
    pub default_branch: Option<String>,
}

/// A provider lookup paired with the exact Git context that produced it.
#[derive(Debug, Clone, PartialEq)]
pub struct ChangeRequestResolution {
    pub source: CheckoutSourceContext,
    pub change_request: Option<ChangeRequestSummary>,
}

/// Safe, provider-facing failure classes. These variants never contain command output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ChangeRequestError {
    #[error("Git repository context is unavailable")]
    RepositoryUnavailable,
    #[error("GitHub is unavailable for this repository")]
    UnsupportedRepository,
    #[error("GitHub CLI is not installed")]
    CliUnavailable,
    #[error("GitHub CLI is not authenticated for this host")]
    Authentication,
    #[error("GitHub API rate limit reached")]
    RateLimited,
    #[error("GitHub request timed out")]
    Timeout,
    #[error("GitHub returned an invalid response")]
    Decode,
    #[error("GitHub request failed")]
    CommandFailed,
}

/// Provider boundary shared by the checkout badge and future source-control views.
#[async_trait]
pub trait ChangeRequestProvider: Send + Sync {
    fn provider(&self) -> &'static str;

    async fn find_for_branch(
        &self,
        source: &CheckoutSourceContext,
    ) -> Result<Option<ChangeRequestSummary>, ChangeRequestError>;
}

/// Checkout inspection plus provider resolution, injectable for cache/service tests.
#[async_trait]
pub trait CheckoutChangeRequestLookup: Send + Sync {
    async fn inspect_checkout(
        &self,
        cwd: &Path,
    ) -> Result<CheckoutSourceContext, ChangeRequestError>;

    async fn resolve_github_source(
        &self,
        source: &CheckoutSourceContext,
    ) -> Result<Option<ChangeRequestSummary>, ChangeRequestError>;
}

/// Host-side resolver. All subprocesses run on the device that owns `cwd`.
#[derive(Clone)]
pub struct ChangeRequestResolver {
    inspector: GitCheckoutInspector,
    github: GitHubCli,
}

impl ChangeRequestResolver {
    pub fn new() -> Self {
        let runner: Arc<dyn ProcessRunner> = Arc::new(SystemProcessRunner);
        Self {
            inspector: GitCheckoutInspector::new(runner.clone()),
            github: GitHubCli::with_runner(runner),
        }
    }

    pub async fn resolve_github(
        &self,
        cwd: &Path,
    ) -> Result<ChangeRequestResolution, ChangeRequestError> {
        let source = self.inspect_checkout(cwd).await?;
        let change_request = self.resolve_github_source(&source).await?;
        Ok(ChangeRequestResolution {
            source,
            change_request,
        })
    }

    /// Inspect Git separately so callers can key caches before a provider request.
    pub async fn inspect_checkout(
        &self,
        cwd: &Path,
    ) -> Result<CheckoutSourceContext, ChangeRequestError> {
        self.inspector.inspect(cwd).await
    }

    /// Resolve an already-inspected checkout without repeating Git subprocesses.
    pub async fn resolve_github_source(
        &self,
        source: &CheckoutSourceContext,
    ) -> Result<Option<ChangeRequestSummary>, ChangeRequestError> {
        if source.branch.host.is_none()
            || source.branch.owner.is_none()
            || source.branch.repository.is_none()
        {
            return Err(ChangeRequestError::UnsupportedRepository);
        }
        self.github.find_for_branch(source).await
    }
}

impl Default for ChangeRequestResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl ChangeRequestResolver {
    pub async fn list_change_requests(
        &self,
        roots: &[PathBuf],
        request: &ListChangeRequestsRequest,
    ) -> ChangeRequestListing {
        let mut listing = ChangeRequestListing::default();
        let mut repositories: Vec<RepositoryIdentity> = Vec::new();
        let identities = futures::future::join_all(
            roots
                .iter()
                .map(|root| async move { (root, self.inspector.repository_identity(root).await) }),
        )
        .await;
        for (root, identity) in identities {
            match identity {
                Ok(identity) => {
                    listing.projects.push(zeron_proto::ChangeRequestProject {
                        repository: identity.name_with_owner(),
                        host: identity.remote.host.clone(),
                        space_id: String::new(),
                        path: root.to_string_lossy().into_owned(),
                    });
                    if !repositories
                        .iter()
                        .any(|known| known.remote.same_repository(&identity.remote))
                    {
                        repositories.push(identity);
                    }
                }
                Err(error) => listing.errors.push(ChangeRequestListError {
                    host: None,
                    repository: None,
                    path: root.to_string_lossy().into_owned(),
                    failure: error.into(),
                }),
            }
        }
        listing.repositories = repositories
            .iter()
            .map(RepositoryIdentity::name_with_owner)
            .collect();
        listing.repositories.sort();
        listing.repositories.dedup();
        if let Some(wanted) = &request.filters.repository {
            repositories.retain(|identity| identity.name_with_owner().eq_ignore_ascii_case(wanted));
        }

        let mut hosts: Vec<(String, PathBuf)> = Vec::new();
        for identity in &repositories {
            if !hosts.iter().any(|(host, _)| *host == identity.remote.host) {
                hosts.push((identity.remote.host.clone(), identity.root.clone()));
            }
        }
        let viewers = futures::future::join_all(hosts.iter().map(|(host, root)| async move {
            (host.clone(), self.github.viewer_login(host, root).await)
        }))
        .await;
        let mut viewer_failures = Vec::new();
        for (host, viewer) in viewers {
            match viewer {
                Ok(login) => {
                    listing.viewers.insert(host, login);
                }
                Err(error) => viewer_failures.push((host, error)),
            }
        }

        let limit = request
            .limit
            .unwrap_or(DEFAULT_LIST_LIMIT)
            .clamp(1, MAX_LIST_LIMIT);
        let mut readable = Vec::new();
        for identity in repositories {
            if let Some((_, error)) = viewer_failures
                .iter()
                .find(|(host, _)| *host == identity.remote.host)
            {
                listing.errors.push(identity.list_error((*error).into()));
            } else {
                readable.push(identity);
            }
        }
        let mut by_host: Vec<Vec<&RepositoryIdentity>> = Vec::new();
        for identity in &readable {
            match by_host
                .iter_mut()
                .find(|group| group[0].remote.host == identity.remote.host)
            {
                Some(group) => group.push(identity),
                None => by_host.push(vec![identity]),
            }
        }
        let searches = futures::future::join_all(by_host.iter().map(|group| {
            let viewer = listing.viewers[&group[0].remote.host].clone();
            async move {
                let result = self
                    .github
                    .search_host_items(group, request, &viewer, limit)
                    .await;
                (group, viewer, result)
            }
        }))
        .await;
        let mut fallback = Vec::new();
        for (group, viewer, result) in searches {
            match result {
                Ok(Some((items, truncated))) => {
                    listing.items.extend(items);
                    listing.truncated |= truncated;
                }
                Ok(None) => {
                    fallback.extend(group.iter().map(|identity| (*identity, viewer.clone())))
                }
                Err(error) => listing.errors.extend(
                    group
                        .iter()
                        .map(|identity| identity.list_error(error.into())),
                ),
            }
        }
        let reads =
            futures::future::join_all(fallback.iter().map(|(identity, viewer)| async move {
                let result = self
                    .github
                    .list_repository_items(identity, request, viewer, limit)
                    .await;
                (*identity, result)
            }))
            .await;
        for (identity, result) in reads {
            match result {
                Ok((items, truncated)) => {
                    listing.items.extend(items);
                    listing.truncated |= truncated;
                }
                Err(error) => listing.errors.push(identity.list_error(error.into())),
            }
        }
        listing
            .items
            .sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        listing
    }
}

impl ChangeRequestResolver {
    pub async fn list_change_request_stats(
        &self,
        roots: &[PathBuf],
        request: &ListChangeRequestStatsRequest,
    ) -> ChangeRequestStats {
        let identities: Vec<RepositoryIdentity> = futures::future::join_all(
            roots
                .iter()
                .map(|root| self.inspector.repository_identity(root)),
        )
        .await
        .into_iter()
        .filter_map(Result::ok)
        .collect();
        let mut chunks: Vec<(&RepositoryIdentity, Vec<&ChangeRequestRef>)> = Vec::new();
        for reference in request.refs.iter().take(MAX_STATS_REFS) {
            let Some(identity) = identities.iter().find(|identity| {
                identity.remote.host.eq_ignore_ascii_case(&reference.host)
                    && identity
                        .name_with_owner()
                        .eq_ignore_ascii_case(&reference.repository)
            }) else {
                continue;
            };
            if !valid_repository_path(&reference.repository) || reference.number == 0 {
                continue;
            }
            match chunks.iter_mut().find(|(host, refs)| {
                host.remote.host == identity.remote.host && refs.len() < STATS_CHUNK
            }) {
                Some((_, refs)) => refs.push(reference),
                None => chunks.push((identity, vec![reference])),
            }
        }
        let mut stats = Vec::new();
        for batch in chunks.chunks(STATS_CONCURRENCY) {
            let results = futures::future::join_all(
                batch
                    .iter()
                    .map(|(identity, refs)| self.github.pull_request_stats(identity, refs)),
            )
            .await;
            stats.extend(results.into_iter().flatten());
        }
        ChangeRequestStats { stats }
    }

    pub async fn get_change_request(
        &self,
        roots: &[PathBuf],
        request: &GetChangeRequestRequest,
    ) -> Result<ChangeRequestDetail, ChangeRequestError> {
        let identity = self
            .space_identity(roots, &request.host, &request.repository)
            .await?;
        let (viewer, view) = futures::try_join!(
            self.github
                .viewer_login(&identity.remote.host, &identity.root),
            self.github.view_pull_request(&identity, request.number),
        )?;
        to_detail(&identity, &viewer, view).ok_or(ChangeRequestError::Decode)
    }

    /// One page of a pull request's changed files as a unified patch; `cursor` is the page
    /// number the previous slice handed back.
    pub async fn get_change_request_diff(
        &self,
        roots: &[PathBuf],
        request: &GetChangeRequestDiffRequest,
    ) -> Result<ChangeRequestDiffSlice, ChangeRequestError> {
        let page = match request.cursor.as_deref() {
            None => 1,
            Some(cursor) => cursor
                .parse::<u32>()
                .ok()
                .filter(|page| *page >= 1)
                .ok_or(ChangeRequestError::Decode)?,
        };
        if request
            .commit
            .as_deref()
            .is_some_and(|commit| !valid_commit_sha(commit))
        {
            return Err(ChangeRequestError::Decode);
        }
        let identity = self
            .space_identity(roots, &request.host, &request.repository)
            .await?;
        let stdout = self
            .github
            .diff_files_page(&identity, request.number, page, request.commit.as_deref())
            .await?;
        let (mut slice, raw_count) = files_patch(&stdout).ok_or(ChangeRequestError::Decode)?;
        if raw_count >= DIFF_FILES_PAGE_SIZE {
            slice.next_cursor = Some((page + 1).to_string());
        }
        Ok(slice)
    }

    /// The conversation t3code reads for the Timeline and the Summary's Comments: comments,
    /// reviews, review-thread comments, reactions and commits, from one GraphQL read plus up
    /// to ten pages of review threads in all.
    pub async fn get_change_request_activity(
        &self,
        roots: &[PathBuf],
        request: &GetChangeRequestRequest,
    ) -> Result<ChangeRequestActivity, ChangeRequestError> {
        let identity = self
            .space_identity(roots, &request.host, &request.repository)
            .await?;
        let first = self
            .github
            .graphql(
                &identity,
                &activity_query(),
                activity_variables(&identity, request.number, None),
            )
            .await?;
        let first: GhActivityResponse =
            serde_json::from_slice(&first).map_err(|_| ChangeRequestError::Decode)?;
        let mut data = first.data.ok_or(ChangeRequestError::Decode)?;
        let pull = data
            .repository
            .as_mut()
            .and_then(|repository| repository.pull_request.as_mut())
            .ok_or(ChangeRequestError::Decode)?;
        let mut threads = std::mem::take(&mut pull.review_threads.nodes);
        let mut page = pull.review_threads.page_info.clone();
        let mut pages = 1;
        while page.has_next_page && pages < REVIEW_THREAD_PAGES {
            let Some(cursor) = page.end_cursor.clone() else {
                break;
            };
            let next = self
                .github
                .graphql(
                    &identity,
                    &threads_query(),
                    activity_variables(&identity, request.number, Some(cursor)),
                )
                .await?;
            let next: GhActivityResponse =
                serde_json::from_slice(&next).map_err(|_| ChangeRequestError::Decode)?;
            let connection = next
                .data
                .and_then(|data| data.repository)
                .and_then(|repository| repository.pull_request)
                .map(|pull| pull.review_threads)
                .ok_or(ChangeRequestError::Decode)?;
            threads.extend(connection.nodes);
            page = connection.page_info;
            pages += 1;
        }
        Ok(to_activity(
            &identity.remote.host,
            data,
            threads,
            page.has_next_page,
        ))
    }

    pub async fn post_change_request_comment(
        &self,
        roots: &[PathBuf],
        request: &PostChangeRequestCommentRequest,
    ) -> Result<(), ChangeRequestError> {
        if request.body.trim().is_empty() {
            return Err(ChangeRequestError::Decode);
        }
        let identity = self
            .space_identity(roots, &request.host, &request.repository)
            .await?;
        self.github
            .run_gh_input(
                &identity.root,
                vec![
                    "pr".into(),
                    "comment".into(),
                    request.number.to_string(),
                    "--repo".into(),
                    format!("{}/{}", identity.remote.host, identity.name_with_owner()),
                    "--body-file".into(),
                    "-".into(),
                ],
                GITHUB_TIMEOUT,
                GITHUB_OUTPUT_LIMIT,
                Some(request.body.clone().into_bytes()),
            )
            .await
            .map(|_| ())
    }

    /// Reacts to the pull request itself when no subject is given; any other subject must be
    /// a comment, review or review comment GitHub places on this same pull request.
    pub async fn set_change_request_reaction(
        &self,
        roots: &[PathBuf],
        request: &SetChangeRequestReactionRequest,
    ) -> Result<(), ChangeRequestError> {
        let identity = self
            .space_identity(roots, &request.host, &request.repository)
            .await?;
        let subject = request
            .subject_id
            .clone()
            .filter(|subject| !subject.is_empty());
        let mut variables = activity_variables(&identity, request.number, None);
        variables["subjectId"] = serde_json::Value::String(subject.clone().unwrap_or_default());
        let query = if subject.is_some() {
            SUBJECT_SCOPE_QUERY
        } else {
            PULL_REQUEST_ID_QUERY
        };
        let scope = self.github.graphql(&identity, query, variables).await?;
        let scope: GhScopeResponse =
            serde_json::from_slice(&scope).map_err(|_| ChangeRequestError::Decode)?;
        let subject = subject_in_scope(&scope, subject.as_deref())
            .ok_or(ChangeRequestError::UnsupportedRepository)?;
        let mutation = if request.add {
            "mutation($subjectId: ID!, $content: ReactionContent!) { addReaction(input: { subjectId: $subjectId, content: $content }) { reaction { content } } }"
        } else {
            "mutation($subjectId: ID!, $content: ReactionContent!) { removeReaction(input: { subjectId: $subjectId, content: $content }) { reaction { content } } }"
        };
        self.github
            .graphql(
                &identity,
                mutation,
                serde_json::json!({
                    "subjectId": subject,
                    "content": reaction_name(request.content),
                }),
            )
            .await
            .map(|_| ())
    }

    /// What the reader may do on this pull request, read fresh from the host (t3code
    /// `getViewerPermissions`): repository merge strategies and access, and whether the branch
    /// can be brought up to date.
    pub async fn get_change_request_permissions(
        &self,
        roots: &[PathBuf],
        request: &GetChangeRequestRequest,
    ) -> Result<ChangeRequestPermissions, ChangeRequestError> {
        let identity = self
            .space_identity(roots, &request.host, &request.repository)
            .await?;
        self.permissions(&identity, request.number).await
    }

    async fn permissions(
        &self,
        identity: &RepositoryIdentity,
        number: u64,
    ) -> Result<ChangeRequestPermissions, ChangeRequestError> {
        let raw = self
            .github
            .graphql(
                identity,
                PERMISSIONS_QUERY,
                activity_variables(identity, number, None),
            )
            .await?;
        let response: GhPermissionsResponse =
            serde_json::from_slice(&raw).map_err(|_| ChangeRequestError::Decode)?;
        let repository = response
            .data
            .and_then(|data| data.repository)
            .ok_or(ChangeRequestError::Decode)?;
        let flags = repository_flags(&repository);
        let pull = repository
            .pull_request
            .as_ref()
            .ok_or(ChangeRequestError::Decode)?;
        let mut permissions = to_permissions(&flags, pull);
        let open = pull.state.eq_ignore_ascii_case("OPEN");
        if let (true, Some(owner)) = (open, pull.head_repository_owner.as_ref()) {
            let mut variables = activity_variables(identity, number, None);
            variables["headRef"] =
                serde_json::Value::String(format!("{}:{}", owner.login, pull.head_ref_name));
            let comparison = self
                .github
                .graphql(identity, BASE_COMPARISON_QUERY, variables)
                .await
                .ok()
                .and_then(|raw| serde_json::from_slice::<GhComparisonResponse>(&raw).ok())
                .and_then(|response| response.data?.repository?.pull_request);
            if let Some(comparison) = comparison {
                permissions.can_update_branch = comparison.viewer_can_update_branch == Some(true);
                permissions.behind_by = comparison
                    .base_ref
                    .and_then(|base| base.compare)
                    .map(|compare| compare.behind_by);
            }
        }
        permissions.default_branch = repository.default_branch_ref.map(|branch| branch.name);
        Ok(permissions)
    }

    /// Runs one host action after a fresh permission read; a comment, when given, is posted
    /// first so a refused close still leaves it on the pull request (t3code `Panel:900-917`).
    pub async fn run_change_request_action(
        &self,
        roots: &[PathBuf],
        request: &RunChangeRequestActionRequest,
    ) -> Result<(), ActionError> {
        let identity = self
            .space_identity(roots, &request.host, &request.repository)
            .await?;
        let permissions = self.permissions(&identity, request.number).await?;
        action_allowed(request.action, &permissions).map_err(ActionError::Refused)?;
        let repo = format!("{}/{}", identity.remote.host, identity.name_with_owner());
        let number = request.number.to_string();
        if let Some(comment) = request
            .comment
            .as_deref()
            .filter(|comment| !comment.trim().is_empty())
        {
            self.github
                .run_gh_action(
                    &identity.root,
                    vec![
                        "pr".into(),
                        "comment".into(),
                        number.clone(),
                        "--repo".into(),
                        repo.clone(),
                        "--body-file".into(),
                        "-".into(),
                    ],
                    Some(comment.as_bytes().to_vec()),
                )
                .await?;
        }
        let pr = |args: &[&str]| -> Vec<String> {
            let mut all: Vec<String> = vec!["pr".into()];
            all.push(args[0].into());
            all.push(number.clone());
            all.extend(["--repo".into(), repo.clone()]);
            all.extend(args[1..].iter().map(|arg| (*arg).to_owned()));
            all
        };
        let args = match request.action {
            ChangeRequestAction::Merge { method } => pr(&["merge", merge_flag(method)]),
            ChangeRequestAction::EnableAutoMerge { method } => {
                pr(&["merge", "--auto", merge_flag(method)])
            }
            ChangeRequestAction::DisableAutoMerge => pr(&["merge", "--disable-auto"]),
            ChangeRequestAction::Ready => pr(&["ready"]),
            ChangeRequestAction::Draft => pr(&["ready", "--undo"]),
            ChangeRequestAction::Close => pr(&["close"]),
            ChangeRequestAction::Reopen => pr(&["reopen"]),
            ChangeRequestAction::UpdateBranch { rebase } => {
                if rebase {
                    pr(&["update-branch", "--rebase"])
                } else {
                    pr(&["update-branch"])
                }
            }
            ChangeRequestAction::Revert => {
                let pull = self.pull_request_id(&identity, request.number).await?;
                self.github
                    .graphql_action(
                        &identity,
                        "mutation($pullRequestId: ID!) { revertPullRequest(input: { pullRequestId: $pullRequestId }) { revertPullRequest { id } } }",
                        serde_json::json!({ "pullRequestId": pull }),
                    )
                    .await?;
                return Ok(());
            }
            ChangeRequestAction::ApproveWorkflows => {
                return self.approve_workflows(&identity, request.number).await;
            }
        };
        self.github
            .run_gh_action(&identity.root, args, None)
            .await
            .map(|_| ())
    }

    /// Fork pull requests only: re-reads the checks so the runs approved are the ones waiting
    /// now, not the ones the reader saw.
    async fn approve_workflows(
        &self,
        identity: &RepositoryIdentity,
        number: u64,
    ) -> Result<(), ActionError> {
        let view = self.github.view_pull_request(identity, number).await?;
        if !view.is_cross_repository {
            return Ok(());
        }
        let runs: Vec<String> = view
            .summary
            .status_check_rollup
            .unwrap_or_default()
            .into_iter()
            .filter_map(|check| {
                let url = check.details_url.or(check.target_url)?;
                let state = check
                    .conclusion
                    .or(check.state)
                    .unwrap_or_default()
                    .to_ascii_uppercase();
                (state == "ACTION_REQUIRED").then_some(())?;
                workflow_run_id(&url)
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        for run in runs {
            self.github
                .run_gh_action(
                    &identity.root,
                    vec![
                        "api".into(),
                        "--method".into(),
                        "POST".into(),
                        "--hostname".into(),
                        identity.remote.host.clone(),
                        format!(
                            "repos/{}/actions/runs/{run}/approve",
                            identity.name_with_owner()
                        ),
                        "--silent".into(),
                    ],
                    None,
                )
                .await?;
        }
        Ok(())
    }

    async fn pull_request_id(
        &self,
        identity: &RepositoryIdentity,
        number: u64,
    ) -> Result<String, ChangeRequestError> {
        let raw = self
            .github
            .graphql(
                identity,
                PULL_REQUEST_ID_QUERY,
                activity_variables(identity, number, None),
            )
            .await?;
        let scope: GhScopeResponse =
            serde_json::from_slice(&raw).map_err(|_| ChangeRequestError::Decode)?;
        subject_in_scope(&scope, None).ok_or(ChangeRequestError::Decode)
    }

    /// Title and description edits; an omitted field is left as it is on the host.
    pub async fn update_change_request(
        &self,
        roots: &[PathBuf],
        request: &UpdateChangeRequestRequest,
    ) -> Result<(), ActionError> {
        let title = request.title.as_deref().map(str::trim);
        if title.is_some_and(str::is_empty) {
            return Err(ActionError::Refused("A title cannot be empty.".into()));
        }
        if title.is_none() && request.body.is_none() {
            return Err(ActionError::Refused("Nothing was changed.".into()));
        }
        let identity = self
            .space_identity(roots, &request.host, &request.repository)
            .await?;
        let pull = self.pull_request_id(&identity, request.number).await?;
        let mut variables = serde_json::json!({ "pullRequestId": pull });
        if let Some(title) = title {
            variables["title"] = serde_json::Value::String(title.to_owned());
        }
        if let Some(body) = &request.body {
            variables["body"] = serde_json::Value::String(body.clone());
        }
        self.github
            .graphql_action(
                &identity,
                "mutation($pullRequestId: ID!, $title: String, $body: String) { updatePullRequest(input: { pullRequestId: $pullRequestId, title: $title, body: $body }) { pullRequest { id } } }",
                variables,
            )
            .await
            .map(|_| ())
    }

    pub async fn list_change_request_label_candidates(
        &self,
        roots: &[PathBuf],
        request: &GetChangeRequestRequest,
    ) -> Result<ChangeRequestLabelCandidates, ChangeRequestError> {
        let identity = self
            .space_identity(roots, &request.host, &request.repository)
            .await?;
        let raw = self
            .github
            .graphql(
                &identity,
                LABEL_CANDIDATES_QUERY,
                activity_variables(&identity, request.number, None),
            )
            .await?;
        let response: GhLabelCandidatesResponse =
            serde_json::from_slice(&raw).map_err(|_| ChangeRequestError::Decode)?;
        let repository = response
            .data
            .and_then(|data| data.repository)
            .ok_or(ChangeRequestError::Decode)?;
        Ok(to_label_candidates(repository))
    }

    pub async fn set_change_request_label(
        &self,
        roots: &[PathBuf],
        request: &SetChangeRequestLabelRequest,
    ) -> Result<(), ActionError> {
        let name = request.name.trim();
        if name.is_empty() {
            return Err(ActionError::Refused("A label needs a name.".into()));
        }
        let identity = self
            .space_identity(roots, &request.host, &request.repository)
            .await?;
        let permissions = self.permissions(&identity, request.number).await?;
        if !permissions.can_triage {
            return Err(ActionError::Refused(
                "You need triage access on this repository to change its labels.".into(),
            ));
        }
        let path = format!(
            "repos/{}/issues/{}/labels",
            identity.name_with_owner(),
            request.number
        );
        let mut args: Vec<String> = vec!["api".into(), "--method".into()];
        let input = if request.add {
            args.extend([
                "POST".into(),
                "--hostname".into(),
                identity.remote.host.clone(),
                path,
                "--input".into(),
                "-".into(),
            ]);
            Some(
                serde_json::to_vec(&serde_json::json!({ "labels": [name] }))
                    .map_err(|_| ActionError::Host(ChangeRequestError::Decode))?,
            )
        } else {
            args.extend([
                "DELETE".into(),
                "--hostname".into(),
                identity.remote.host.clone(),
                format!("{path}/{}", percent_encode(name)),
            ]);
            None
        };
        self.github
            .run_gh_action(&identity.root, args, input)
            .await
            .map(|_| ())
    }

    pub async fn list_change_request_reviewer_candidates(
        &self,
        roots: &[PathBuf],
        request: &GetChangeRequestRequest,
    ) -> Result<ChangeRequestReviewerCandidates, ChangeRequestError> {
        let identity = self
            .space_identity(roots, &request.host, &request.repository)
            .await?;
        let raw = self
            .github
            .graphql(
                &identity,
                REVIEWER_CANDIDATES_QUERY,
                activity_variables(&identity, request.number, None),
            )
            .await?;
        let response: GhReviewerCandidatesResponse =
            serde_json::from_slice(&raw).map_err(|_| ChangeRequestError::Decode)?;
        let repository = response
            .data
            .and_then(|data| data.repository)
            .ok_or(ChangeRequestError::Decode)?;
        Ok(to_reviewer_candidates(repository))
    }

    pub async fn set_change_request_reviewer(
        &self,
        roots: &[PathBuf],
        request: &SetChangeRequestReviewerRequest,
    ) -> Result<(), ActionError> {
        if !valid_login(&request.login) {
            return Err(ActionError::Refused(
                "That reviewer name is not valid.".into(),
            ));
        }
        let identity = self
            .space_identity(roots, &request.host, &request.repository)
            .await?;
        let permissions = self.permissions(&identity, request.number).await?;
        if !permissions.can_write {
            return Err(ActionError::Refused(
                "Asking someone to review needs write access on this repository".into(),
            ));
        }
        let body = if request.team {
            serde_json::json!({ "team_reviewers": [request.login] })
        } else {
            serde_json::json!({ "reviewers": [request.login] })
        };
        self.github
            .run_gh_action(
                &identity.root,
                vec![
                    "api".into(),
                    "--method".into(),
                    if request.add { "POST" } else { "DELETE" }.into(),
                    "--hostname".into(),
                    identity.remote.host.clone(),
                    format!(
                        "repos/{}/pulls/{}/requested_reviewers",
                        identity.name_with_owner(),
                        request.number
                    ),
                    "--input".into(),
                    "-".into(),
                ],
                Some(
                    serde_json::to_vec(&body)
                        .map_err(|_| ActionError::Host(ChangeRequestError::Decode))?,
                ),
            )
            .await
            .map(|_| ())
    }

    /// One review with its inline comments; the request body (user text) travels on stdin.
    pub async fn submit_change_request_review(
        &self,
        roots: &[PathBuf],
        request: &SubmitChangeRequestReviewRequest,
    ) -> Result<(), ActionError> {
        if request.event != ChangeRequestReviewEvent::Approve
            && request.body.trim().is_empty()
            && request.comments.is_empty()
        {
            return Err(ActionError::Refused(
                "A review needs a summary or at least one comment.".into(),
            ));
        }
        if request
            .comments
            .iter()
            .any(|comment| comment.body.trim().is_empty() || comment.line == 0)
        {
            return Err(ActionError::Refused(
                "A line comment cannot be empty.".into(),
            ));
        }
        let identity = self
            .space_identity(roots, &request.host, &request.repository)
            .await?;
        let permissions = self.permissions(&identity, request.number).await?;
        if permissions.did_author && request.event != ChangeRequestReviewEvent::Comment {
            return Err(ActionError::Refused(
                "You cannot approve or request changes on your own pull request.".into(),
            ));
        }
        let body = serde_json::json!({
            "event": match request.event {
                ChangeRequestReviewEvent::Comment => "COMMENT",
                ChangeRequestReviewEvent::Approve => "APPROVE",
                ChangeRequestReviewEvent::RequestChanges => "REQUEST_CHANGES",
            },
            "body": request.body,
            "comments": request.comments.iter().map(|comment| serde_json::json!({
                "path": comment.path,
                "line": comment.line,
                "side": if comment.left_side { "LEFT" } else { "RIGHT" },
                "body": comment.body,
            })).collect::<Vec<_>>(),
        });
        self.github
            .run_gh_action(
                &identity.root,
                vec![
                    "api".into(),
                    "--method".into(),
                    "POST".into(),
                    "--hostname".into(),
                    identity.remote.host.clone(),
                    format!(
                        "repos/{}/pulls/{}/reviews",
                        identity.name_with_owner(),
                        request.number
                    ),
                    "--input".into(),
                    "-".into(),
                ],
                Some(
                    serde_json::to_vec(&body)
                        .map_err(|_| ActionError::Host(ChangeRequestError::Decode))?,
                ),
            )
            .await
            .map(|_| ())
    }

    pub async fn reply_to_change_request_thread(
        &self,
        roots: &[PathBuf],
        request: &ReplyToChangeRequestThreadRequest,
    ) -> Result<(), ActionError> {
        if request.body.trim().is_empty() {
            return Err(ActionError::Refused("A reply cannot be empty.".into()));
        }
        let identity = self
            .space_identity(roots, &request.host, &request.repository)
            .await?;
        self.thread_in_scope(&identity, request.number, &request.thread_id)
            .await?;
        self.github
            .graphql_action(
                &identity,
                "mutation($threadId: ID!, $body: String!) { addPullRequestReviewThreadReply(input: { pullRequestReviewThreadId: $threadId, body: $body }) { comment { id } } }",
                serde_json::json!({ "threadId": request.thread_id, "body": request.body }),
            )
            .await
            .map(|_| ())
    }

    pub async fn set_change_request_thread_resolved(
        &self,
        roots: &[PathBuf],
        request: &SetChangeRequestThreadResolvedRequest,
    ) -> Result<(), ActionError> {
        let identity = self
            .space_identity(roots, &request.host, &request.repository)
            .await?;
        let permissions = self.permissions(&identity, request.number).await?;
        if !(permissions.can_write || permissions.did_author) {
            return Err(ActionError::Refused(
                "Resolving a conversation needs write access or authorship of this pull request."
                    .into(),
            ));
        }
        self.thread_in_scope(&identity, request.number, &request.thread_id)
            .await?;
        let mutation = if request.resolved {
            "mutation($threadId: ID!) { resolveReviewThread(input: { threadId: $threadId }) { thread { isResolved } } }"
        } else {
            "mutation($threadId: ID!) { unresolveReviewThread(input: { threadId: $threadId }) { thread { isResolved } } }"
        };
        self.github
            .graphql_action(
                &identity,
                mutation,
                serde_json::json!({ "threadId": request.thread_id }),
            )
            .await
            .map(|_| ())
    }

    /// Where a chat about this pull request runs (t3code `git.preparePullRequestThread`): the
    /// repository as it is, the repository switched to the PR (`gh pr checkout --force`), or a
    /// worktree on the PR's head branch. `space_id` is filled in by the RPC layer.
    pub async fn prepare_change_request_chat(
        &self,
        roots: &[PathBuf],
        worktrees_root: &Path,
        request: &PrepareChangeRequestChatRequest,
    ) -> Result<ChangeRequestChat, ActionError> {
        let identity = self
            .space_identity(roots, &request.host, &request.repository)
            .await?;
        let root = identity.root.to_string_lossy().into_owned();
        match request.checkout {
            ChangeRequestCheckoutMode::None => Ok(ChangeRequestChat {
                root: root.clone(),
                cwd: root,
                ..Default::default()
            }),
            ChangeRequestCheckoutMode::Local => {
                self.github
                    .run_gh_action(
                        &identity.root,
                        vec![
                            "pr".into(),
                            "checkout".into(),
                            request.number.to_string(),
                            "--repo".into(),
                            format!("{}/{}", identity.remote.host, identity.name_with_owner()),
                            "--force".into(),
                        ],
                        None,
                    )
                    .await?;
                let branch = self
                    .inspector
                    .git_run(&identity.root, &["branch", "--show-current"], GIT_TIMEOUT)
                    .await?;
                Ok(ChangeRequestChat {
                    root: root.clone(),
                    cwd: root,
                    branch: (!branch.is_empty()).then_some(branch),
                    worktree: false,
                    on_head: true,
                    space_id: None,
                })
            }
            ChangeRequestCheckoutMode::Worktree => {
                self.prepare_worktree(&identity, worktrees_root, request.number)
                    .await
            }
        }
    }

    async fn prepare_worktree(
        &self,
        identity: &RepositoryIdentity,
        worktrees_root: &Path,
        number: u64,
    ) -> Result<ChangeRequestChat, ActionError> {
        let view = self.github.view_pull_request(identity, number).await?;
        let head = view.summary.head_ref_name.clone();
        let branch = if view.is_cross_repository {
            format!("pr-{number}")
        } else {
            head
        };
        if !valid_branch_name(&branch) {
            return Err(ActionError::Refused(
                "This pull request's branch name cannot be checked out safely.".into(),
            ));
        }
        let git = &self.inspector;
        let root = &identity.root;
        let remotes = git.git_run(root, &["remote", "-v"], GIT_TIMEOUT).await?;
        let remote = primary_remote(&remotes)
            .map(|(name, _)| name.to_owned())
            .ok_or(ActionError::Host(ChangeRequestError::UnsupportedRepository))?;
        git.git_run(
            root,
            &[
                "fetch",
                "--quiet",
                &remote,
                &format!("refs/pull/{number}/head"),
            ],
            GIT_FETCH_TIMEOUT,
        )
        .await?;
        let head_oid = git
            .git_run(root, &["rev-parse", "FETCH_HEAD"], GIT_TIMEOUT)
            .await?;
        let listing = git
            .git_run(root, &["worktree", "list", "--porcelain"], GIT_TIMEOUT)
            .await?;
        let worktrees = parse_worktrees(&listing);
        let branch_ref = format!("refs/heads/{branch}");
        if let Some((path, _)) = worktrees.iter().find(|(_, name)| *name == branch_ref) {
            if Path::new(path) == root.as_path() {
                return Err(ActionError::Refused(
                    "This PR branch is already checked out in the main repo. Use Local, or switch the main repo off that branch before creating a worktree thread.".into(),
                ));
            }
            let path = PathBuf::from(path);
            let clean = git
                .git_run(&path, &["status", "--porcelain"], GIT_TIMEOUT)
                .await?
                .is_empty();
            let behind = git
                .git_run(
                    &path,
                    &["merge-base", "--is-ancestor", "HEAD", &head_oid],
                    GIT_TIMEOUT,
                )
                .await
                .is_ok();
            if clean && behind {
                git.git_run(
                    &path,
                    &["merge", "--ff-only", "--quiet", &head_oid],
                    GIT_TIMEOUT,
                )
                .await?;
            }
            let current = git
                .git_run(&path, &["rev-parse", "HEAD"], GIT_TIMEOUT)
                .await?;
            return Ok(ChangeRequestChat {
                cwd: path.to_string_lossy().into_owned(),
                branch: Some(branch),
                worktree: true,
                root: root.to_string_lossy().into_owned(),
                on_head: current == head_oid,
                space_id: None,
            });
        }
        let existing = git
            .git_run(
                root,
                &["rev-parse", "--verify", "--quiet", &branch_ref],
                GIT_TIMEOUT,
            )
            .await
            .ok();
        match existing {
            None => {
                git.git_run(root, &["branch", "--", &branch, &head_oid], GIT_TIMEOUT)
                    .await?;
            }
            Some(oid) if oid != head_oid => {
                let fast_forward = git
                    .git_run(
                        root,
                        &["merge-base", "--is-ancestor", &oid, &head_oid],
                        GIT_TIMEOUT,
                    )
                    .await
                    .is_ok();
                if fast_forward {
                    git.git_run(
                        root,
                        &["branch", "-f", "--", &branch, &head_oid],
                        GIT_TIMEOUT,
                    )
                    .await?;
                }
            }
            Some(_) => {}
        }
        let repo_name = root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "repo".into());
        let base = worktrees_root.join(repo_name);
        std::fs::create_dir_all(&base)
            .map_err(|_| ActionError::Host(ChangeRequestError::RepositoryUnavailable))?;
        let mut path = base.join(format!("pr-{number}"));
        let mut suffix = 2;
        while path.exists() {
            path = base.join(format!("pr-{number}-{suffix}"));
            suffix += 1;
        }
        let path_text = path.to_string_lossy().into_owned();
        git.git_run(
            root,
            &["worktree", "add", "--quiet", &path_text, &branch],
            GIT_FETCH_TIMEOUT,
        )
        .await?;
        let current = git
            .git_run(&path, &["rev-parse", "HEAD"], GIT_TIMEOUT)
            .await?;
        Ok(ChangeRequestChat {
            cwd: path_text,
            branch: Some(branch),
            worktree: true,
            root: root.to_string_lossy().into_owned(),
            on_head: current == head_oid,
            space_id: None,
        })
    }

    /// A client-supplied thread id must name a review thread of this pull request.
    async fn thread_in_scope(
        &self,
        identity: &RepositoryIdentity,
        number: u64,
        thread: &str,
    ) -> Result<(), ActionError> {
        let mut variables = activity_variables(identity, number, None);
        variables["subjectId"] = serde_json::Value::String(thread.to_owned());
        let raw = self
            .github
            .graphql(identity, THREAD_SCOPE_QUERY, variables)
            .await?;
        let scope: GhScopeResponse = serde_json::from_slice(&raw)
            .map_err(|_| ActionError::Host(ChangeRequestError::Decode))?;
        subject_in_scope(&scope, Some(thread))
            .map(|_| ())
            .ok_or(ActionError::Host(ChangeRequestError::UnsupportedRepository))
    }

    pub async fn create_change_request(
        &self,
        root: &Path,
        request: &CreateChangeRequestRequest,
    ) -> Result<CreatedChangeRequest, ActionError> {
        let title = request.title.trim();
        if title.is_empty() {
            return Err(ActionError::Refused(
                "Give the pull request a title.".into(),
            ));
        }
        if title.chars().count() > 256 || request.body.len() > 64 * 1024 {
            return Err(ActionError::Refused(
                "The pull request title or description is too long.".into(),
            ));
        }
        let ref_name = |name: &str| {
            !name.is_empty()
                && !name.starts_with('-')
                && !name.chars().any(|c| c.is_whitespace() || c.is_control())
        };
        let base = request.base.as_deref().filter(|base| !base.is_empty());
        if !ref_name(&request.head) || base.is_some_and(|base| !ref_name(base)) {
            return Err(ActionError::Refused("That is not a branch name.".into()));
        }
        let identity = self
            .space_identity(&[root.to_path_buf()], &request.host, &request.repository)
            .await?;
        let repo = format!("{}/{}", identity.remote.host, identity.name_with_owner());
        let mut args: Vec<String> = vec![
            "pr".into(),
            "create".into(),
            "--repo".into(),
            repo,
            format!("--head={}", request.head),
        ];
        if let Some(base) = base {
            args.push(format!("--base={base}"));
        }
        args.extend([format!("--title={title}"), "--body-file".into(), "-".into()]);
        if request.draft {
            args.push("--draft".into());
        }
        let stdout = self
            .github
            .run_gh_action(root, args, Some(request.body.clone().into_bytes()))
            .await?;
        let url = String::from_utf8_lossy(&stdout)
            .lines()
            .map(str::trim)
            .rfind(|line| line.starts_with("https://"))
            .map(str::to_owned)
            .ok_or(ActionError::Host(ChangeRequestError::Decode))?;
        let number = url
            .trim_end_matches('/')
            .rsplit_once("/pull/")
            .and_then(|(_, number)| number.parse().ok())
            .ok_or(ActionError::Host(ChangeRequestError::Decode))?;
        Ok(CreatedChangeRequest {
            host: identity.remote.host.clone(),
            repository: identity.name_with_owner(),
            number,
            url,
        })
    }

    async fn space_identity(
        &self,
        roots: &[PathBuf],
        host: &str,
        repository: &str,
    ) -> Result<RepositoryIdentity, ChangeRequestError> {
        let identities = futures::future::join_all(
            roots
                .iter()
                .map(|root| self.inspector.repository_identity(root)),
        )
        .await;
        identities
            .into_iter()
            .filter_map(Result::ok)
            .find(|identity| {
                identity.remote.host.eq_ignore_ascii_case(host)
                    && identity.name_with_owner().eq_ignore_ascii_case(repository)
            })
            .ok_or(ChangeRequestError::UnsupportedRepository)
    }
}

/// Remote choice copies t3code: `upstream`, else `origin`, else the first fetch remote by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryIdentity {
    pub root: PathBuf,
    pub remote: GitRemote,
}

impl RepositoryIdentity {
    fn name_with_owner(&self) -> String {
        format!("{}/{}", self.remote.owner, self.remote.repository)
    }

    fn list_error(&self, failure: ChangeRequestFailure) -> ChangeRequestListError {
        ChangeRequestListError {
            host: Some(self.remote.host.clone()),
            repository: Some(self.name_with_owner()),
            path: self.root.to_string_lossy().into_owned(),
            failure,
        }
    }
}

impl GitRemote {
    fn same_repository(&self, other: &GitRemote) -> bool {
        self.host.eq_ignore_ascii_case(&other.host)
            && self.owner.eq_ignore_ascii_case(&other.owner)
            && self.repository.eq_ignore_ascii_case(&other.repository)
    }
}

impl From<ChangeRequestError> for ChangeRequestFailure {
    fn from(error: ChangeRequestError) -> Self {
        match error {
            ChangeRequestError::RepositoryUnavailable => Self::RepositoryUnavailable,
            ChangeRequestError::UnsupportedRepository => Self::UnsupportedRepository,
            ChangeRequestError::CliUnavailable => Self::CliUnavailable,
            ChangeRequestError::Authentication => Self::Authentication,
            ChangeRequestError::RateLimited => Self::RateLimited,
            ChangeRequestError::Timeout => Self::Timeout,
            ChangeRequestError::Decode => Self::Decode,
            ChangeRequestError::CommandFailed => Self::CommandFailed,
        }
    }
}

#[async_trait]
impl CheckoutChangeRequestLookup for ChangeRequestResolver {
    async fn inspect_checkout(
        &self,
        cwd: &Path,
    ) -> Result<CheckoutSourceContext, ChangeRequestError> {
        ChangeRequestResolver::inspect_checkout(self, cwd).await
    }

    async fn resolve_github_source(
        &self,
        source: &CheckoutSourceContext,
    ) -> Result<Option<ChangeRequestSummary>, ChangeRequestError> {
        ChangeRequestResolver::resolve_github_source(self, source).await
    }
}

const GH_CONCURRENCY: usize = 12;
const RATE_LIMIT_FIRST_PAUSE: Duration = Duration::from_secs(30);
const RATE_LIMIT_MAX_PAUSE: Duration = Duration::from_secs(15 * 60);

static GH_SLOTS: std::sync::LazyLock<tokio::sync::Semaphore> =
    std::sync::LazyLock::new(|| tokio::sync::Semaphore::new(GH_CONCURRENCY));
static SHARED_RATE_LIMITS: std::sync::LazyLock<Arc<RateLimits>> =
    std::sync::LazyLock::new(|| Arc::new(RateLimits::default()));

fn rate_limit_pause(attempt: u32) -> Duration {
    RATE_LIMIT_FIRST_PAUSE
        .saturating_mul(2u32.saturating_pow(attempt.saturating_sub(1)))
        .min(RATE_LIMIT_MAX_PAUSE)
}

// Writes bypass the pause so a user's own action is never silently dropped.
#[derive(Default)]
struct RateLimits {
    hosts: std::sync::Mutex<std::collections::HashMap<String, (u32, std::time::Instant)>>,
}

impl RateLimits {
    fn paused(&self, host: &str) -> bool {
        self.hosts
            .lock()
            .ok()
            .and_then(|hosts| {
                hosts
                    .get(host)
                    .map(|(_, until)| *until > std::time::Instant::now())
            })
            .unwrap_or(false)
    }

    fn record(&self, host: &str, result: &Result<Vec<u8>, ChangeRequestError>) {
        let Ok(mut hosts) = self.hosts.lock() else {
            return;
        };
        match result {
            Err(ChangeRequestError::RateLimited) => {
                let attempt = hosts.get(host).map_or(0, |(attempt, _)| *attempt) + 1;
                hosts.insert(
                    host.to_owned(),
                    (
                        attempt,
                        std::time::Instant::now() + rate_limit_pause(attempt),
                    ),
                );
            }
            Ok(_) => {
                hosts.remove(host);
            }
            Err(_) => {}
        }
    }
}

fn host_of_args(args: &[String]) -> String {
    let after = |flag: &str| {
        args.iter()
            .position(|arg| arg == flag)
            .and_then(|at| args.get(at + 1))
    };
    after("--hostname")
        .cloned()
        .or_else(|| {
            after("--repo")
                .filter(|repo| repo.matches('/').count() >= 2)
                .and_then(|repo| repo.split('/').next().map(str::to_owned))
        })
        .unwrap_or_else(|| "github.com".into())
        .to_ascii_lowercase()
}

/// GitHub provider backed by the host's existing `gh` installation and auth.
#[derive(Clone)]
pub struct GitHubCli {
    runner: Arc<dyn ProcessRunner>,
    rate_limits: Arc<RateLimits>,
}

impl GitHubCli {
    pub fn new() -> Self {
        Self {
            runner: Arc::new(SystemProcessRunner),
            rate_limits: SHARED_RATE_LIMITS.clone(),
        }
    }

    fn with_runner(runner: Arc<dyn ProcessRunner>) -> Self {
        Self {
            runner,
            rate_limits: Arc::new(RateLimits::default()),
        }
    }

    async fn list_for_selector(
        &self,
        source: &CheckoutSourceContext,
        selector: &str,
    ) -> Result<Vec<GhPullRequest>, ChangeRequestError> {
        let request = ProcessRequest {
            program: "gh".into(),
            args: vec![
                "pr".into(),
                "list".into(),
                "--head".into(),
                selector.into(),
                "--state".into(),
                "all".into(),
                "--limit".into(),
                GITHUB_RESULT_LIMIT.into(),
                "--json".into(),
                GITHUB_JSON_FIELDS.into(),
            ],
            cwd: source.checkout_root.clone(),
            env: vec![("GH_PROMPT_DISABLED".into(), "1".into())],
            timeout: GITHUB_TIMEOUT,
            output_limit: GITHUB_OUTPUT_LIMIT,
            stdin: None,
        };
        let output = self.runner.run(request).await.map_err(classify_run_error)?;
        if !output.success {
            return Err(classify_github_failure(&output.stderr));
        }
        if output.stdout_truncated {
            return Err(ChangeRequestError::Decode);
        }
        serde_json::from_slice(&output.stdout).map_err(|_| ChangeRequestError::Decode)
    }

    /// Resolve the remote's default branch through the provider API.
    ///
    /// This deliberately never uses Git remote transport (`git ls-remote`)
    /// against the watched checkout, which would honor repository-controlled
    /// transport and credential configuration. `gh` queries the
    /// already-sanitized host/owner/repository identity with its own
    /// credentials. Failures degrade to "unknown", matching the pre-existing
    /// behavior when a default branch cannot be determined.
    async fn default_branch(&self, source: &CheckoutSourceContext) -> Option<String> {
        let host = source.branch.host.as_deref()?;
        let owner = source.branch.owner.as_deref()?;
        let repository = source.branch.repository.as_deref()?;
        let request = ProcessRequest {
            program: "gh".into(),
            args: vec![
                "repo".into(),
                "view".into(),
                format!("{host}/{owner}/{repository}"),
                "--json".into(),
                "defaultBranchRef".into(),
            ],
            cwd: source.checkout_root.clone(),
            env: vec![("GH_PROMPT_DISABLED".into(), "1".into())],
            timeout: GITHUB_TIMEOUT,
            output_limit: GITHUB_OUTPUT_LIMIT,
            stdin: None,
        };
        let output = self.runner.run(request).await.ok()?;
        if !output.success || output.stdout_truncated {
            return None;
        }
        let view: GhRepoView = serde_json::from_slice(&output.stdout).ok()?;
        view.default_branch_ref
            .map(|reference| reference.name)
            .filter(|name| !name.is_empty())
    }

    async fn run_gh(
        &self,
        cwd: &Path,
        args: Vec<String>,
        timeout: Duration,
        output_limit: usize,
    ) -> Result<Vec<u8>, ChangeRequestError> {
        self.run_gh_input(cwd, args, timeout, output_limit, None)
            .await
    }

    /// User-authored text (comment bodies, GraphQL variables) travels on stdin, never argv.
    async fn run_gh_input(
        &self,
        cwd: &Path,
        args: Vec<String>,
        timeout: Duration,
        output_limit: usize,
        stdin: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, ChangeRequestError> {
        let host = host_of_args(&args);
        if self.rate_limits.paused(&host) {
            return Err(ChangeRequestError::RateLimited);
        }
        let request = ProcessRequest {
            program: "gh".into(),
            args,
            cwd: cwd.to_owned(),
            env: vec![("GH_PROMPT_DISABLED".into(), "1".into())],
            timeout,
            output_limit,
            stdin,
        };
        let result = async {
            let _slot = GH_SLOTS.acquire().await;
            let output = self.runner.run(request).await.map_err(classify_run_error)?;
            if !output.success {
                return Err(classify_github_failure(&output.stderr));
            }
            if output.stdout_truncated {
                return Err(ChangeRequestError::Decode);
            }
            Ok(output.stdout)
        }
        .await;
        self.rate_limits.record(&host, &result);
        result
    }

    async fn viewer_login(&self, host: &str, cwd: &Path) -> Result<String, ChangeRequestError> {
        let stdout = self
            .run_gh(
                cwd,
                ["api", "--hostname", host, "user", "--jq", ".login"]
                    .map(str::to_owned)
                    .to_vec(),
                GITHUB_TIMEOUT,
                GITHUB_OUTPUT_LIMIT,
            )
            .await?;
        let login = String::from_utf8(stdout)
            .map_err(|_| ChangeRequestError::Decode)?
            .trim()
            .to_owned();
        if login.is_empty() {
            return Err(ChangeRequestError::Authentication);
        }
        Ok(login)
    }

    async fn list_repository(
        &self,
        identity: &RepositoryIdentity,
        state: ChangeRequestListState,
        author: Option<&str>,
        search: Option<&str>,
        limit: u32,
    ) -> Result<(Vec<GhListPullRequest>, usize), ChangeRequestError> {
        let state = match state {
            ChangeRequestListState::Open => "open",
            ChangeRequestListState::Closed => "closed",
            ChangeRequestListState::Merged => "merged",
            ChangeRequestListState::All => "all",
        };
        let mut args: Vec<String> = vec![
            "pr".into(),
            "list".into(),
            "--repo".into(),
            format!("{}/{}", identity.remote.host, identity.name_with_owner()),
        ];
        if let Some(author) = author {
            args.extend(["--author".into(), author.to_owned()]);
        }
        if let Some(search) = search {
            args.extend(["--search".into(), search.to_owned()]);
        }
        args.extend([
            "--state".into(),
            state.into(),
            "--limit".into(),
            (limit + 1).to_string(),
            "--json".into(),
            GITHUB_LIST_FIELDS.into(),
        ]);
        let stdout = self
            .run_gh(
                &identity.root,
                args,
                GITHUB_LIST_TIMEOUT,
                GITHUB_LIST_OUTPUT_LIMIT,
            )
            .await?;
        let rows: Vec<serde_json::Value> =
            serde_json::from_slice(&stdout).map_err(|_| ChangeRequestError::Decode)?;
        let raw_count = rows.len();
        let rows = rows
            .into_iter()
            .filter_map(|row| serde_json::from_value(row).ok())
            .collect();
        Ok((rows, raw_count))
    }

    async fn list_repository_items(
        &self,
        identity: &RepositoryIdentity,
        request: &ListChangeRequestsRequest,
        viewer: &str,
        limit: u32,
    ) -> Result<(Vec<ChangeRequestListItem>, bool), ChangeRequestError> {
        let mut feed_terms = vec!["sort:updated-desc"];
        if request.state == ChangeRequestListState::Closed {
            feed_terms.insert(0, "is:unmerged");
        }
        let feed_search = feed_terms.join(" ");
        let reviewing_search = format!("review-requested:{viewer} sort:updated-desc");
        let (feed, authored, reviewing) = match request.involvement {
            ChangeRequestInvolvement::All => {
                let (feed, authored, reviewing) = futures::try_join!(
                    self.list_feed(identity, request.state, &feed_search, limit),
                    self.list_repository(
                        identity,
                        request.state,
                        Some(viewer),
                        Some(&feed_search),
                        limit
                    ),
                    self.list_repository(
                        identity,
                        request.state,
                        None,
                        Some(&reviewing_search),
                        limit
                    ),
                )?;
                (feed, authored.0, reviewing.0)
            }
            ChangeRequestInvolvement::Authored => (
                self.list_repository(
                    identity,
                    request.state,
                    Some(viewer),
                    Some(&feed_search),
                    limit,
                )
                .await?,
                Vec::new(),
                Vec::new(),
            ),
            ChangeRequestInvolvement::Reviewing => (
                self.list_repository(
                    identity,
                    request.state,
                    None,
                    Some(&reviewing_search),
                    limit,
                )
                .await?,
                Vec::new(),
                Vec::new(),
            ),
        };
        let (feed, raw_count) = feed;
        let truncated = raw_count > limit as usize;
        let review_requested: Vec<u64> = reviewing.iter().map(|row| row.number).collect();
        let mut items: Vec<ChangeRequestListItem> = Vec::new();
        for row in feed
            .into_iter()
            .take(limit as usize)
            .chain(authored.into_iter().take(limit as usize))
            .chain(reviewing.into_iter().take(limit as usize))
        {
            if items.iter().any(|item| item.number == row.number) {
                continue;
            }
            let requested = review_requested.contains(&row.number);
            if let Some(mut item) = to_list_item(identity, viewer, row)
                .filter(|item| matches_filters(item, &request.filters, viewer))
            {
                if requested
                    && request.involvement == ChangeRequestInvolvement::All
                    && !item
                        .author
                        .as_ref()
                        .is_some_and(|author| author.login.eq_ignore_ascii_case(viewer))
                {
                    item.viewer_review_requested = true;
                }
                items.push(item);
            }
        }
        Ok((items, truncated))
    }

    async fn view_pull_request(
        &self,
        identity: &RepositoryIdentity,
        number: u64,
    ) -> Result<GhDetailPullRequest, ChangeRequestError> {
        let stdout = self
            .run_gh(
                &identity.root,
                vec![
                    "pr".into(),
                    "view".into(),
                    number.to_string(),
                    "--repo".into(),
                    format!("{}/{}", identity.remote.host, identity.name_with_owner()),
                    "--json".into(),
                    GITHUB_DETAIL_FIELDS.into(),
                ],
                GITHUB_LIST_TIMEOUT,
                GITHUB_LIST_OUTPUT_LIMIT,
            )
            .await?;
        serde_json::from_slice(&stdout).map_err(|_| ChangeRequestError::Decode)
    }

    /// A write: the host's own refusal comes back in words rather than as a bare failure class.
    async fn run_gh_action(
        &self,
        cwd: &Path,
        args: Vec<String>,
        stdin: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, ActionError> {
        let request = ProcessRequest {
            program: "gh".into(),
            args,
            cwd: cwd.to_owned(),
            env: vec![("GH_PROMPT_DISABLED".into(), "1".into())],
            timeout: GITHUB_LIST_TIMEOUT,
            output_limit: GITHUB_OUTPUT_LIMIT,
            stdin,
        };
        let output = {
            let _slot = GH_SLOTS.acquire().await;
            self.runner.run(request).await.map_err(classify_run_error)?
        };
        if output.success {
            return Ok(output.stdout);
        }
        match classify_github_failure(&output.stderr) {
            ChangeRequestError::CommandFailed => {
                Err(ActionError::Refused(readable_failure(&output.stderr)))
            }
            other => Err(ActionError::Host(other)),
        }
    }

    async fn graphql_action(
        &self,
        identity: &RepositoryIdentity,
        query: &str,
        variables: serde_json::Value,
    ) -> Result<Vec<u8>, ActionError> {
        let input = serde_json::to_vec(&serde_json::json!({
            "query": query,
            "variables": variables,
        }))
        .map_err(|_| ActionError::Host(ChangeRequestError::Decode))?;
        let stdout = self
            .run_gh_action(
                &identity.root,
                vec![
                    "api".into(),
                    "graphql".into(),
                    "--hostname".into(),
                    identity.remote.host.clone(),
                    "--input".into(),
                    "-".into(),
                ],
                Some(input),
            )
            .await?;
        let errors: GhGraphqlErrors = serde_json::from_slice(&stdout).unwrap_or_default();
        if let Some(error) = errors.errors.first() {
            let message = error["message"].as_str().unwrap_or_default();
            return Err(ActionError::Refused(readable_failure(message.as_bytes())));
        }
        Ok(stdout)
    }

    async fn graphql(
        &self,
        identity: &RepositoryIdentity,
        query: &str,
        variables: serde_json::Value,
    ) -> Result<Vec<u8>, ChangeRequestError> {
        let input = serde_json::to_vec(&serde_json::json!({
            "query": query,
            "variables": variables,
        }))
        .map_err(|_| ChangeRequestError::Decode)?;
        let stdout = self
            .run_gh_input(
                &identity.root,
                vec![
                    "api".into(),
                    "graphql".into(),
                    "--hostname".into(),
                    identity.remote.host.clone(),
                    "--input".into(),
                    "-".into(),
                ],
                GITHUB_LIST_TIMEOUT,
                GITHUB_LIST_OUTPUT_LIMIT,
                Some(input),
            )
            .await?;
        let errors: GhGraphqlErrors = serde_json::from_slice(&stdout).unwrap_or_default();
        if !errors.errors.is_empty() {
            return Err(ChangeRequestError::Decode);
        }
        Ok(stdout)
    }

    async fn diff_files_page(
        &self,
        identity: &RepositoryIdentity,
        number: u64,
        page: u32,
        commit: Option<&str>,
    ) -> Result<Vec<u8>, ChangeRequestError> {
        let repository = identity.name_with_owner();
        if !valid_repository_path(&repository) {
            return Err(ChangeRequestError::UnsupportedRepository);
        }
        let paging = format!("per_page={DIFF_FILES_PAGE_SIZE}&page={page}");
        let mut args: Vec<String> = vec![
            "api".into(),
            "--hostname".into(),
            identity.remote.host.clone(),
        ];
        match commit {
            None => args.push(format!("repos/{repository}/pulls/{number}/files?{paging}")),
            Some(commit) => args.extend([
                format!("repos/{repository}/commits/{commit}?{paging}"),
                "--jq".into(),
                ".files // []".into(),
            ]),
        }
        self.run_gh(
            &identity.root,
            args,
            GITHUB_DIFF_TIMEOUT,
            GITHUB_LIST_OUTPUT_LIMIT,
        )
        .await
    }

    async fn search(
        &self,
        root: &Path,
        host: &str,
        query: String,
        rows: u32,
    ) -> Result<(Vec<GhSearchPullRequest>, usize, bool), ChangeRequestError> {
        let mut nodes = Vec::new();
        let mut cursor: Option<String> = None;
        let mut has_next = false;
        while nodes.len() < rows as usize {
            let first = (rows as usize - nodes.len()).min(100);
            let document = format!(
                "query($q: String!, $after: String) {{ search(query: $q, type: ISSUE, first: {first}, after: $after) {{ pageInfo {{ hasNextPage endCursor }} nodes {{ ... on PullRequest {{ {GITHUB_SEARCH_FIELDS} }} }} }} }}"
            );
            let mut args = vec![
                "api".into(),
                "graphql".into(),
                "--hostname".into(),
                host.into(),
                "-f".into(),
                format!("q={query}"),
                "-f".into(),
                format!("query={document}"),
            ];
            if let Some(cursor) = &cursor {
                args.extend(["-f".into(), format!("after={cursor}")]);
            }
            let stdout = self
                .run_gh(root, args, GITHUB_LIST_TIMEOUT, GITHUB_LIST_OUTPUT_LIMIT)
                .await?;
            let response: GhSearchResponse =
                serde_json::from_slice(&stdout).map_err(|_| ChangeRequestError::Decode)?;
            let search = response
                .data
                .ok_or(ChangeRequestError::CommandFailed)?
                .search;
            nodes.extend(search.nodes);
            has_next = search.page_info.has_next_page;
            cursor = search.page_info.end_cursor.filter(|_| has_next);
            if cursor.is_none() {
                break;
            }
        }
        let raw_count = nodes.len();
        let rows = nodes
            .into_iter()
            .filter_map(|node| serde_json::from_value(node).ok())
            .collect();
        Ok((rows, raw_count, has_next))
    }

    async fn search_host_items(
        &self,
        group: &[&RepositoryIdentity],
        request: &ListChangeRequestsRequest,
        viewer: &str,
        limit: u32,
    ) -> Result<Option<(Vec<ChangeRequestListItem>, bool)>, ChangeRequestError> {
        if group
            .iter()
            .any(|identity| !valid_repository_path(&identity.name_with_owner()))
            || !valid_login(viewer)
        {
            return Ok(None);
        }
        let host = &group[0].remote.host;
        let root = &group[0].root;
        let repos = group
            .iter()
            .map(|identity| format!("repo:{}", identity.name_with_owner()))
            .collect::<Vec<_>>()
            .join(" ");
        let state = match request.state {
            ChangeRequestListState::Open => "is:open",
            ChangeRequestListState::Closed => "is:closed is:unmerged",
            ChangeRequestListState::Merged => "is:merged",
            ChangeRequestListState::All => "",
        };
        let text = request
            .query
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_owned();
        let phrase = if text.is_empty() {
            String::new()
        } else {
            format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
        };
        let qualifiers = filter_qualifiers(&request.filters, viewer).join(" ");
        let query = |extra: &str| {
            [
                format!("is:pr {state}"),
                extra.to_owned(),
                phrase.clone(),
                qualifiers.clone(),
                "sort:updated-desc".into(),
                repos.clone(),
            ]
            .into_iter()
            .filter(|part| !part.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" ")
        };
        let rows = limit + 1;
        let authored = format!("author:{viewer}");
        let reviewing = format!("review-requested:{viewer}");
        let empty = || async { Ok((Vec::new(), 0, false)) };
        let ((feed, raw_count, has_next), (authored_rows, _, _), (reviewing_rows, _, _)) =
            match request.involvement {
                ChangeRequestInvolvement::All => futures::try_join!(
                    self.search(root, host, query(""), rows),
                    self.search(root, host, query(&authored), rows),
                    self.search(root, host, query(&reviewing), rows),
                )?,
                ChangeRequestInvolvement::Authored => futures::try_join!(
                    self.search(root, host, query(&authored), rows),
                    empty(),
                    empty(),
                )?,
                ChangeRequestInvolvement::Reviewing => futures::try_join!(
                    self.search(root, host, query(&reviewing), rows),
                    empty(),
                    empty(),
                )?,
            };
        if raw_count == 0 && request.involvement == ChangeRequestInvolvement::All && text.is_empty()
        {
            return Ok(None);
        }
        let review_requested: Vec<(String, u64)> = reviewing_rows
            .iter()
            .map(|row| {
                (
                    row.repository.name_with_owner.to_ascii_lowercase(),
                    row.number,
                )
            })
            .collect();
        let mut items: Vec<ChangeRequestListItem> = Vec::new();
        for row in feed
            .into_iter()
            .take(limit as usize)
            .chain(authored_rows.into_iter().take(limit as usize))
            .chain(reviewing_rows.into_iter().take(limit as usize))
        {
            let Some(identity) = group.iter().find(|identity| {
                identity
                    .name_with_owner()
                    .eq_ignore_ascii_case(&row.repository.name_with_owner)
            }) else {
                continue;
            };
            let key = (
                row.repository.name_with_owner.to_ascii_lowercase(),
                row.number,
            );
            if items
                .iter()
                .any(|item| item.number == key.1 && item.repository.eq_ignore_ascii_case(&key.0))
            {
                continue;
            }
            let requested = review_requested.contains(&key);
            if let Some(mut item) = to_list_item(identity, viewer, row.into_list_row()) {
                if requested
                    && !item
                        .author
                        .as_ref()
                        .is_some_and(|author| author.login.eq_ignore_ascii_case(viewer))
                {
                    item.viewer_review_requested = true;
                }
                items.push(item);
            }
        }
        Ok(Some((items, raw_count > limit as usize || has_next)))
    }

    async fn pull_request_stats(
        &self,
        identity: &RepositoryIdentity,
        refs: &[&ChangeRequestRef],
    ) -> Vec<ChangeRequestStat> {
        let fields = refs
            .iter()
            .enumerate()
            .filter_map(|(index, reference)| {
                let (owner, name) = reference.repository.split_once('/')?;
                Some(format!(
                    "s{index}: repository(owner: \"{owner}\", name: \"{name}\") {{ pullRequest(number: {}) {{ additions deletions }} }}",
                    reference.number
                ))
            })
            .collect::<Vec<_>>()
            .join(" ");
        let Ok(stdout) = self
            .run_gh(
                &identity.root,
                vec![
                    "api".into(),
                    "graphql".into(),
                    "--hostname".into(),
                    identity.remote.host.clone(),
                    "-f".into(),
                    format!("query=query {{ {fields} }}"),
                ],
                GITHUB_LIST_TIMEOUT,
                GITHUB_OUTPUT_LIMIT,
            )
            .await
        else {
            return Vec::new();
        };
        let Ok(response) = serde_json::from_slice::<serde_json::Value>(&stdout) else {
            return Vec::new();
        };
        refs.iter()
            .enumerate()
            .filter_map(|(index, reference)| {
                let pull_request = response.pointer(&format!("/data/s{index}/pullRequest"))?;
                Some(ChangeRequestStat {
                    reference: (*reference).clone(),
                    additions: pull_request.get("additions")?.as_u64()?,
                    deletions: pull_request.get("deletions")?.as_u64()?,
                })
            })
            .collect()
    }

    async fn list_feed(
        &self,
        identity: &RepositoryIdentity,
        state: ChangeRequestListState,
        search: &str,
        limit: u32,
    ) -> Result<(Vec<GhListPullRequest>, usize), ChangeRequestError> {
        let (rows, raw_count) = self
            .list_repository(identity, state, None, Some(search), limit)
            .await?;
        if raw_count > 0 {
            return Ok((rows, raw_count));
        }
        let (rows, raw_count) = self
            .list_repository(identity, state, None, None, limit)
            .await?;
        let rows = rows
            .into_iter()
            .filter(|row| {
                state != ChangeRequestListState::Closed
                    || gh_list_state(row) != ChangeRequestState::Merged
            })
            .collect();
        Ok((rows, raw_count))
    }
}

impl Default for GitHubCli {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ChangeRequestProvider for GitHubCli {
    fn provider(&self) -> &'static str {
        "github"
    }

    async fn find_for_branch(
        &self,
        source: &CheckoutSourceContext,
    ) -> Result<Option<ChangeRequestSummary>, ChangeRequestError> {
        if source.branch.host.is_none()
            || source.branch.owner.is_none()
            || source.branch.repository.is_none()
        {
            return Err(ChangeRequestError::UnsupportedRepository);
        }
        if source.branch.head_selectors.is_empty() {
            return Ok(None);
        }

        for selector in &source.branch.head_selectors {
            let candidates = self.list_for_selector(source, selector).await?;
            if candidates.is_empty() {
                continue;
            }
            let default_branch = match source.default_branch.clone() {
                Some(branch) => Some(branch),
                None if needs_default_branch(source, &candidates) => {
                    self.default_branch(source).await
                }
                None => None,
            };
            return select_pull_request(
                self.provider(),
                source,
                default_branch.as_deref(),
                candidates,
            );
        }
        Ok(None)
    }
}

impl BranchHeadContext {
    /// Build provider head selectors from already-inspected Git branch metadata.
    pub fn resolve(
        local_branch: impl Into<String>,
        upstream_ref: Option<&str>,
        remote_name: Option<&str>,
        remote_url: Option<&str>,
    ) -> Self {
        let local_branch = local_branch.into();
        let upstream_ref = non_empty(upstream_ref).map(str::to_owned);
        let remote_name = non_empty(remote_name)
            .map(str::to_owned)
            .or_else(|| upstream_ref.as_deref().and_then(remote_from_upstream));
        let remote_url = non_empty(remote_url).map(str::to_owned);
        let parsed_remote = remote_url.as_deref().and_then(parse_git_remote);
        let upstream_branch = upstream_ref
            .as_deref()
            .and_then(|upstream| branch_from_upstream(upstream, remote_name.as_deref()));
        let head_branch = upstream_branch.unwrap_or(&local_branch).to_owned();

        let mut head_selectors = Vec::new();
        if let Some(remote) = parsed_remote.as_ref() {
            push_unique(
                &mut head_selectors,
                format!("{}:{head_branch}", remote.owner),
            );
        }
        if upstream_ref.is_some() {
            push_unique(&mut head_selectors, head_branch.clone());
        }
        if upstream_ref.is_none() || local_branch == head_branch {
            push_unique(&mut head_selectors, local_branch.clone());
        }

        Self {
            local_branch,
            upstream_ref,
            remote_name,
            remote_url,
            host: parsed_remote.as_ref().map(|remote| remote.host.clone()),
            repository: parsed_remote
                .as_ref()
                .map(|remote| remote.repository.clone()),
            owner: parsed_remote.map(|remote| remote.owner),
            head_branch,
            head_selectors,
        }
    }
}

/// Parse the SSH scp-like, SSH URL, and HTTP(S) forms commonly used by GitHub.
///
/// The host is intentionally not restricted to `github.com`: GitHub Enterprise
/// installations use arbitrary hostnames and are resolved later by the provider.
pub fn parse_git_remote(remote_url: &str) -> Option<GitRemote> {
    let remote_url = remote_url.trim();
    if remote_url.is_empty() || remote_url.chars().any(char::is_whitespace) {
        return None;
    }

    let (host, path) = if let Some((scheme, remainder)) = remote_url.split_once("://") {
        if !matches!(
            scheme.to_ascii_lowercase().as_str(),
            "ssh" | "http" | "https"
        ) {
            return None;
        }
        let (authority, path) = remainder.split_once('/')?;
        (host_from_authority(authority)?, path)
    } else {
        let (authority, path) = remote_url.split_once(':')?;
        if authority.contains('/') || path.starts_with('/') {
            return None;
        }
        (host_from_authority(authority)?, path)
    };

    let path = path.trim_matches('/');
    let mut segments = path.split('/');
    let owner = segments.next()?;
    let repository = segments.next()?;
    if segments.next().is_some() || owner.is_empty() || repository.is_empty() {
        return None;
    }
    let repository = repository.strip_suffix(".git").unwrap_or(repository);
    if repository.is_empty() {
        return None;
    }

    Some(GitRemote {
        host: host.to_ascii_lowercase(),
        owner: owner.to_owned(),
        repository: repository.to_owned(),
    })
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn host_from_authority(authority: &str) -> Option<&str> {
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = host.split_once(':').map_or(host, |(host, _)| host);
    (!host.is_empty()).then_some(host)
}

fn remote_from_upstream(upstream: &str) -> Option<String> {
    normalized_upstream(upstream)
        .split_once('/')
        .map(|(remote, _)| remote.to_owned())
}

fn branch_from_upstream<'a>(upstream: &'a str, remote_name: Option<&str>) -> Option<&'a str> {
    let upstream = normalized_upstream(upstream);
    if let Some(remote_name) = remote_name {
        let prefix = format!("{remote_name}/");
        if let Some(branch) = upstream.strip_prefix(&prefix) {
            return (!branch.is_empty()).then_some(branch);
        }
    }
    upstream
        .split_once('/')
        .and_then(|(_, branch)| (!branch.is_empty()).then_some(branch))
}

fn normalized_upstream(upstream: &str) -> &str {
    upstream.strip_prefix("refs/remotes/").unwrap_or(upstream)
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

#[derive(Clone)]
struct GitCheckoutInspector {
    runner: Arc<dyn ProcessRunner>,
}

impl GitCheckoutInspector {
    fn new(runner: Arc<dyn ProcessRunner>) -> Self {
        Self { runner }
    }

    async fn inspect(&self, cwd: &Path) -> Result<CheckoutSourceContext, ChangeRequestError> {
        let checkout_root_value = self
            .git_required(cwd, &["rev-parse", "--show-toplevel"])
            .await?;
        if checkout_root_value.is_empty() {
            return Err(ChangeRequestError::RepositoryUnavailable);
        }
        let checkout_root = PathBuf::from(checkout_root_value);
        let local_branch = self
            .git_required(&checkout_root, &["branch", "--show-current"])
            .await?;
        if local_branch.is_empty() || local_branch == "HEAD" {
            return Err(ChangeRequestError::RepositoryUnavailable);
        }

        let upstream_ref = self
            .git_optional(
                &checkout_root,
                &[
                    "rev-parse",
                    "--abbrev-ref",
                    "--symbolic-full-name",
                    "@{upstream}",
                ],
            )
            .await;
        let branch_remote_key = format!("branch.{local_branch}.remote");
        let mut remote_name = self
            .git_optional(&checkout_root, &["config", "--get", &branch_remote_key])
            .await
            .filter(|remote| remote != ".");
        if remote_name.is_none() {
            remote_name = upstream_ref.as_deref().and_then(remote_from_upstream);
        }
        if remote_name.is_none() {
            remote_name = self
                .git_optional(&checkout_root, &["config", "--get", "remote.pushDefault"])
                .await;
        }
        if remote_name.is_none()
            && let Some(remotes) = self.git_optional(&checkout_root, &["remote"]).await
        {
            let remotes: Vec<_> = remotes
                .lines()
                .filter(|remote| !remote.is_empty())
                .collect();
            // `origin` is Git's conventional default destination when no
            // branch/upstream or push-default configuration is present. It is
            // still the useful PR source for a local branch in a fork +
            // upstream checkout.
            remote_name = remotes
                .iter()
                .find(|remote| **remote == "origin")
                .map(|remote| (*remote).to_owned())
                .or_else(|| (remotes.len() == 1).then(|| remotes[0].to_owned()));
        }

        let remote_url = if let Some(remote) = remote_name.as_deref() {
            self.git_optional(&checkout_root, &["remote", "get-url", "--push", remote])
                .await
        } else {
            None
        };
        let remote_url = if remote_url.is_none() {
            if let Some(remote) = remote_name.as_deref() {
                self.git_optional(&checkout_root, &["remote", "get-url", remote])
                    .await
            } else {
                None
            }
        } else {
            remote_url
        };

        let branch = BranchHeadContext::resolve(
            local_branch,
            upstream_ref.as_deref(),
            remote_name.as_deref(),
            remote_url.as_deref(),
        );
        // The default branch is read from local refs only. `refs/remotes/
        // <remote>/HEAD` is a fetch-time cache and may be absent; the provider
        // resolves the remote's default branch itself when needed. Never fall
        // back to `git ls-remote` here: it executes repository-controlled
        // transport and credential configuration (for example
        // `core.sshCommand`), which would let any watched checkout run
        // arbitrary commands on the host.
        let default_branch = if let Some(remote) = branch.remote_name.as_deref() {
            let remote_head = format!("refs/remotes/{remote}/HEAD");
            self.git_optional(
                &checkout_root,
                &["symbolic-ref", "--quiet", "--short", &remote_head],
            )
            .await
            .and_then(|reference| branch_from_upstream(&reference, Some(remote)).map(str::to_owned))
        } else {
            None
        };

        Ok(CheckoutSourceContext {
            checkout_root,
            branch,
            default_branch,
        })
    }

    async fn repository_identity(
        &self,
        cwd: &Path,
    ) -> Result<RepositoryIdentity, ChangeRequestError> {
        let root = self
            .git_required(cwd, &["rev-parse", "--show-toplevel"])
            .await?;
        if root.is_empty() {
            return Err(ChangeRequestError::RepositoryUnavailable);
        }
        let root = PathBuf::from(root);
        let remotes = self
            .git_optional(&root, &["remote", "-v"])
            .await
            .unwrap_or_default();
        let remote = primary_fetch_remote(&remotes)
            .and_then(parse_git_remote)
            .ok_or(ChangeRequestError::UnsupportedRepository)?;
        Ok(RepositoryIdentity { root, remote })
    }

    async fn git_required(&self, cwd: &Path, args: &[&str]) -> Result<String, ChangeRequestError> {
        self.git(cwd, args)
            .await
            .ok_or(ChangeRequestError::RepositoryUnavailable)
    }

    /// A write-side git call: the host's own words when it refuses.
    async fn git_run(
        &self,
        cwd: &Path,
        args: &[&str],
        timeout: Duration,
    ) -> Result<String, ActionError> {
        let output = self
            .runner
            .run(ProcessRequest {
                program: "git".into(),
                args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                cwd: cwd.to_owned(),
                env: vec![("GIT_TERMINAL_PROMPT".into(), "0".into())],
                timeout,
                output_limit: GIT_OUTPUT_LIMIT,
                stdin: None,
            })
            .await
            .map_err(classify_run_error)?;
        if !output.success {
            return Err(ActionError::Refused(readable_failure(&output.stderr)));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    async fn git_optional(&self, cwd: &Path, args: &[&str]) -> Option<String> {
        self.git(cwd, args).await
    }

    async fn git(&self, cwd: &Path, args: &[&str]) -> Option<String> {
        let output = self
            .runner
            .run(ProcessRequest {
                program: "git".into(),
                args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                cwd: cwd.to_owned(),
                env: Vec::new(),
                timeout: GIT_TIMEOUT,
                output_limit: GIT_OUTPUT_LIMIT,
                stdin: None,
            })
            .await
            .ok()?;
        if !output.success || output.stdout_truncated {
            return None;
        }
        String::from_utf8(output.stdout)
            .ok()
            .map(|value| value.trim().to_owned())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhRepoView {
    default_branch_ref: Option<GhBranchRef>,
}

#[derive(Debug, Deserialize)]
struct GhBranchRef {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPullRequest {
    number: u64,
    title: String,
    url: String,
    state: GhPullRequestState,
    base_ref_name: String,
    head_ref_name: String,
    updated_at: DateTime<Utc>,
    is_cross_repository: bool,
    head_repository_owner: GhRepositoryOwner,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum GhPullRequestState {
    Open,
    Closed,
    Merged,
}

impl From<GhPullRequestState> for ChangeRequestState {
    fn from(state: GhPullRequestState) -> Self {
        match state {
            GhPullRequestState::Open => Self::Open,
            GhPullRequestState::Closed => Self::Closed,
            GhPullRequestState::Merged => Self::Merged,
        }
    }
}

#[derive(Debug, Deserialize)]
struct GhRepositoryOwner {
    login: String,
}

/// The default branch only matters to suppress historical terminal pull
/// requests on the default branch itself; skip the extra provider request
/// unless a terminal candidate could actually be suppressed.
fn needs_default_branch(source: &CheckoutSourceContext, candidates: &[GhPullRequest]) -> bool {
    candidates.iter().any(|candidate| {
        candidate.head_ref_name == source.branch.head_branch
            && !matches!(candidate.state, GhPullRequestState::Open)
    })
}

fn select_pull_request(
    provider: &str,
    source: &CheckoutSourceContext,
    default_branch: Option<&str>,
    candidates: Vec<GhPullRequest>,
) -> Result<Option<ChangeRequestSummary>, ChangeRequestError> {
    let expected_owner = source.branch.owner.as_deref();
    let on_default_branch = default_branch == Some(&source.branch.head_branch);
    let mut matching: Vec<GhPullRequest> = candidates
        .into_iter()
        .filter(|candidate| candidate.head_ref_name == source.branch.head_branch)
        .filter(|candidate| {
            expected_owner.is_some_and(|owner| {
                candidate
                    .head_repository_owner
                    .login
                    .eq_ignore_ascii_case(owner)
            }) || (expected_owner.is_none() && !candidate.is_cross_repository)
        })
        .filter(|candidate| {
            !on_default_branch || matches!(candidate.state, GhPullRequestState::Open)
        })
        .collect();

    matching.sort_by(|left, right| {
        let left_open = matches!(left.state, GhPullRequestState::Open);
        let right_open = matches!(right.state, GhPullRequestState::Open);
        right_open
            .cmp(&left_open)
            .then_with(|| right.updated_at.cmp(&left.updated_at))
            .then_with(|| right.number.cmp(&left.number))
    });

    matching
        .into_iter()
        .next()
        .map(|pull_request| to_summary(provider, pull_request))
        .transpose()
}

fn to_summary(
    provider: &str,
    pull_request: GhPullRequest,
) -> Result<ChangeRequestSummary, ChangeRequestError> {
    if pull_request.number == 0
        || pull_request.title.trim().is_empty()
        || pull_request.base_ref_name.trim().is_empty()
        || pull_request.head_ref_name.trim().is_empty()
    {
        return Err(ChangeRequestError::Decode);
    }
    let url =
        reqwest::Url::parse(pull_request.url.trim()).map_err(|_| ChangeRequestError::Decode)?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(ChangeRequestError::Decode);
    }

    Ok(ChangeRequestSummary {
        provider: provider.to_owned(),
        number: pull_request.number,
        title: pull_request.title,
        url: url.to_string(),
        state: pull_request.state.into(),
        base_ref: pull_request.base_ref_name,
        head_ref: pull_request.head_ref_name,
    })
}

fn resolve_author(author: &str, viewer: &str) -> String {
    match author.trim() {
        "me" | "@me" => viewer.to_owned(),
        other => other.to_owned(),
    }
}

fn quoted_qualifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "").trim())
}

fn filter_qualifiers(filters: &ChangeRequestListFilters, viewer: &str) -> Vec<String> {
    let mut qualifiers: Vec<String> = filters
        .labels
        .iter()
        .filter(|group| !group.is_empty())
        .map(|group| {
            format!(
                "label:{}",
                group
                    .iter()
                    .map(|label| quoted_qualifier(label))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        })
        .collect();
    qualifiers.extend(
        filters
            .excluded_labels
            .iter()
            .map(|label| format!("-label:{}", quoted_qualifier(label))),
    );
    if let Some(author) = &filters.author {
        qualifiers.push(format!(
            "author:{}",
            quoted_qualifier(&resolve_author(author, viewer))
        ));
    }
    if let Some(draft) = filters.draft {
        qualifiers.push(format!("draft:{}", draft == ChangeRequestDraftFilter::Only));
    }
    if let Some(review) = filters.review {
        qualifiers.push(format!(
            "review:{}",
            match review {
                ChangeRequestReviewFilter::Approved => "approved",
                ChangeRequestReviewFilter::ChangesRequested => "changes_requested",
                ChangeRequestReviewFilter::ReviewRequired => "required",
                ChangeRequestReviewFilter::None => "none",
            }
        ));
    }
    if let Some(checks) = filters.checks {
        qualifiers.push(format!(
            "status:{}",
            if checks == ChangeRequestChecksState::Passing {
                "success"
            } else {
                "failure"
            }
        ));
    }
    qualifiers
}

fn matches_filters(
    item: &ChangeRequestListItem,
    filters: &ChangeRequestListFilters,
    viewer: &str,
) -> bool {
    let holds = |label: &str| {
        item.labels
            .iter()
            .any(|held| held.name.trim().eq_ignore_ascii_case(label.trim()))
    };
    filters
        .draft
        .is_none_or(|draft| item.is_draft == (draft == ChangeRequestDraftFilter::Only))
        && filters.review.is_none_or(|review| {
            item.review_decision
                == match review {
                    ChangeRequestReviewFilter::None => None,
                    ChangeRequestReviewFilter::Approved => {
                        Some(ChangeRequestReviewDecision::Approved)
                    }
                    ChangeRequestReviewFilter::ChangesRequested => {
                        Some(ChangeRequestReviewDecision::ChangesRequested)
                    }
                    ChangeRequestReviewFilter::ReviewRequired => {
                        Some(ChangeRequestReviewDecision::ReviewRequired)
                    }
                }
        })
        && filters
            .checks
            .is_none_or(|checks| item.checks_state == Some(checks))
        && filters
            .labels
            .iter()
            .all(|group| group.iter().any(|label| holds(label)))
        && !filters.excluded_labels.iter().any(|label| holds(label))
        && filters.author.as_deref().is_none_or(|author| {
            item.author.as_ref().is_some_and(|held| {
                held.login
                    .eq_ignore_ascii_case(&resolve_author(author, viewer))
            })
        })
}

fn valid_repository_path(path: &str) -> bool {
    let valid = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    };
    path.split_once('/')
        .is_some_and(|(owner, name)| valid(owner) && valid(name))
}

fn valid_login(login: &str) -> bool {
    !login.is_empty()
        && login
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '[' | ']'))
}

#[derive(Debug, Deserialize)]
struct GhSearchResponse {
    #[serde(default)]
    data: Option<GhSearchData>,
}

#[derive(Debug, Deserialize)]
struct GhSearchData {
    search: GhSearch,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhSearch {
    page_info: GhPageInfo,
    #[serde(default)]
    nodes: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPageInfo {
    #[serde(default)]
    has_next_page: bool,
    #[serde(default)]
    end_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GhNodes<T> {
    #[serde(default = "Vec::new")]
    nodes: Vec<Option<T>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhNameWithOwner {
    name_with_owner: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhRequestedReviewer {
    #[serde(default)]
    requested_reviewer: Option<GhReviewRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhCommitNode {
    commit: GhCommitRollup,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhCommitRollup {
    #[serde(default)]
    status_check_rollup: Option<GhRollupState>,
}

#[derive(Debug, Deserialize)]
struct GhRollupState {
    #[serde(default)]
    state: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhSearchPullRequest {
    number: u64,
    title: String,
    url: String,
    #[serde(default)]
    author: Option<GhActor>,
    head_ref_name: String,
    base_ref_name: String,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    is_draft: bool,
    #[serde(default)]
    mergeable: Option<String>,
    #[serde(default)]
    review_decision: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    #[serde(default)]
    merged_at: Option<String>,
    repository: GhNameWithOwner,
    #[serde(default)]
    review_requests: Option<GhNodes<GhRequestedReviewer>>,
    #[serde(default)]
    labels: Option<GhNodes<GhLabel>>,
    #[serde(default)]
    commits: Option<GhNodes<GhCommitNode>>,
}

impl GhSearchPullRequest {
    fn into_list_row(self) -> GhListPullRequest {
        let rollup = self
            .commits
            .into_iter()
            .flat_map(|commits| commits.nodes)
            .flatten()
            .filter_map(|node| node.commit.status_check_rollup)
            .filter_map(|rollup| rollup.state)
            .map(|state| GhCheck {
                state: Some(state),
                ..Default::default()
            })
            .collect();
        GhListPullRequest {
            number: self.number,
            title: self.title,
            url: self.url,
            author: self.author,
            head_ref_name: self.head_ref_name,
            base_ref_name: self.base_ref_name,
            state: self.state,
            is_draft: self.is_draft,
            mergeable: self.mergeable,
            review_decision: self.review_decision,
            additions: 0,
            deletions: 0,
            created_at: self.created_at,
            updated_at: self.updated_at,
            merged_at: self.merged_at,
            review_requests: self.review_requests.map(|requests| {
                requests
                    .nodes
                    .into_iter()
                    .flatten()
                    .filter_map(|node| node.requested_reviewer)
                    .collect()
            }),
            labels: self
                .labels
                .map(|labels| labels.nodes.into_iter().flatten().collect()),
            status_check_rollup: Some(rollup),
        }
    }
}

const GIT_FETCH_TIMEOUT: Duration = Duration::from_secs(120);

/// `git worktree list --porcelain` as (path, branch ref) pairs; detached entries have no branch.
fn parse_worktrees(listing: &str) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    let mut path: Option<String> = None;
    for line in listing.lines() {
        if let Some(value) = line.strip_prefix("worktree ") {
            path = Some(value.to_owned());
        } else if let (Some(value), Some(current)) = (line.strip_prefix("branch "), &path) {
            entries.push((current.clone(), value.to_owned()));
        } else if line.is_empty() {
            path = None;
        }
    }
    entries
}

/// A branch name we are willing to hand to git: no option-looking or ref-escaping forms.
fn valid_branch_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && !name.starts_with('/')
        && !name.ends_with('/')
        && !name.ends_with(".lock")
        && !name.contains("..")
        && !name.contains("//")
        && !name.contains("@{")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
}

fn primary_fetch_remote(output: &str) -> Option<&str> {
    primary_remote(output).map(|(_, url)| url)
}

fn primary_remote(output: &str) -> Option<(&str, &str)> {
    let mut remotes: Vec<(&str, &str)> = output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let name = parts.next()?;
            let url = parts.next()?;
            (parts.next() == Some("(fetch)")).then_some((name, url))
        })
        .collect();
    remotes.sort_by(|left, right| left.0.cmp(right.0));
    ["upstream", "origin"]
        .iter()
        .find_map(|preferred| remotes.iter().find(|(name, _)| name == preferred))
        .or_else(|| remotes.first())
        .copied()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhListPullRequest {
    number: u64,
    title: String,
    url: String,
    #[serde(default)]
    author: Option<GhActor>,
    head_ref_name: String,
    base_ref_name: String,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    is_draft: bool,
    #[serde(default)]
    mergeable: Option<String>,
    #[serde(default)]
    review_decision: Option<String>,
    #[serde(default)]
    additions: u64,
    #[serde(default)]
    deletions: u64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    #[serde(default)]
    merged_at: Option<String>,
    #[serde(default)]
    review_requests: Option<Vec<GhReviewRequest>>,
    #[serde(default)]
    labels: Option<Vec<GhLabel>>,
    #[serde(default)]
    status_check_rollup: Option<Vec<GhCheck>>,
}

#[derive(Debug, Deserialize)]
struct GhActor {
    login: String,
}

#[derive(Debug, Deserialize)]
struct GhReviewRequest {
    #[serde(default)]
    login: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GhLabel {
    name: String,
    #[serde(default)]
    color: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhCheck {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    details_url: Option<String>,
    #[serde(default)]
    target_url: Option<String>,
    #[serde(default)]
    workflow_name: Option<String>,
    #[serde(default)]
    completed_at: Option<String>,
    #[serde(default)]
    started_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhDetailPullRequest {
    #[serde(flatten)]
    summary: GhListPullRequest,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    changed_files: u64,
    #[serde(default)]
    closed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    is_cross_repository: bool,
    #[serde(default)]
    comments: Option<Vec<GhComment>>,
    #[serde(default)]
    reviews: Option<Vec<GhReview>>,
    #[serde(default)]
    latest_reviews: Option<Vec<GhReview>>,
    #[serde(default)]
    commits: Option<Vec<GhCommit>>,
    #[serde(default)]
    auto_merge_request: Option<GhAutoMergeRequest>,
    #[serde(default)]
    head_repository_owner: Option<GhActor>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhAutoMergeRequest {
    #[serde(default)]
    merge_method: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhComment {
    id: String,
    #[serde(default)]
    author: Option<GhActor>,
    #[serde(default)]
    body: String,
    created_at: DateTime<Utc>,
    #[serde(default)]
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhReview {
    #[serde(default)]
    id: String,
    #[serde(default)]
    author: Option<GhActor>,
    #[serde(default)]
    body: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    submitted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhCommit {
    #[serde(default)]
    oid: String,
    #[serde(default)]
    message_headline: String,
    #[serde(default)]
    committed_date: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
struct GhPullRequestFile {
    filename: String,
    #[serde(default)]
    previous_filename: Option<String>,
    #[serde(default)]
    status: String,
    #[serde(default)]
    additions: u64,
    #[serde(default)]
    deletions: u64,
    #[serde(default)]
    patch: Option<String>,
}

const PULL_REQUEST_ID_QUERY: &str = "query($owner: String!, $name: String!, $number: Int!) { repository(owner: $owner, name: $name) { pullRequest(number: $number) { id } } }";
const SUBJECT_SCOPE_QUERY: &str = "query($owner: String!, $name: String!, $number: Int!, $subjectId: ID!) { repository(owner: $owner, name: $name) { pullRequest(number: $number) { id } } node(id: $subjectId) { id ... on IssueComment { pullRequest { id } } ... on PullRequestReviewComment { pullRequest { id } } ... on PullRequestReview { pullRequest { id } } } }";

fn threads_fields() -> String {
    format!(
        "reviewThreads(first: 100, after: $cursor) {{ pageInfo {{ hasNextPage endCursor }} nodes {{ id isResolved isOutdated path line diffSide comments(first: 10) {{ totalCount pageInfo {{ hasNextPage }} nodes {{ id author {{ login }} body createdAt url {REACTION_GROUPS} }} }} }} }}"
    )
}

fn activity_query() -> String {
    format!(
        "query($owner: String!, $name: String!, $number: Int!, $cursor: String) {{ viewer {{ login }} repository(owner: $owner, name: $name) {{ pullRequest(number: $number) {{ id viewerCanUpdate {REACTION_GROUPS} {threads} comments(last: 100) {{ totalCount nodes {{ id author {{ login }} body createdAt url {REACTION_GROUPS} }} }} reviews(last: 100) {{ totalCount nodes {{ id author {{ login }} body state submittedAt url {REACTION_GROUPS} }} }} reviewDismissals: timelineItems(itemTypes: [REVIEW_DISMISSED_EVENT], first: 100) {{ nodes {{ ... on ReviewDismissedEvent {{ dismissalMessage review {{ id }} }} }} }} commits(last: 100) {{ nodes {{ commit {{ oid messageHeadline committedDate additions deletions parents(first: 1) {{ totalCount }} authors(first: 1) {{ nodes {{ name user {{ login }} }} }} }} }} }} }} }} }}",
        threads = threads_fields()
    )
}

fn threads_query() -> String {
    format!(
        "query($owner: String!, $name: String!, $number: Int!, $cursor: String) {{ repository(owner: $owner, name: $name) {{ pullRequest(number: $number) {{ {threads} }} }} }}",
        threads = threads_fields()
    )
}

fn activity_variables(
    identity: &RepositoryIdentity,
    number: u64,
    cursor: Option<String>,
) -> serde_json::Value {
    serde_json::json!({
        "owner": identity.remote.owner,
        "name": identity.remote.repository,
        "number": number,
        "cursor": cursor,
    })
}

const REACTION_CONTENTS: [(ChangeRequestReactionContent, &str); 8] = [
    (ChangeRequestReactionContent::ThumbsUp, "THUMBS_UP"),
    (ChangeRequestReactionContent::ThumbsDown, "THUMBS_DOWN"),
    (ChangeRequestReactionContent::Laugh, "LAUGH"),
    (ChangeRequestReactionContent::Hooray, "HOORAY"),
    (ChangeRequestReactionContent::Confused, "CONFUSED"),
    (ChangeRequestReactionContent::Heart, "HEART"),
    (ChangeRequestReactionContent::Rocket, "ROCKET"),
    (ChangeRequestReactionContent::Eyes, "EYES"),
];

fn reaction_name(content: ChangeRequestReactionContent) -> &'static str {
    REACTION_CONTENTS
        .iter()
        .find(|(known, _)| *known == content)
        .map(|(_, name)| *name)
        .unwrap_or("THUMBS_UP")
}

fn reaction_content(name: &str) -> Option<ChangeRequestReactionContent> {
    REACTION_CONTENTS
        .iter()
        .find(|(_, known)| *known == name)
        .map(|(content, _)| *content)
}

#[derive(Debug, Default, Deserialize)]
struct GhGraphqlErrors {
    #[serde(default)]
    errors: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct GhScopeResponse {
    #[serde(default)]
    data: Option<GhScopeData>,
}

#[derive(Debug, Deserialize)]
struct GhScopeData {
    #[serde(default)]
    repository: Option<GhScopeRepository>,
    #[serde(default)]
    node: Option<GhScopeNode>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhScopeRepository {
    #[serde(default)]
    pull_request: Option<GhNodeId>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhScopeNode {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    pull_request: Option<GhNodeId>,
}

#[derive(Debug, Clone, Deserialize)]
struct GhNodeId {
    id: String,
}

/// The node to react to: the pull request itself, or a subject GitHub says belongs to it.
fn subject_in_scope(scope: &GhScopeResponse, subject: Option<&str>) -> Option<String> {
    let data = scope.data.as_ref()?;
    let pull = data.repository.as_ref()?.pull_request.as_ref()?.id.clone();
    let Some(subject) = subject else {
        return Some(pull);
    };
    let node = data.node.as_ref()?;
    let owner = node.pull_request.as_ref()?.id.as_str();
    (owner == pull && node.id.as_deref() == Some(subject)).then(|| subject.to_owned())
}

#[derive(Debug, Deserialize)]
struct GhActivityResponse {
    #[serde(default)]
    data: Option<GhActivityData>,
}

#[derive(Debug, Deserialize)]
struct GhActivityData {
    #[serde(default)]
    viewer: Option<GhActor>,
    #[serde(default)]
    repository: Option<GhActivityRepository>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhActivityRepository {
    #[serde(default)]
    pull_request: Option<GhActivityPull>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhConnection<T> {
    #[serde(default)]
    total_count: Option<u64>,
    #[serde(default)]
    page_info: GhPageInfo,
    #[serde(default = "Vec::new")]
    nodes: Vec<T>,
}

impl<T> Default for GhConnection<T> {
    fn default() -> Self {
        Self {
            total_count: None,
            page_info: GhPageInfo::default(),
            nodes: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhActivityPull {
    #[serde(default)]
    id: String,
    #[serde(default)]
    viewer_can_update: bool,
    #[serde(default)]
    reaction_groups: Vec<GhReactionGroup>,
    #[serde(default)]
    review_threads: GhConnection<GhReviewThread>,
    #[serde(default)]
    comments: GhConnection<GhActivityComment>,
    #[serde(default)]
    reviews: GhConnection<GhActivityComment>,
    #[serde(default)]
    review_dismissals: GhConnection<Option<GhDismissal>>,
    #[serde(default)]
    commits: GhConnection<GhActivityCommitNode>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhReactionGroup {
    content: String,
    #[serde(default)]
    viewer_has_reacted: bool,
    #[serde(default)]
    reactors: GhConnection<Option<GhActor>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhReviewThread {
    #[serde(default)]
    id: String,
    #[serde(default)]
    is_resolved: bool,
    #[serde(default)]
    is_outdated: bool,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    line: Option<u64>,
    #[serde(default)]
    diff_side: Option<String>,
    #[serde(default)]
    comments: GhConnection<GhActivityComment>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhActivityComment {
    #[serde(default)]
    id: String,
    #[serde(default)]
    author: Option<GhActor>,
    #[serde(default)]
    body: String,
    #[serde(default)]
    created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    submitted_at: Option<DateTime<Utc>>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    reaction_groups: Vec<GhReactionGroup>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhDismissal {
    #[serde(default)]
    dismissal_message: Option<String>,
    #[serde(default)]
    review: Option<GhNodeId>,
}

#[derive(Debug, Deserialize)]
struct GhActivityCommitNode {
    commit: GhActivityCommitInfo,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhActivityCommitInfo {
    oid: String,
    #[serde(default)]
    message_headline: String,
    #[serde(default)]
    committed_date: Option<DateTime<Utc>>,
    #[serde(default)]
    additions: Option<u64>,
    #[serde(default)]
    deletions: Option<u64>,
    #[serde(default)]
    parents: GhConnection<serde_json::Value>,
    #[serde(default)]
    authors: GhConnection<GhCommitAuthor>,
}

#[derive(Debug, Deserialize)]
struct GhCommitAuthor {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    user: Option<GhActor>,
}

fn reactions(groups: Vec<GhReactionGroup>) -> Vec<ChangeRequestReaction> {
    groups
        .into_iter()
        .filter_map(|group| {
            let count = group.reactors.total_count.unwrap_or(0);
            if count == 0 {
                return None;
            }
            Some(ChangeRequestReaction {
                content: reaction_content(&group.content)?,
                count,
                viewer_has_reacted: group.viewer_has_reacted,
                reactors: group
                    .reactors
                    .nodes
                    .into_iter()
                    .flatten()
                    .map(|actor| actor.login)
                    .collect(),
            })
        })
        .collect()
}

/// True where markdown renders nothing: whitespace, or only HTML comments.
fn renders_empty(body: &str) -> bool {
    let mut rest = body;
    let mut visible = String::new();
    while let Some(start) = rest.find("<!--") {
        visible.push_str(&rest[..start]);
        rest = match rest[start..].find("-->") {
            Some(end) => &rest[start + end + 3..],
            None => "",
        };
    }
    visible.push_str(rest);
    visible.trim().is_empty()
}

fn is_verdict(state: &str) -> bool {
    matches!(
        state.to_ascii_uppercase().as_str(),
        "APPROVED" | "CHANGES_REQUESTED" | "DISMISSED"
    )
}

fn to_activity(
    host: &str,
    data: GhActivityData,
    threads: Vec<GhReviewThread>,
    threads_remain: bool,
) -> ChangeRequestActivity {
    let actor = |author: Option<GhActor>| {
        author.map(|actor| ChangeRequestActor {
            avatar_url: avatar_url(host, &actor.login),
            login: actor.login,
        })
    };
    let viewer = data.viewer.map(|viewer| viewer.login).unwrap_or_default();
    let Some(pull) = data
        .repository
        .and_then(|repository| repository.pull_request)
    else {
        return ChangeRequestActivity {
            viewer,
            ..Default::default()
        };
    };
    let dismissals: std::collections::HashMap<String, String> = pull
        .review_dismissals
        .nodes
        .into_iter()
        .flatten()
        .filter_map(|dismissal| Some((dismissal.review?.id, dismissal.dismissal_message?)))
        .collect();
    let comment = |kind, source: GhActivityComment, created_at| ChangeRequestActivityComment {
        id: source.id,
        kind,
        author: actor(source.author),
        body: source.body,
        created_at,
        url: source.url,
        review_state: None,
        path: None,
        line: None,
        thread_id: None,
        thread_resolved: false,
        outdated: false,
        left_side: false,
        reactions: reactions(source.reaction_groups),
    };
    let issue_read = pull.comments.nodes.len() as u64;
    let issue_total = pull
        .comments
        .total_count
        .unwrap_or(issue_read)
        .max(issue_read);
    let mut truncated = threads_remain || issue_total > issue_read;
    let mut comments = Vec::new();
    for source in pull.comments.nodes {
        if let Some(created_at) = source.created_at {
            comments.push(comment(
                ChangeRequestCommentKind::Comment,
                source,
                created_at,
            ));
        }
    }
    let mut kept_reviews = 0;
    for mut source in pull.reviews.nodes {
        let state = source.state.take().unwrap_or_default();
        let Some(submitted_at) = source.submitted_at else {
            continue;
        };
        if source.body.trim().is_empty() && !is_verdict(&state) {
            continue;
        }
        if state.eq_ignore_ascii_case("DISMISSED")
            && renders_empty(&source.body)
            && let Some(message) = dismissals.get(&source.id)
        {
            source.body = message.clone();
        }
        kept_reviews += 1;
        let mut review = comment(ChangeRequestCommentKind::Review, source, submitted_at);
        review.review_state = Some(state);
        comments.push(review);
    }
    let mut thread_total = 0;
    for thread in threads {
        thread_total += thread
            .comments
            .total_count
            .unwrap_or(thread.comments.nodes.len() as u64);
        truncated |= thread.comments.page_info.has_next_page;
        for source in thread.comments.nodes {
            let Some(created_at) = source.created_at else {
                continue;
            };
            let mut remark = comment(ChangeRequestCommentKind::ReviewComment, source, created_at);
            remark.path = thread.path.clone();
            remark.line = thread.line;
            remark.thread_id = Some(thread.id.clone());
            remark.thread_resolved = thread.is_resolved;
            remark.outdated = thread.is_outdated;
            remark.left_side = thread.diff_side.as_deref() == Some("LEFT");
            comments.push(remark);
        }
    }
    comments.sort_by(|left, right| left.created_at.cmp(&right.created_at));
    let commits = pull
        .commits
        .nodes
        .into_iter()
        .map(|node| {
            let commit = node.commit;
            let merge = commit.parents.total_count.unwrap_or(0) > 1;
            let author =
                commit
                    .authors
                    .nodes
                    .into_iter()
                    .next()
                    .and_then(|author| match author.user {
                        Some(user) => actor(Some(user)),
                        None => author.name.map(|login| ChangeRequestActor {
                            login,
                            avatar_url: None,
                        }),
                    });
            ChangeRequestActivityCommit {
                oid: commit.oid,
                headline: commit.message_headline,
                committed_at: commit.committed_date,
                additions: commit.additions.filter(|_| !merge),
                deletions: commit.deletions.filter(|_| !merge),
                author,
            }
        })
        .collect();
    ChangeRequestActivity {
        viewer,
        subject_id: pull.id,
        reactions: reactions(pull.reaction_groups),
        comments,
        comment_count: issue_total + kept_reviews + thread_total,
        truncated,
        commits,
        viewer_can_update: pull.viewer_can_update,
    }
}

/// A refusal the reader should see in words: a permission gate here, or the host's own message
/// for a write it turned down. Anything else stays a classified [`ChangeRequestError`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionError {
    Refused(String),
    Host(ChangeRequestError),
}

impl From<ChangeRequestError> for ActionError {
    fn from(error: ChangeRequestError) -> Self {
        Self::Host(error)
    }
}

const THREAD_SCOPE_QUERY: &str = "query($owner: String!, $name: String!, $number: Int!, $subjectId: ID!) { repository(owner: $owner, name: $name) { pullRequest(number: $number) { id } } node(id: $subjectId) { id ... on PullRequestReviewThread { pullRequest { id } } } }";
const PERMISSIONS_QUERY: &str = "query($owner: String!, $name: String!, $number: Int!) { repository(owner: $owner, name: $name) { mergeCommitAllowed squashMergeAllowed rebaseMergeAllowed viewerPermission defaultBranchRef { name } pullRequest(number: $number) { state viewerCanUpdate viewerDidAuthor headRefName headRepositoryOwner { login } } } }";
const BASE_COMPARISON_QUERY: &str = "query($owner: String!, $name: String!, $number: Int!, $headRef: String!) { repository(owner: $owner, name: $name) { pullRequest(number: $number) { viewerCanUpdateBranch baseRef { compare(headRef: $headRef) { behindBy } } } } }";
const LABEL_CANDIDATES_QUERY: &str = "query($owner: String!, $name: String!, $number: Int!) { repository(owner: $owner, name: $name) { labels(first: 100, orderBy: { field: NAME, direction: ASC }) { pageInfo { hasNextPage } nodes { name color description } } pullRequest(number: $number) { labels(first: 100) { nodes { name } } } } }";
const REVIEWER_CANDIDATES_QUERY: &str = "query($owner: String!, $name: String!, $number: Int!) { repository(owner: $owner, name: $name) { assignableUsers(first: 100) { pageInfo { hasNextPage } nodes { login name } } pullRequest(number: $number) { author { login } reviewRequests(first: 100) { nodes { requestedReviewer { ... on User { login name } ... on Team { slug name } ... on Bot { login } } } } } } }";

#[derive(Debug, Deserialize)]
struct GhPermissionsResponse {
    #[serde(default)]
    data: Option<GhPermissionsData>,
}

#[derive(Debug, Deserialize)]
struct GhPermissionsData {
    #[serde(default)]
    repository: Option<GhPermissionsRepository>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPermissionsRepository {
    #[serde(default)]
    merge_commit_allowed: bool,
    #[serde(default)]
    squash_merge_allowed: bool,
    #[serde(default)]
    rebase_merge_allowed: bool,
    #[serde(default)]
    viewer_permission: Option<String>,
    #[serde(default)]
    default_branch_ref: Option<GhRefName>,
    #[serde(default)]
    pull_request: Option<GhPermissionsPull>,
}

#[derive(Debug, Deserialize)]
struct GhRefName {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPermissionsPull {
    #[serde(default)]
    state: String,
    #[serde(default)]
    viewer_can_update: Option<bool>,
    #[serde(default)]
    viewer_did_author: Option<bool>,
    #[serde(default)]
    head_ref_name: String,
    #[serde(default)]
    head_repository_owner: Option<GhActor>,
}

#[derive(Debug, Deserialize)]
struct GhComparisonResponse {
    #[serde(default)]
    data: Option<GhComparisonData>,
}

#[derive(Debug, Deserialize)]
struct GhComparisonData {
    #[serde(default)]
    repository: Option<GhComparisonRepository>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhComparisonRepository {
    #[serde(default)]
    pull_request: Option<GhComparisonPull>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhComparisonPull {
    #[serde(default)]
    viewer_can_update_branch: Option<bool>,
    #[serde(default)]
    base_ref: Option<GhComparisonBase>,
}

#[derive(Debug, Deserialize)]
struct GhComparisonBase {
    #[serde(default)]
    compare: Option<GhCompare>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhCompare {
    behind_by: u64,
}

struct RepositoryFlags {
    merge: bool,
    squash: bool,
    rebase: bool,
    permission: String,
}

fn repository_flags(repository: &GhPermissionsRepository) -> RepositoryFlags {
    RepositoryFlags {
        merge: repository.merge_commit_allowed,
        squash: repository.squash_merge_allowed,
        rebase: repository.rebase_merge_allowed,
        permission: repository
            .viewer_permission
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_ascii_uppercase(),
    }
}

/// t3code S6.5: write is ADMIN/MAINTAIN/WRITE, triage adds TRIAGE, update is unknown-means-yes.
fn to_permissions(flags: &RepositoryFlags, pull: &GhPermissionsPull) -> ChangeRequestPermissions {
    let can_write = matches!(flags.permission.as_str(), "ADMIN" | "MAINTAIN" | "WRITE");
    let merge_methods = [
        (flags.merge, ChangeRequestMergeMethod::Merge),
        (flags.squash, ChangeRequestMergeMethod::Squash),
        (flags.rebase, ChangeRequestMergeMethod::Rebase),
    ]
    .into_iter()
    .filter_map(|(allowed, method)| allowed.then_some(method))
    .collect();
    ChangeRequestPermissions {
        merge_methods,
        can_write,
        can_triage: can_write || flags.permission == "TRIAGE",
        can_update: pull.viewer_can_update != Some(false),
        did_author: pull.viewer_did_author == Some(true),
        can_update_branch: false,
        behind_by: None,
        default_branch: None,
    }
}

fn action_allowed(
    action: ChangeRequestAction,
    permissions: &ChangeRequestPermissions,
) -> Result<(), String> {
    let method = match action {
        ChangeRequestAction::Merge { method } | ChangeRequestAction::EnableAutoMerge { method } => {
            Some(method)
        }
        _ => None,
    };
    if let Some(method) = method
        && !permissions.merge_methods.contains(&method)
    {
        return Err(format!(
            "This host cannot merge with the {} strategy.",
            merge_flag(method).trim_start_matches("--")
        ));
    }
    let allowed = match action {
        ChangeRequestAction::Merge { .. }
        | ChangeRequestAction::EnableAutoMerge { .. }
        | ChangeRequestAction::DisableAutoMerge
        | ChangeRequestAction::Revert
        | ChangeRequestAction::ApproveWorkflows => permissions.can_write,
        ChangeRequestAction::Ready
        | ChangeRequestAction::Draft
        | ChangeRequestAction::Close
        | ChangeRequestAction::Reopen => permissions.can_update,
        ChangeRequestAction::UpdateBranch { .. } => permissions.can_update_branch,
    };
    if allowed {
        Ok(())
    } else {
        Err(match action {
            ChangeRequestAction::UpdateBranch { .. } => {
                "This branch cannot be updated from here.".to_string()
            }
            ChangeRequestAction::Ready
            | ChangeRequestAction::Draft
            | ChangeRequestAction::Close
            | ChangeRequestAction::Reopen => {
                "You cannot change this pull request on this repository.".to_string()
            }
            _ => "You need write access on this repository to merge.".to_string(),
        })
    }
}

fn merge_flag(method: ChangeRequestMergeMethod) -> &'static str {
    match method {
        ChangeRequestMergeMethod::Merge => "--merge",
        ChangeRequestMergeMethod::Squash => "--squash",
        ChangeRequestMergeMethod::Rebase => "--rebase",
    }
}

/// The run id in `…/actions/runs/<id>[/…]`, digits only.
fn workflow_run_id(url: &str) -> Option<String> {
    let (_, rest) = url.split_once("/actions/runs/")?;
    let id: String = rest.chars().take_while(char::is_ascii_digit).collect();
    (!id.is_empty()).then_some(id)
}

/// RFC 3986 unreserved characters pass through; everything else is `%XX`.
fn percent_encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// The host's own words for a refused write, capped like t3code's `readableFailure`.
fn readable_failure(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let text = text.trim();
    if text.chars().count() > 320 {
        format!("{}…", text.chars().take(319).collect::<String>())
    } else {
        text.to_owned()
    }
}

#[derive(Debug, Deserialize)]
struct GhLabelCandidatesResponse {
    #[serde(default)]
    data: Option<GhLabelCandidatesData>,
}

#[derive(Debug, Deserialize)]
struct GhLabelCandidatesData {
    #[serde(default)]
    repository: Option<GhLabelCandidatesRepository>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhLabelCandidatesRepository {
    #[serde(default)]
    labels: GhConnection<GhRepoLabel>,
    #[serde(default)]
    pull_request: Option<GhLabelledPull>,
}

#[derive(Debug, Deserialize)]
struct GhRepoLabel {
    #[serde(default)]
    name: String,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GhLabelledPull {
    #[serde(default)]
    labels: GhConnection<GhRepoLabel>,
}

/// Repository labels by name; applied ones the first page missed are put first (t3code
/// GHJ:2277-2310).
fn to_label_candidates(repository: GhLabelCandidatesRepository) -> ChangeRequestLabelCandidates {
    let applied: Vec<String> = repository
        .pull_request
        .map(|pull| pull.labels.nodes)
        .unwrap_or_default()
        .into_iter()
        .map(|label| label.name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .collect();
    let mut labels: Vec<ChangeRequestLabelCandidate> = repository
        .labels
        .nodes
        .into_iter()
        .filter(|label| !label.name.trim().is_empty())
        .map(|label| ChangeRequestLabelCandidate {
            applied: applied.iter().any(|name| name == label.name.trim()),
            name: label.name.trim().to_owned(),
            color: label.color,
            description: label.description.filter(|text| !text.trim().is_empty()),
        })
        .collect();
    let missing: Vec<ChangeRequestLabelCandidate> = applied
        .iter()
        .filter(|name| !labels.iter().any(|label| &label.name == *name))
        .map(|name| ChangeRequestLabelCandidate {
            name: name.clone(),
            color: None,
            description: None,
            applied: true,
        })
        .collect();
    labels.splice(0..0, missing);
    ChangeRequestLabelCandidates {
        labels,
        truncated: repository.labels.page_info.has_next_page,
    }
}

#[derive(Debug, Deserialize)]
struct GhReviewerCandidatesResponse {
    #[serde(default)]
    data: Option<GhReviewerCandidatesData>,
}

#[derive(Debug, Deserialize)]
struct GhReviewerCandidatesData {
    #[serde(default)]
    repository: Option<GhReviewerCandidatesRepository>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhReviewerCandidatesRepository {
    #[serde(default)]
    assignable_users: GhConnection<GhNamedActor>,
    #[serde(default)]
    pull_request: Option<GhRequestedPull>,
}

#[derive(Debug, Deserialize)]
struct GhNamedActor {
    #[serde(default)]
    login: Option<String>,
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhRequestedPull {
    #[serde(default)]
    author: Option<GhActor>,
    #[serde(default)]
    review_requests: GhConnection<GhRequestedReviewerNode>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhRequestedReviewerNode {
    #[serde(default)]
    requested_reviewer: Option<GhNamedActor>,
}

/// Already-requested reviewers first (teams by slug), then everyone with access except the
/// author and those already listed (t3code GHJ:2162-2203).
fn to_reviewer_candidates(
    repository: GhReviewerCandidatesRepository,
) -> ChangeRequestReviewerCandidates {
    let pull = repository.pull_request;
    let author = pull
        .as_ref()
        .and_then(|pull| pull.author.as_ref())
        .map(|author| author.login.clone());
    let mut reviewers: Vec<ChangeRequestReviewerCandidate> = Vec::new();
    for node in pull
        .map(|pull| pull.review_requests.nodes)
        .unwrap_or_default()
    {
        let Some(actor) = node.requested_reviewer else {
            continue;
        };
        let (login, team) = match (actor.login, actor.slug) {
            (Some(login), _) => (login, false),
            (None, Some(slug)) => (slug, true),
            (None, None) => continue,
        };
        reviewers.push(ChangeRequestReviewerCandidate {
            login,
            name: actor.name.filter(|name| !name.trim().is_empty()),
            team,
            requested: true,
        });
    }
    for user in repository.assignable_users.nodes {
        let Some(login) = user.login else {
            continue;
        };
        let taken = author
            .as_deref()
            .is_some_and(|author| author.eq_ignore_ascii_case(&login))
            || reviewers
                .iter()
                .any(|reviewer| !reviewer.team && reviewer.login.eq_ignore_ascii_case(&login));
        if !taken {
            reviewers.push(ChangeRequestReviewerCandidate {
                login,
                name: user.name.filter(|name| !name.trim().is_empty()),
                team: false,
                requested: false,
            });
        }
    }
    ChangeRequestReviewerCandidates {
        reviewers,
        truncated: repository.assignable_users.page_info.has_next_page,
    }
}

const RESPONSE_CACHE_CAPACITY: usize = 256;

static RESPONSE_CACHE: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, serde_json::Value)>>,
> = std::sync::LazyLock::new(Default::default);

pub(crate) fn cached_response(key: &str, ttl: Duration) -> Option<serde_json::Value> {
    let cache = RESPONSE_CACHE.lock().ok()?;
    cache
        .get(key)
        .filter(|(at, _)| at.elapsed() < ttl)
        .map(|(_, value)| value.clone())
}

pub(crate) fn remember_response(key: String, value: &serde_json::Value) {
    let Ok(mut cache) = RESPONSE_CACHE.lock() else {
        return;
    };
    if cache.len() >= RESPONSE_CACHE_CAPACITY
        && let Some(oldest) = cache
            .iter()
            .min_by_key(|(_, (at, _))| *at)
            .map(|(key, _)| key.clone())
    {
        cache.remove(&oldest);
    }
    cache.insert(key, (std::time::Instant::now(), value.clone()));
}

pub(crate) fn invalidate_responses() {
    if let Ok(mut cache) = RESPONSE_CACHE.lock() {
        cache.clear();
    }
}

fn valid_commit_sha(commit: &str) -> bool {
    (7..=64).contains(&commit.len()) && commit.chars().all(|c| c.is_ascii_hexdigit())
}

/// The files API hands back hunks per file with no `diff --git` header, so the unified patch is
/// assembled here (t3code `decodePullRequestFilesJson`). Returns the slice and the raw entry
/// count, counted before decoding so the caller can page past entries that fail to decode.
fn files_patch(raw: &[u8]) -> Option<(ChangeRequestDiffSlice, usize)> {
    let entries: Vec<serde_json::Value> = serde_json::from_slice(raw).ok()?;
    let raw_count = entries.len();
    let mut slice = ChangeRequestDiffSlice::default();
    for entry in entries {
        let Ok(file) = serde_json::from_value::<GhPullRequestFile>(entry) else {
            continue;
        };
        let status = file.status.trim().to_ascii_lowercase();
        let hunks = file.patch.unwrap_or_default();
        if hunks.is_empty() && file.additions + file.deletions > 0 {
            slice.truncated = true;
            slice.omitted_file_stats.push(ChangeRequestFileStat {
                path: file.filename.clone(),
                additions: file.additions,
                deletions: file.deletions,
            });
        }
        let old_path = match file.previous_filename.as_deref().map(str::trim) {
            Some(previous) if status == "renamed" && !previous.is_empty() => previous.to_owned(),
            _ => file.filename.clone(),
        };
        let new_path = &file.filename;
        slice
            .patch
            .push_str(&format!("diff --git a/{old_path} b/{new_path}\n"));
        match status.as_str() {
            "added" => slice.patch.push_str("new file mode 100644\n"),
            "removed" => slice.patch.push_str("deleted file mode 100644\n"),
            "renamed" => slice
                .patch
                .push_str(&format!("rename from {old_path}\nrename to {new_path}\n")),
            _ => {}
        }
        let minus = if status == "added" {
            "/dev/null".to_owned()
        } else {
            format!("a/{old_path}")
        };
        let plus = if status == "removed" {
            "/dev/null".to_owned()
        } else {
            format!("b/{new_path}")
        };
        slice.patch.push_str(&format!("--- {minus}\n+++ {plus}\n"));
        if !hunks.is_empty() {
            slice.patch.push_str(&hunks);
            if !hunks.ends_with('\n') {
                slice.patch.push('\n');
            }
        }
    }
    Some((slice, raw_count))
}

fn gh_list_state(row: &GhListPullRequest) -> ChangeRequestState {
    if row.merged_at.as_deref().is_some_and(|at| !at.is_empty()) {
        return ChangeRequestState::Merged;
    }
    match row.state.as_deref().map(str::to_ascii_uppercase).as_deref() {
        Some("MERGED") => ChangeRequestState::Merged,
        Some("CLOSED") => ChangeRequestState::Closed,
        _ => ChangeRequestState::Open,
    }
}

fn checks_rollup(checks: &[GhCheck]) -> Option<ChangeRequestChecksState> {
    let (mut failing, mut pending, mut passing) = (false, false, false);
    for check in checks {
        if check
            .status
            .as_deref()
            .is_some_and(|status| !status.is_empty() && !status.eq_ignore_ascii_case("COMPLETED"))
        {
            pending = true;
            continue;
        }
        let outcome = check
            .conclusion
            .as_deref()
            .or(check.state.as_deref())
            .unwrap_or_default()
            .to_ascii_uppercase();
        match outcome.as_str() {
            "SUCCESS" => passing = true,
            "FAILURE" | "ERROR" | "TIMED_OUT" | "STARTUP_FAILURE" | "CANCELLED" => failing = true,
            "ACTION_REQUIRED" | "PENDING" | "EXPECTED" => pending = true,
            _ => {}
        }
    }
    if failing {
        Some(ChangeRequestChecksState::Failing)
    } else if pending {
        Some(ChangeRequestChecksState::Pending)
    } else if passing {
        Some(ChangeRequestChecksState::Passing)
    } else {
        None
    }
}

fn check_status(check: &GhCheck) -> ChangeRequestCheckStatus {
    if check
        .status
        .as_deref()
        .is_some_and(|status| !status.is_empty() && !status.eq_ignore_ascii_case("COMPLETED"))
    {
        return ChangeRequestCheckStatus::Pending;
    }
    match check
        .conclusion
        .as_deref()
        .or(check.state.as_deref())
        .unwrap_or_default()
        .to_ascii_uppercase()
        .as_str()
    {
        "SUCCESS" => ChangeRequestCheckStatus::Success,
        "ACTION_REQUIRED" => ChangeRequestCheckStatus::ActionRequired,
        "FAILURE" | "ERROR" | "TIMED_OUT" | "STARTUP_FAILURE" => ChangeRequestCheckStatus::Failure,
        "CANCELLED" => ChangeRequestCheckStatus::Cancelled,
        "SKIPPED" => ChangeRequestCheckStatus::Skipped,
        "PENDING" | "EXPECTED" => ChangeRequestCheckStatus::Pending,
        _ => ChangeRequestCheckStatus::Neutral,
    }
}

fn check_time(check: &GhCheck) -> Option<&str> {
    check
        .completed_at
        .as_deref()
        .or(check.started_at.as_deref())
        .filter(|at| !at.is_empty() && !at.starts_with("0001-01-01"))
}

fn dedupe_checks(rollup: &[GhCheck]) -> Vec<ChangeRequestCheck> {
    let mut kept: Vec<(String, &GhCheck)> = Vec::new();
    for check in rollup {
        let Some(name) = check.name.as_deref().or(check.context.as_deref()) else {
            continue;
        };
        let key = format!(
            "{} {name}",
            check.workflow_name.as_deref().unwrap_or_default()
        );
        match kept.iter_mut().find(|(known, _)| *known == key) {
            Some(slot) => {
                let newer = match (check_time(check), check_time(slot.1)) {
                    (Some(candidate), Some(current)) => candidate >= current,
                    (Some(_), None) | (None, None) => true,
                    (None, Some(_)) => false,
                };
                if newer {
                    slot.1 = check;
                }
            }
            None => kept.push((key, check)),
        }
    }
    let names: Vec<(&str, Option<&str>)> = kept
        .iter()
        .map(|(_, check)| {
            (
                check
                    .name
                    .as_deref()
                    .or(check.context.as_deref())
                    .unwrap_or_default(),
                check.workflow_name.as_deref(),
            )
        })
        .collect();
    kept.iter()
        .map(|(_, check)| {
            let name = check
                .name
                .as_deref()
                .or(check.context.as_deref())
                .unwrap_or_default();
            let shared = names.iter().any(|(other, workflow)| {
                *other == name && *workflow != check.workflow_name.as_deref()
            });
            ChangeRequestCheck {
                name: match (shared, check.workflow_name.as_deref()) {
                    (true, Some(workflow)) if !workflow.is_empty() => {
                        format!("{workflow} / {name}")
                    }
                    _ => name.to_owned(),
                },
                status: check_status(check),
                description: check.description.clone().filter(|text| !text.is_empty()),
                url: check
                    .details_url
                    .clone()
                    .or_else(|| check.target_url.clone())
                    .filter(|url| !url.is_empty()),
                workflow_name: check.workflow_name.clone().filter(|name| !name.is_empty()),
            }
        })
        .collect()
}

fn rollup_of(checks: &[ChangeRequestCheck]) -> Option<ChangeRequestChecksState> {
    use ChangeRequestCheckStatus as S;
    if checks
        .iter()
        .any(|check| matches!(check.status, S::Failure | S::Cancelled))
    {
        Some(ChangeRequestChecksState::Failing)
    } else if checks
        .iter()
        .any(|check| matches!(check.status, S::Pending | S::ActionRequired))
    {
        Some(ChangeRequestChecksState::Pending)
    } else if checks.iter().any(|check| check.status == S::Success) {
        Some(ChangeRequestChecksState::Passing)
    } else {
        None
    }
}

fn verdict_of(state: &str) -> Option<ChangeRequestVerdict> {
    match state {
        "APPROVED" => Some(ChangeRequestVerdict::Approved),
        "CHANGES_REQUESTED" => Some(ChangeRequestVerdict::ChangesRequested),
        _ => None,
    }
}

fn to_detail(
    identity: &RepositoryIdentity,
    viewer: &str,
    view: GhDetailPullRequest,
) -> Option<ChangeRequestDetail> {
    let host = identity.remote.host.clone();
    let checks = dedupe_checks(
        view.summary
            .status_check_rollup
            .as_deref()
            .unwrap_or_default(),
    );
    let requested: Vec<String> = view
        .summary
        .review_requests
        .iter()
        .flatten()
        .filter_map(|request| request.login.clone())
        .collect();
    let merged_at = view
        .summary
        .merged_at
        .as_deref()
        .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
        .map(|at| at.with_timezone(&Utc));
    let mut item = to_list_item(identity, viewer, view.summary)?;
    item.checks_state = rollup_of(&checks);
    let commits = view.commits.unwrap_or_default();
    let auto_merge = view
        .auto_merge_request
        .map(|request| ChangeRequestAutoMerge {
            method: match request
                .merge_method
                .as_deref()
                .map(str::to_ascii_uppercase)
                .as_deref()
            {
                Some("MERGE") => Some(ChangeRequestMergeMethod::Merge),
                Some("SQUASH") => Some(ChangeRequestMergeMethod::Squash),
                Some("REBASE") => Some(ChangeRequestMergeMethod::Rebase),
                _ => None,
            },
        });
    let head_owner = view.head_repository_owner.map(|owner| owner.login);
    let newest_commit = commits
        .iter()
        .filter_map(|commit| commit.committed_date)
        .max();
    let latest_reviews = view.latest_reviews.unwrap_or_default();
    let verdict_for = |login: &str| {
        latest_reviews
            .iter()
            .filter(|review| {
                review
                    .author
                    .as_ref()
                    .is_some_and(|author| author.login.eq_ignore_ascii_case(login))
            })
            .find_map(|review| {
                verdict_of(&review.state).map(|verdict| {
                    let stale = matches!(
                        (review.submitted_at, newest_commit),
                        (Some(submitted), Some(newest)) if submitted < newest
                    );
                    (verdict, stale)
                })
            })
    };
    let mut reviewers: Vec<ChangeRequestReviewer> = Vec::new();
    let reviewed = latest_reviews
        .iter()
        .filter(|review| verdict_of(&review.state).is_some())
        .filter_map(|review| review.author.as_ref().map(|author| author.login.clone()));
    for login in requested.iter().cloned().chain(reviewed) {
        if reviewers
            .iter()
            .any(|known| known.login.eq_ignore_ascii_case(&login))
        {
            continue;
        }
        let verdict = verdict_for(&login);
        reviewers.push(ChangeRequestReviewer {
            avatar_url: avatar_url(&host, &login),
            verdict: verdict.map(|(verdict, _)| verdict),
            stale: verdict.is_some_and(|(_, stale)| stale),
            login,
        });
    }
    let actor = |author: Option<GhActor>| {
        author.map(|actor| ChangeRequestActor {
            avatar_url: avatar_url(&host, &actor.login),
            login: actor.login,
        })
    };
    let issue_comments = view.comments.unwrap_or_default();
    let comment_count = issue_comments.len() as u64;
    let mut comments: Vec<ChangeRequestComment> = issue_comments
        .into_iter()
        .map(|comment| ChangeRequestComment {
            id: comment.id,
            kind: ChangeRequestCommentKind::Comment,
            author: actor(comment.author),
            body: comment.body,
            created_at: comment.created_at,
            url: comment.url.filter(|url| !url.is_empty()),
            review_state: None,
        })
        .collect();
    for review in view.reviews.unwrap_or_default() {
        let Some(submitted_at) = review.submitted_at else {
            continue;
        };
        if review.body.trim().is_empty()
            && !matches!(
                review.state.as_str(),
                "APPROVED" | "CHANGES_REQUESTED" | "DISMISSED"
            )
        {
            continue;
        }
        comments.push(ChangeRequestComment {
            id: review.id,
            kind: ChangeRequestCommentKind::Review,
            author: actor(review.author),
            body: review.body,
            created_at: submitted_at,
            url: None,
            review_state: Some(review.state),
        });
    }
    comments.sort_by(|left, right| left.created_at.cmp(&right.created_at));
    Some(ChangeRequestDetail {
        item,
        body: view.body.unwrap_or_default(),
        changed_files: view.changed_files,
        merged_at,
        closed_at: view.closed_at,
        is_cross_repository: view.is_cross_repository,
        reviewers,
        checks,
        comments,
        comment_count,
        commit_count: commits.len() as u64,
        commits: commits
            .into_iter()
            .filter(|commit| !commit.oid.is_empty())
            .map(|commit| ChangeRequestCommit {
                oid: commit.oid,
                headline: commit.message_headline,
                committed_at: commit.committed_date,
            })
            .collect(),
        auto_merge,
        head_owner,
    })
}

fn avatar_url(host: &str, login: &str) -> Option<String> {
    let valid = !login.is_empty()
        && login.len() <= 39
        && login.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && !login.starts_with('-');
    valid.then(|| format!("https://{host}/{login}.png?size=80"))
}

fn to_list_item(
    identity: &RepositoryIdentity,
    viewer: &str,
    row: GhListPullRequest,
) -> Option<ChangeRequestListItem> {
    if row.number == 0 || row.title.trim().is_empty() {
        return None;
    }
    let url = reqwest::Url::parse(row.url.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    let state = gh_list_state(&row);
    let host = identity.remote.host.clone();
    let author = row.author.map(|actor| ChangeRequestActor {
        avatar_url: avatar_url(&host, &actor.login),
        login: actor.login,
    });
    let viewer_review_requested = !author
        .as_ref()
        .is_some_and(|author| author.login.eq_ignore_ascii_case(viewer))
        && row.review_requests.iter().flatten().any(|request| {
            request
                .login
                .as_deref()
                .is_some_and(|login| login.eq_ignore_ascii_case(viewer))
        });
    let labels = row
        .labels
        .unwrap_or_default()
        .into_iter()
        .filter_map(|label| {
            let name = label.name.trim().to_owned();
            (!name.is_empty()).then_some(ChangeRequestLabel {
                name,
                color: label.color.filter(|color| !color.is_empty()),
            })
        })
        .collect();
    Some(ChangeRequestListItem {
        provider: "github".into(),
        repository: identity.name_with_owner(),
        number: row.number,
        title: row.title,
        url: url.to_string(),
        author,
        head_ref: row.head_ref_name,
        base_ref: row.base_ref_name,
        state,
        is_draft: row.is_draft,
        mergeability: match row.mergeable.as_deref() {
            Some("MERGEABLE") => ChangeRequestMergeability::Mergeable,
            Some("CONFLICTING") => ChangeRequestMergeability::Conflicting,
            _ => ChangeRequestMergeability::Unknown,
        },
        additions: row.additions,
        deletions: row.deletions,
        created_at: row.created_at,
        updated_at: row.updated_at,
        viewer_review_requested,
        labels,
        review_decision: match row.review_decision.as_deref() {
            Some("APPROVED") => Some(ChangeRequestReviewDecision::Approved),
            Some("CHANGES_REQUESTED") => Some(ChangeRequestReviewDecision::ChangesRequested),
            Some("REVIEW_REQUIRED") => Some(ChangeRequestReviewDecision::ReviewRequired),
            _ => None,
        },
        checks_state: checks_rollup(row.status_check_rollup.as_deref().unwrap_or_default()),
        host,
        device_id: None,
    })
}

fn classify_github_failure(stderr: &[u8]) -> ChangeRequestError {
    let stderr = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    if stderr.contains("rate limit") || stderr.contains("secondary rate") {
        ChangeRequestError::RateLimited
    } else if stderr.contains("not logged")
        || stderr.contains("authentication")
        || stderr.contains("authenticate")
        || stderr.contains("gh auth login")
        || stderr.contains("bad credentials")
    {
        ChangeRequestError::Authentication
    } else {
        ChangeRequestError::CommandFailed
    }
}

fn classify_run_error(error: ProcessRunError) -> ChangeRequestError {
    match error {
        ProcessRunError::Spawn(io::ErrorKind::NotFound) => ChangeRequestError::CliUnavailable,
        ProcessRunError::Timeout => ChangeRequestError::Timeout,
        ProcessRunError::Spawn(_) | ProcessRunError::Io => ChangeRequestError::CommandFailed,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcessRequest {
    program: String,
    args: Vec<String>,
    cwd: PathBuf,
    env: Vec<(String, String)>,
    timeout: Duration,
    output_limit: usize,
    stdin: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcessOutput {
    success: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    stdout_truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessRunError {
    Spawn(io::ErrorKind),
    Timeout,
    Io,
}

#[async_trait]
trait ProcessRunner: Send + Sync {
    async fn run(&self, request: ProcessRequest) -> Result<ProcessOutput, ProcessRunError>;
}

struct SystemProcessRunner;

#[async_trait]
impl ProcessRunner for SystemProcessRunner {
    async fn run(&self, request: ProcessRequest) -> Result<ProcessOutput, ProcessRunError> {
        let mut command = tokio::process::Command::new(&request.program);
        if request.program == "gh" {
            zeron_harness::compose_login_shell_path(&mut command);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.as_std_mut().creation_flags(0x08000000);
        }
        command
            .args(&request.args)
            .current_dir(&request.cwd)
            .envs(request.env)
            .stdin(if request.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command
            .spawn()
            .map_err(|error| ProcessRunError::Spawn(error.kind()))?;
        let stdout = child.stdout.take().ok_or(ProcessRunError::Io)?;
        let stderr = child.stderr.take().ok_or(ProcessRunError::Io)?;
        let stdin = child.stdin.take().zip(request.stdin);
        let completed = tokio::time::timeout(request.timeout, async {
            tokio::try_join!(
                async {
                    if let Some((mut pipe, input)) = stdin {
                        use tokio::io::AsyncWriteExt;
                        pipe.write_all(&input).await?;
                        pipe.shutdown().await?;
                    }
                    Ok::<_, io::Error>(())
                },
                child.wait(),
                read_capped(stdout, request.output_limit),
                read_capped(stderr, request.output_limit),
            )
        })
        .await;

        let (_, status, (stdout, stdout_truncated), (stderr, _stderr_truncated)) = match completed {
            Ok(Ok(output)) => output,
            Ok(Err(_)) => return Err(ProcessRunError::Io),
            Err(_) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(ProcessRunError::Timeout);
            }
        };
        Ok(ProcessOutput {
            success: status.success(),
            stdout,
            stderr,
            stdout_truncated,
        })
    }
}

async fn read_capped(
    mut reader: impl AsyncRead + Unpin,
    limit: usize,
) -> io::Result<(Vec<u8>, bool)> {
    let mut output = Vec::with_capacity(limit.min(16 * 1024));
    let mut buffer = [0_u8; 8 * 1024];
    let mut truncated = false;
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        let remaining = limit.saturating_sub(output.len());
        let keep = read.min(remaining);
        output.extend_from_slice(&buffer[..keep]);
        truncated |= keep < read;
    }
    Ok((output, truncated))
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct FakeProcessRunner {
        requests: Mutex<Vec<ProcessRequest>>,
        responses: Mutex<VecDeque<Result<ProcessOutput, ProcessRunError>>>,
    }

    impl FakeProcessRunner {
        fn with_responses(
            responses: impl IntoIterator<Item = Result<ProcessOutput, ProcessRunError>>,
        ) -> Arc<Self> {
            Arc::new(Self {
                requests: Mutex::new(Vec::new()),
                responses: Mutex::new(responses.into_iter().collect()),
            })
        }

        fn requests(&self) -> Vec<ProcessRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ProcessRunner for FakeProcessRunner {
        async fn run(&self, request: ProcessRequest) -> Result<ProcessOutput, ProcessRunError> {
            self.requests.lock().unwrap().push(request);
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("missing fake process response")
        }
    }

    struct ExecutableGhRunner {
        executable: PathBuf,
    }

    #[async_trait]
    impl ProcessRunner for ExecutableGhRunner {
        async fn run(&self, mut request: ProcessRequest) -> Result<ProcessOutput, ProcessRunError> {
            if request.program == "gh" {
                #[cfg(unix)]
                {
                    request.program = self.executable.to_string_lossy().into_owned();
                }
                #[cfg(windows)]
                {
                    request.args.splice(
                        ..0,
                        [
                            "-NoLogo".into(),
                            "-NoProfile".into(),
                            "-NonInteractive".into(),
                            "-ExecutionPolicy".into(),
                            "Bypass".into(),
                            "-File".into(),
                            self.executable.to_string_lossy().into_owned(),
                        ],
                    );
                    request.program = "powershell.exe".into();
                }
            }
            SystemProcessRunner.run(request).await
        }
    }

    fn run_git(cwd: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("git fixture command starts");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn command_success(stdout: impl Into<Vec<u8>>) -> Result<ProcessOutput, ProcessRunError> {
        Ok(ProcessOutput {
            success: true,
            stdout: stdout.into(),
            stderr: Vec::new(),
            stdout_truncated: false,
        })
    }

    fn command_failure(stderr: &str) -> Result<ProcessOutput, ProcessRunError> {
        Ok(ProcessOutput {
            success: false,
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
            stdout_truncated: false,
        })
    }

    fn source(branch: &str, owner: &str, default_branch: Option<&str>) -> CheckoutSourceContext {
        CheckoutSourceContext {
            checkout_root: PathBuf::from("/checkout"),
            branch: BranchHeadContext::resolve(
                branch,
                Some(&format!("origin/{branch}")),
                Some("origin"),
                Some(&format!("https://github.com/{owner}/zeron.git")),
            ),
            default_branch: default_branch.map(str::to_owned),
        }
    }

    fn pull_request(
        number: u64,
        state: &str,
        owner: &str,
        branch: &str,
        updated_at: &str,
    ) -> serde_json::Value {
        serde_json::json!({
            "number": number,
            "title": format!("Pull request {number}"),
            "url": format!("https://github.com/acme/zeron/pull/{number}"),
            "state": state,
            "baseRefName": "main",
            "headRefName": branch,
            "updatedAt": updated_at,
            "isCrossRepository": owner != "acme",
            "headRepositoryOwner": { "login": owner }
        })
    }

    async fn resolve_with(
        source: &CheckoutSourceContext,
        response: Result<ProcessOutput, ProcessRunError>,
    ) -> (
        Result<Option<ChangeRequestSummary>, ChangeRequestError>,
        Arc<FakeProcessRunner>,
    ) {
        let runner = FakeProcessRunner::with_responses([response]);
        let github = GitHubCli::with_runner(runner.clone());
        let result = github.find_for_branch(source).await;
        (result, runner)
    }

    #[tokio::test]
    async fn github_cli_uses_exact_arguments_and_checkout_cwd() {
        let source = source("feature/status", "acme", Some("main"));
        let json = serde_json::to_vec(&vec![pull_request(
            90,
            "OPEN",
            "acme",
            "feature/status",
            "2026-08-15T12:00:00Z",
        )])
        .unwrap();
        let (result, runner) = resolve_with(&source, command_success(json)).await;

        let summary = result.unwrap().unwrap();
        assert_eq!(summary.number, 90);
        assert_eq!(summary.state, ChangeRequestState::Open);
        assert_eq!(summary.provider, "github");
        let requests = runner.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].program, "gh");
        assert_eq!(requests[0].cwd, Path::new("/checkout"));
        assert_eq!(
            requests[0].args,
            [
                "pr",
                "list",
                "--head",
                "acme:feature/status",
                "--state",
                "all",
                "--limit",
                "20",
                "--json",
                GITHUB_JSON_FIELDS,
            ]
        );
        assert_eq!(requests[0].env, [("GH_PROMPT_DISABLED".into(), "1".into())]);
        assert_eq!(requests[0].timeout, GITHUB_TIMEOUT);
        assert_eq!(requests[0].output_limit, GITHUB_OUTPUT_LIMIT);
    }

    #[tokio::test]
    async fn real_git_checkout_resolves_through_host_fake_gh() {
        let temp = tempfile::tempdir().expect("fixture tempdir");
        let checkout = temp.path().join("checkout");
        std::fs::create_dir_all(&checkout).expect("checkout directory");
        run_git(&checkout, &["init", "-q", "-b", "main"]);
        run_git(&checkout, &["checkout", "-q", "-b", "feature/status"]);
        run_git(
            &checkout,
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/acme/zeron.git",
            ],
        );

        #[cfg(unix)]
        let fake_gh = temp.path().join("gh");
        #[cfg(windows)]
        let fake_gh = temp.path().join("gh.ps1");
        #[cfg(unix)]
        let fake_gh_contents = r##"#!/bin/sh
if [ "$GH_PROMPT_DISABLED" != "1" ]; then
  echo "interactive auth was not disabled" >&2
  exit 2
fi
printf '%s\n' '[{"number":90,"title":"Host-resolved pull request","url":"https://github.com/acme/zeron/pull/90","state":"OPEN","baseRefName":"main","headRefName":"feature/status","updatedAt":"2026-08-15T12:00:00Z","isCrossRepository":false,"headRepositoryOwner":{"login":"acme"}}]'
"##;
        #[cfg(windows)]
        let fake_gh_contents = r##"if ($env:GH_PROMPT_DISABLED -ne '1') {
  [Console]::Error.WriteLine('interactive auth was not disabled')
  exit 2
}
[Console]::Out.WriteLine('[{"number":90,"title":"Host-resolved pull request","url":"https://github.com/acme/zeron/pull/90","state":"OPEN","baseRefName":"main","headRefName":"feature/status","updatedAt":"2026-08-15T12:00:00Z","isCrossRepository":false,"headRepositoryOwner":{"login":"acme"}}]')
"##;
        std::fs::write(&fake_gh, fake_gh_contents).expect("write fake gh");
        #[cfg(unix)]
        {
            let mut permissions = std::fs::metadata(&fake_gh)
                .expect("fake gh metadata")
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&fake_gh, permissions).expect("make fake gh executable");
        }

        let runner: Arc<dyn ProcessRunner> = Arc::new(ExecutableGhRunner {
            executable: fake_gh,
        });
        let resolver = ChangeRequestResolver {
            inspector: GitCheckoutInspector::new(runner.clone()),
            github: GitHubCli::with_runner(runner),
        };

        let resolution = resolver
            .resolve_github(&checkout)
            .await
            .expect("host resolves fake GitHub pull request");

        assert_eq!(
            std::fs::canonicalize(&resolution.source.checkout_root).expect("resolved root"),
            std::fs::canonicalize(&checkout).expect("fixture root")
        );
        assert_eq!(resolution.source.branch.local_branch, "feature/status");
        assert_eq!(resolution.source.branch.owner.as_deref(), Some("acme"));
        let pull_request = resolution.change_request.expect("pull request");
        assert_eq!(pull_request.number, 90);
        assert_eq!(pull_request.state, ChangeRequestState::Open);
        assert_eq!(pull_request.head_ref, "feature/status");
    }

    #[tokio::test]
    async fn git_inspector_uses_origin_without_upstream_in_a_multi_remote_checkout() {
        let temp = tempfile::tempdir().expect("fixture tempdir");
        let checkout = temp.path().join("checkout");
        std::fs::create_dir_all(&checkout).expect("checkout directory");
        run_git(&checkout, &["init", "-q", "-b", "main"]);
        run_git(&checkout, &["checkout", "-q", "-b", "feature/no-upstream"]);
        run_git(
            &checkout,
            &[
                "remote",
                "add",
                "origin",
                "git@github.com:contributor/zeron.git",
            ],
        );
        run_git(
            &checkout,
            &[
                "remote",
                "add",
                "upstream",
                "https://github.com/acme/zeron.git",
            ],
        );

        let source = ChangeRequestResolver::new()
            .inspect_checkout(&checkout)
            .await
            .expect("inspect checkout without branch upstream");

        assert_eq!(source.branch.upstream_ref, None);
        assert_eq!(source.branch.remote_name.as_deref(), Some("origin"));
        assert_eq!(
            source.branch.remote_url.as_deref(),
            Some("git@github.com:contributor/zeron.git")
        );
        assert_eq!(source.branch.owner.as_deref(), Some("contributor"));
        assert_eq!(
            source.branch.head_selectors,
            ["contributor:feature/no-upstream", "feature/no-upstream"]
        );
    }

    #[tokio::test]
    async fn git_inspector_never_runs_remote_transport_for_an_absent_default_branch() {
        let temp = tempfile::tempdir().expect("fixture tempdir");
        let checkout = temp.path().join("checkout");
        std::fs::create_dir_all(&checkout).expect("checkout directory");
        run_git(&checkout, &["init", "-q", "-b", "main"]);
        run_git(
            &checkout,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.com",
                "commit",
                "--allow-empty",
                "-qm",
                "fixture",
            ],
        );
        run_git(
            &checkout,
            &["remote", "add", "origin", "git@github.com:acme/zeron.git"],
        );
        // A repository-controlled transport command: any Git subcommand that
        // touches the remote (such as the former `ls-remote` default-branch
        // fallback) would execute it and create the marker.
        let marker = temp.path().join("transport-ran");
        #[cfg(unix)]
        let transport_command = format!("sh -c 'touch {}; exit 1'", marker.display());
        #[cfg(windows)]
        let transport_command = format!(
            "powershell.exe -NoLogo -NoProfile -NonInteractive -Command \"New-Item -ItemType File -Force -LiteralPath '{}'; exit 1\"",
            marker.to_string_lossy().replace('\'', "''")
        );
        run_git(
            &checkout,
            &["config", "core.sshCommand", &transport_command],
        );

        let source = ChangeRequestResolver::new()
            .inspect_checkout(&checkout)
            .await
            .expect("inspect checkout without a cached remote HEAD");

        assert_eq!(source.branch.remote_name.as_deref(), Some("origin"));
        assert_eq!(
            source.default_branch, None,
            "an unfetched default branch stays unknown until the provider resolves it"
        );
        assert!(
            !marker.exists(),
            "checkout inspection must never invoke Git remote transport"
        );
    }

    #[tokio::test]
    async fn merged_pull_request_is_returned_when_no_open_exists() {
        let source = source("feature/status", "acme", Some("main"));
        let json = serde_json::to_vec(&vec![pull_request(
            91,
            "MERGED",
            "acme",
            "feature/status",
            "2026-08-15T12:00:00Z",
        )])
        .unwrap();

        let (result, _) = resolve_with(&source, command_success(json)).await;
        let summary = result.unwrap().unwrap();
        assert_eq!(summary.number, 91);
        assert_eq!(summary.state, ChangeRequestState::Merged);
    }

    #[tokio::test]
    async fn closed_pull_request_is_returned_when_no_open_exists() {
        let source = source("feature/status", "acme", Some("main"));
        let json = serde_json::to_vec(&vec![pull_request(
            92,
            "CLOSED",
            "acme",
            "feature/status",
            "2026-08-15T12:00:00Z",
        )])
        .unwrap();

        let (result, _) = resolve_with(&source, command_success(json)).await;
        assert_eq!(result.unwrap().unwrap().state, ChangeRequestState::Closed);
    }

    #[tokio::test]
    async fn open_pull_request_wins_over_newer_terminal_candidates() {
        let source = source("feature/status", "acme", Some("main"));
        let json = serde_json::to_vec(&vec![
            pull_request(
                93,
                "MERGED",
                "acme",
                "feature/status",
                "2026-08-15T14:00:00Z",
            ),
            pull_request(94, "OPEN", "acme", "feature/status", "2026-08-15T10:00:00Z"),
            pull_request(
                95,
                "CLOSED",
                "acme",
                "feature/status",
                "2026-08-15T15:00:00Z",
            ),
        ])
        .unwrap();

        let (result, _) = resolve_with(&source, command_success(json)).await;
        assert_eq!(result.unwrap().unwrap().number, 94);
    }

    #[tokio::test]
    async fn newest_terminal_pull_request_wins_without_an_open_candidate() {
        let source = source("feature/status", "acme", Some("main"));
        let json = serde_json::to_vec(&vec![
            pull_request(
                96,
                "MERGED",
                "acme",
                "feature/status",
                "2026-08-15T12:00:00Z",
            ),
            pull_request(
                97,
                "CLOSED",
                "acme",
                "feature/status",
                "2026-08-15T13:00:00Z",
            ),
        ])
        .unwrap();

        let (result, _) = resolve_with(&source, command_success(json)).await;
        assert_eq!(result.unwrap().unwrap().number, 97);
    }

    #[tokio::test]
    async fn empty_specific_lookup_uses_the_next_safe_selector() {
        let source = source("feature/status", "acme", Some("main"));
        let json = serde_json::to_vec(&vec![pull_request(
            100,
            "OPEN",
            "acme",
            "feature/status",
            "2026-08-15T12:00:00Z",
        )])
        .unwrap();
        let runner = FakeProcessRunner::with_responses([
            command_success(b"[]".to_vec()),
            command_success(json),
        ]);
        let github = GitHubCli::with_runner(runner.clone());

        let result = github.find_for_branch(&source).await.unwrap().unwrap();

        assert_eq!(result.number, 100);
        let requests = runner.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].args[3], "acme:feature/status");
        assert_eq!(requests[1].args[3], "feature/status");
    }

    #[tokio::test]
    async fn pull_request_from_another_fork_is_rejected() {
        let source = source("feature/status", "contributor", Some("main"));
        let json = serde_json::to_vec(&vec![pull_request(
            98,
            "OPEN",
            "someone-else",
            "feature/status",
            "2026-08-15T12:00:00Z",
        )])
        .unwrap();

        let (result, _) = resolve_with(&source, command_success(json)).await;
        assert_eq!(result.unwrap(), None);
    }

    #[tokio::test]
    async fn partial_or_invalid_json_is_classified_as_decode() {
        let source = source("feature/status", "acme", Some("main"));
        for json in [br#"[{"number":90}]"#.to_vec(), b"not-json".to_vec()] {
            let (result, _) = resolve_with(&source, command_success(json)).await;
            assert_eq!(result.unwrap_err(), ChangeRequestError::Decode);
        }
    }

    #[tokio::test]
    async fn truncated_json_is_classified_as_decode() {
        let source = source("feature/status", "acme", Some("main"));
        let runner = FakeProcessRunner::with_responses([Ok(ProcessOutput {
            success: true,
            stdout: b"[]".to_vec(),
            stderr: Vec::new(),
            stdout_truncated: true,
        })]);
        let github = GitHubCli::with_runner(runner);

        let result = github.find_for_branch(&source).await;

        assert_eq!(result.unwrap_err(), ChangeRequestError::Decode);
    }

    #[tokio::test]
    async fn missing_github_executable_is_classified() {
        let source = source("feature/status", "acme", Some("main"));
        let (result, _) = resolve_with(
            &source,
            Err(ProcessRunError::Spawn(io::ErrorKind::NotFound)),
        )
        .await;
        assert_eq!(result.unwrap_err(), ChangeRequestError::CliUnavailable);
    }

    #[tokio::test]
    async fn authentication_failure_is_classified_without_exposing_stderr() {
        let source = source("feature/status", "acme", Some("main"));
        let secret_stderr = "not logged into github.example.com; token secret-value";
        let (result, _) = resolve_with(&source, command_failure(secret_stderr)).await;
        let error = result.unwrap_err();
        assert_eq!(error, ChangeRequestError::Authentication);
        assert!(!error.to_string().contains("secret-value"));
    }

    #[tokio::test]
    async fn rate_limit_failure_is_classified() {
        let source = source("feature/status", "acme", Some("main"));
        let (result, _) =
            resolve_with(&source, command_failure("GraphQL: API rate limit exceeded")).await;
        assert_eq!(result.unwrap_err(), ChangeRequestError::RateLimited);
    }

    #[tokio::test]
    async fn generic_github_failure_is_classified_without_exposing_stderr() {
        let source = source("feature/status", "acme", Some("main"));
        let (result, _) = resolve_with(
            &source,
            command_failure("provider failed with sensitive details"),
        )
        .await;
        let error = result.unwrap_err();
        assert_eq!(error, ChangeRequestError::CommandFailed);
        assert!(!error.to_string().contains("sensitive details"));
    }

    #[tokio::test]
    async fn timeout_is_classified() {
        let source = source("feature/status", "acme", Some("main"));
        let (result, _) = resolve_with(&source, Err(ProcessRunError::Timeout)).await;
        assert_eq!(result.unwrap_err(), ChangeRequestError::Timeout);
    }

    #[tokio::test]
    async fn terminal_pull_request_is_suppressed_on_default_branch() {
        let source = source("main", "acme", Some("main"));
        let json = serde_json::to_vec(&vec![pull_request(
            99,
            "MERGED",
            "acme",
            "main",
            "2026-08-15T12:00:00Z",
        )])
        .unwrap();

        let (result, _) = resolve_with(&source, command_success(json)).await;
        assert_eq!(result.unwrap(), None);
    }

    #[tokio::test]
    async fn unknown_default_branch_resolves_through_the_provider_before_suppression() {
        let source = source("main", "acme", None);
        let list = serde_json::to_vec(&vec![pull_request(
            99,
            "MERGED",
            "acme",
            "main",
            "2026-08-15T12:00:00Z",
        )])
        .unwrap();
        let runner = FakeProcessRunner::with_responses([
            command_success(list),
            command_success(br#"{"defaultBranchRef":{"name":"main"}}"#.to_vec()),
        ]);
        let github = GitHubCli::with_runner(runner.clone());

        let result = github.find_for_branch(&source).await;

        assert_eq!(result.unwrap(), None, "historical PR on main is suppressed");
        let requests = runner.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].program, "gh");
        assert_eq!(
            requests[1].args,
            [
                "repo",
                "view",
                "github.com/acme/zeron",
                "--json",
                "defaultBranchRef"
            ]
        );
        assert_eq!(requests[1].env, [("GH_PROMPT_DISABLED".into(), "1".into())]);
    }

    #[tokio::test]
    async fn open_candidates_skip_the_provider_default_branch_lookup() {
        let source = source("feature/status", "acme", None);
        let json = serde_json::to_vec(&vec![pull_request(
            90,
            "OPEN",
            "acme",
            "feature/status",
            "2026-08-15T12:00:00Z",
        )])
        .unwrap();

        let (result, runner) = resolve_with(&source, command_success(json)).await;

        assert_eq!(result.unwrap().unwrap().number, 90);
        assert_eq!(runner.requests().len(), 1);
    }

    #[tokio::test]
    async fn provider_default_branch_failure_keeps_the_terminal_pull_request() {
        let source = source("main", "acme", None);
        let list = serde_json::to_vec(&vec![pull_request(
            99,
            "MERGED",
            "acme",
            "main",
            "2026-08-15T12:00:00Z",
        )])
        .unwrap();
        let runner = FakeProcessRunner::with_responses([
            command_success(list),
            command_failure("could not resolve repository"),
        ]);
        let github = GitHubCli::with_runner(runner);

        let result = github.find_for_branch(&source).await;

        assert_eq!(result.unwrap().unwrap().number, 99);
    }

    #[tokio::test]
    async fn git_inspector_reads_branch_upstream_remote_and_default_branch() {
        let runner = FakeProcessRunner::with_responses([
            command_success("/checkout\n"),
            command_success("feature/status\n"),
            command_success("fork/published-status\n"),
            command_success("fork\n"),
            command_success("git@github.com:contributor/zeron.git\n"),
            command_success("fork/main\n"),
        ]);
        let inspector = GitCheckoutInspector::new(runner.clone());

        let source = inspector.inspect(Path::new("/nested/path")).await.unwrap();

        assert_eq!(source.checkout_root, Path::new("/checkout"));
        assert_eq!(source.branch.local_branch, "feature/status");
        assert_eq!(source.branch.head_branch, "published-status");
        assert_eq!(source.branch.owner.as_deref(), Some("contributor"));
        assert_eq!(source.default_branch.as_deref(), Some("main"));
        let requests = runner.requests();
        assert_eq!(requests[0].cwd, Path::new("/nested/path"));
        assert_eq!(requests[0].args, ["rev-parse", "--show-toplevel"]);
        assert_eq!(requests[4].args, ["remote", "get-url", "--push", "fork"]);
    }

    #[test]
    fn parses_supported_git_remote_forms() {
        for (url, expected) in [
            (
                "git@github.com:owner/repo.git",
                GitRemote {
                    host: "github.com".into(),
                    owner: "owner".into(),
                    repository: "repo".into(),
                },
            ),
            (
                "ssh://git@github.com/owner/repo.git",
                GitRemote {
                    host: "github.com".into(),
                    owner: "owner".into(),
                    repository: "repo".into(),
                },
            ),
            (
                "https://github.com/owner/repo",
                GitRemote {
                    host: "github.com".into(),
                    owner: "owner".into(),
                    repository: "repo".into(),
                },
            ),
            (
                "https://user@github.example.com/owner/repo.git/",
                GitRemote {
                    host: "github.example.com".into(),
                    owner: "owner".into(),
                    repository: "repo".into(),
                },
            ),
        ] {
            assert_eq!(parse_git_remote(url), Some(expected), "remote: {url}");
        }
    }

    #[test]
    fn rejects_unsupported_or_malformed_remotes() {
        for url in [
            "file:///tmp/repo",
            "/tmp/repo",
            "git://github.com/owner/repo.git",
            "https://github.com/owner",
            "https://github.com/owner/repo/extra",
            "",
        ] {
            assert_eq!(parse_git_remote(url), None, "remote: {url}");
        }
    }

    #[test]
    fn fork_remote_owner_selector_has_priority() {
        let context = BranchHeadContext::resolve(
            "local-name",
            Some("fork/published-name"),
            Some("fork"),
            Some("git@github.com:contributor/zeron.git"),
        );

        assert_eq!(context.host.as_deref(), Some("github.com"));
        assert_eq!(context.owner.as_deref(), Some("contributor"));
        assert_eq!(context.repository.as_deref(), Some("zeron"));
        assert_eq!(context.head_branch, "published-name");
        assert_eq!(
            context.head_selectors,
            ["contributor:published-name", "published-name"]
        );
    }

    #[test]
    fn branch_without_upstream_keeps_safe_local_fallback() {
        let context = BranchHeadContext::resolve(
            "feature/local",
            None,
            Some("origin"),
            Some("https://github.com/acme/zeron.git"),
        );

        assert_eq!(context.head_branch, "feature/local");
        assert_eq!(
            context.head_selectors,
            ["acme:feature/local", "feature/local"]
        );
    }

    #[test]
    fn matching_local_and_upstream_branches_are_deduplicated() {
        let context = BranchHeadContext::resolve(
            "feature/shared",
            Some("refs/remotes/origin/feature/shared"),
            Some("origin"),
            Some("https://github.com/acme/zeron"),
        );

        assert_eq!(context.remote_name.as_deref(), Some("origin"));
        assert_eq!(
            context.head_selectors,
            ["acme:feature/shared", "feature/shared"]
        );
    }

    type Script = dyn Fn(&ProcessRequest) -> Result<ProcessOutput, ProcessRunError> + Send + Sync;

    struct ScriptedRunner {
        script: Box<Script>,
        requests: Mutex<Vec<ProcessRequest>>,
    }

    impl ScriptedRunner {
        fn new(
            script: impl Fn(&ProcessRequest) -> Result<ProcessOutput, ProcessRunError>
            + Send
            + Sync
            + 'static,
        ) -> Arc<Self> {
            Arc::new(Self {
                script: Box::new(script),
                requests: Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait]
    impl ProcessRunner for ScriptedRunner {
        async fn run(&self, request: ProcessRequest) -> Result<ProcessOutput, ProcessRunError> {
            let output = (self.script)(&request);
            self.requests.lock().unwrap().push(request);
            output
        }
    }

    fn list_resolver(runner: Arc<ScriptedRunner>) -> ChangeRequestResolver {
        ChangeRequestResolver {
            inspector: GitCheckoutInspector::new(runner.clone()),
            github: GitHubCli::with_runner(runner),
        }
    }

    fn arg_after<'a>(request: &'a ProcessRequest, flag: &str) -> Option<&'a str> {
        let index = request.args.iter().position(|arg| arg == flag)?;
        request.args.get(index + 1).map(String::as_str)
    }

    fn list_row(
        number: u64,
        author: &str,
        updated: &str,
        extra: serde_json::Value,
    ) -> serde_json::Value {
        let mut row = serde_json::json!({
            "number": number,
            "title": format!("Change {number}"),
            "url": format!("https://github.com/acme/zeron/pull/{number}"),
            "author": {"login": author, "is_bot": false},
            "headRefName": format!("feature/{number}"),
            "baseRefName": "main",
            "state": "OPEN",
            "isDraft": false,
            "mergeable": "MERGEABLE",
            "reviewDecision": "",
            "additions": 10,
            "deletions": 2,
            "createdAt": "2026-09-01T00:00:00Z",
            "updatedAt": updated,
            "mergedAt": null,
            "reviewRequests": [],
            "labels": [],
            "statusCheckRollup": []
        });
        if let (Some(row), Some(extra)) = (row.as_object_mut(), extra.as_object()) {
            for (key, value) in extra {
                row.insert(key.clone(), value.clone());
            }
        }
        row
    }

    fn empty_search_or_viewer(
        request: &ProcessRequest,
    ) -> Option<Result<ProcessOutput, ProcessRunError>> {
        if request.args.first().map(String::as_str) != Some("api") {
            return None;
        }
        assert_eq!(arg_after(request, "--hostname"), Some("github.com"));
        Some(
            if request.args.get(1).map(String::as_str) == Some("graphql") {
                command_success(
                    r#"{"data":{"search":{"pageInfo":{"hasNextPage":false},"nodes":[]}}}"#,
                )
            } else {
                command_success("alice\n")
            },
        )
    }

    fn git_repo_script(request: &ProcessRequest) -> Option<Result<ProcessOutput, ProcessRunError>> {
        if request.program != "git" {
            return None;
        }
        Some(
            match request
                .args
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .as_slice()
            {
                ["rev-parse", "--show-toplevel"] => command_success("/repo\n"),
                ["remote", "-v"] => command_success(
                    "fork\tgit@github.com:alice/zeron.git (fetch)\n\
                 fork\tgit@github.com:alice/zeron.git (push)\n\
                 origin\thttps://github.com/acme/zeron.git (fetch)\n\
                 origin\thttps://github.com/acme/zeron.git (push)\n",
                ),
                other => panic!("unexpected git {other:?}"),
            },
        )
    }

    #[test]
    fn primary_fetch_remote_prefers_upstream_then_origin_then_first_by_name() {
        let both = "origin\thttps://h/o/a (fetch)\nupstream\thttps://h/u/a (fetch)\n";
        assert_eq!(primary_fetch_remote(both), Some("https://h/u/a"));
        let origin = "zed\thttps://h/z/a (fetch)\norigin\thttps://h/o/a (fetch)\norigin\thttps://h/x/a (push)\n";
        assert_eq!(primary_fetch_remote(origin), Some("https://h/o/a"));
        let others = "zed\thttps://h/z/a (fetch)\nbeta\thttps://h/b/a (fetch)\n";
        assert_eq!(primary_fetch_remote(others), Some("https://h/b/a"));
        assert_eq!(primary_fetch_remote("only\thttps://h/o/a (push)\n"), None);
    }

    #[tokio::test]
    async fn list_change_requests_merges_feed_authored_and_reviewing_reads() {
        let runner = ScriptedRunner::new(|request| {
            if let Some(output) = git_repo_script(request) {
                return output;
            }
            assert_eq!(request.program, "gh");
            assert_eq!(request.cwd, PathBuf::from("/repo"));
            assert!(
                request
                    .env
                    .contains(&("GH_PROMPT_DISABLED".into(), "1".into()))
            );
            if let Some(output) = empty_search_or_viewer(request) {
                return output;
            }
            assert_eq!(arg_after(request, "--repo"), Some("github.com/acme/zeron"));
            assert_eq!(arg_after(request, "--state"), Some("open"));
            assert_eq!(arg_after(request, "--limit"), Some("100"));
            assert_eq!(arg_after(request, "--json"), Some(GITHUB_LIST_FIELDS));
            let rows = if arg_after(request, "--author") == Some("alice") {
                vec![list_row(
                    1,
                    "alice",
                    "2026-09-03T00:00:00Z",
                    serde_json::json!({}),
                )]
            } else if arg_after(request, "--search")
                .is_some_and(|search| search.starts_with("review-requested:alice"))
            {
                vec![list_row(
                    2,
                    "bob",
                    "2026-09-04T00:00:00Z",
                    serde_json::json!({}),
                )]
            } else {
                assert_eq!(arg_after(request, "--search"), Some("sort:updated-desc"));
                vec![
                    list_row(
                        3,
                        "carol",
                        "2026-09-05T00:00:00Z",
                        serde_json::json!({
                            "isDraft": true,
                            "mergeable": "CONFLICTING",
                            "reviewDecision": "CHANGES_REQUESTED",
                            "labels": [{"name": " bug ", "color": "d73a4a"}, {"name": "  "}],
                            "statusCheckRollup": [
                                {"__typename": "CheckRun", "status": "COMPLETED", "conclusion": "SUCCESS"},
                                {"__typename": "StatusContext", "state": "FAILURE"}
                            ]
                        }),
                    ),
                    list_row(1, "alice", "2026-09-03T00:00:00Z", serde_json::json!({})),
                    serde_json::json!({"number": "not a row"}),
                ]
            };
            command_success(serde_json::to_vec(&rows).unwrap())
        });
        let listing = list_resolver(runner)
            .list_change_requests(
                &[PathBuf::from("/repo")],
                &ListChangeRequestsRequest::default(),
            )
            .await;

        assert!(listing.errors.is_empty(), "{:?}", listing.errors);
        assert_eq!(
            listing.viewers.get("github.com").map(String::as_str),
            Some("alice")
        );
        assert!(!listing.truncated);
        let numbers: Vec<u64> = listing.items.iter().map(|item| item.number).collect();
        assert_eq!(numbers, [3, 2, 1]);
        let carol = &listing.items[0];
        assert_eq!(carol.repository, "acme/zeron");
        assert_eq!(carol.host, "github.com");
        assert!(carol.is_draft);
        assert_eq!(carol.mergeability, ChangeRequestMergeability::Conflicting);
        assert_eq!(
            carol.review_decision,
            Some(ChangeRequestReviewDecision::ChangesRequested)
        );
        assert_eq!(carol.checks_state, Some(ChangeRequestChecksState::Failing));
        assert_eq!(
            carol.labels,
            [ChangeRequestLabel {
                name: "bug".into(),
                color: Some("d73a4a".into())
            }]
        );
        assert_eq!(
            carol
                .author
                .as_ref()
                .and_then(|author| author.avatar_url.as_deref()),
            Some("https://github.com/carol.png?size=80")
        );
        assert!(listing.items[1].viewer_review_requested);
        assert!(!listing.items[2].viewer_review_requested);
    }

    #[tokio::test]
    async fn list_change_requests_reports_an_unauthenticated_host_per_repository() {
        let runner = ScriptedRunner::new(|request| {
            if let Some(output) = git_repo_script(request) {
                return output;
            }
            assert_eq!(request.args.first().map(String::as_str), Some("api"));
            command_failure("To get started with GitHub CLI, please run:  gh auth login")
        });
        let listing = list_resolver(runner.clone())
            .list_change_requests(
                &[PathBuf::from("/repo")],
                &ListChangeRequestsRequest::default(),
            )
            .await;

        assert!(listing.items.is_empty());
        assert!(listing.viewers.is_empty());
        assert_eq!(
            listing.errors,
            [ChangeRequestListError {
                host: Some("github.com".into()),
                repository: Some("acme/zeron".into()),
                path: "/repo".into(),
                failure: ChangeRequestFailure::Authentication,
            }]
        );
        assert!(
            runner.requests.lock().unwrap().iter().all(|request| request
                .args
                .first()
                .map(String::as_str)
                != Some("pr"))
        );
    }

    #[tokio::test]
    async fn list_change_requests_reports_missing_gh_and_non_git_roots() {
        let runner = ScriptedRunner::new(|request| {
            if request.program == "git" && request.cwd == Path::new("/plain") {
                return command_failure("fatal: not a git repository");
            }
            if let Some(output) = git_repo_script(request) {
                return output;
            }
            Err(ProcessRunError::Spawn(io::ErrorKind::NotFound))
        });
        let listing = list_resolver(runner)
            .list_change_requests(
                &[PathBuf::from("/plain"), PathBuf::from("/repo")],
                &ListChangeRequestsRequest::default(),
            )
            .await;

        let failures: Vec<_> = listing.errors.iter().map(|error| error.failure).collect();
        assert_eq!(
            failures,
            [
                ChangeRequestFailure::RepositoryUnavailable,
                ChangeRequestFailure::CliUnavailable
            ]
        );
        assert_eq!(listing.errors[0].path, "/plain");
        assert_eq!(listing.errors[0].repository, None);
    }

    #[tokio::test]
    async fn list_change_requests_falls_back_to_an_unsorted_read_and_flags_truncation() {
        let runner = ScriptedRunner::new(|request| {
            if let Some(output) = git_repo_script(request) {
                return output;
            }
            if let Some(output) = empty_search_or_viewer(request) {
                return output;
            }
            assert_eq!(arg_after(request, "--limit"), Some("3"));
            let rows = match arg_after(request, "--search") {
                None => vec![
                    list_row(7, "bob", "2026-09-07T00:00:00Z", serde_json::json!({})),
                    list_row(6, "bob", "2026-09-06T00:00:00Z", serde_json::json!({})),
                    list_row(5, "bob", "2026-09-05T00:00:00Z", serde_json::json!({})),
                ],
                Some(_) => Vec::new(),
            };
            command_success(serde_json::to_vec(&rows).unwrap())
        });
        let request = ListChangeRequestsRequest {
            involvement: ChangeRequestInvolvement::All,
            limit: Some(2),
            ..Default::default()
        };
        let listing = list_resolver(runner.clone())
            .list_change_requests(&[PathBuf::from("/repo")], &request)
            .await;

        assert!(listing.truncated);
        let numbers: Vec<u64> = listing.items.iter().map(|item| item.number).collect();
        assert_eq!(numbers, [7, 6]);
        let unsorted_reads = runner
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.args.first().map(String::as_str) == Some("pr"))
            .filter(|request| arg_after(request, "--search").is_none())
            .count();
        assert_eq!(unsorted_reads, 1);
    }

    #[test]
    fn checks_rollup_matches_t3code_precedence() {
        let check = |status: Option<&str>, conclusion: Option<&str>, state: Option<&str>| GhCheck {
            status: status.map(str::to_owned),
            conclusion: conclusion.map(str::to_owned),
            state: state.map(str::to_owned),
            ..Default::default()
        };
        assert_eq!(checks_rollup(&[]), None);
        assert_eq!(checks_rollup(&[check(None, Some("SKIPPED"), None)]), None);
        assert_eq!(
            checks_rollup(&[check(Some("COMPLETED"), Some("SUCCESS"), None)]),
            Some(ChangeRequestChecksState::Passing)
        );
        assert_eq!(
            checks_rollup(&[
                check(Some("COMPLETED"), Some("SUCCESS"), None),
                check(Some("IN_PROGRESS"), None, None),
            ]),
            Some(ChangeRequestChecksState::Pending)
        );
        assert_eq!(
            checks_rollup(&[
                check(Some("IN_PROGRESS"), None, None),
                check(Some("COMPLETED"), Some("CANCELLED"), None),
            ]),
            Some(ChangeRequestChecksState::Failing)
        );
        assert_eq!(
            checks_rollup(&[check(None, None, Some("PENDING"))]),
            Some(ChangeRequestChecksState::Pending)
        );
    }

    fn detail_json() -> serde_json::Value {
        let mut row = list_row(
            742,
            "carol",
            "2026-10-03T00:00:00Z",
            serde_json::json!({
                "additions": 601,
                "deletions": 95,
                "reviewRequests": [{"__typename": "User", "login": "alice"}, {"__typename": "Team", "name": "core"}],
                "statusCheckRollup": [
                    {"__typename": "CheckRun", "name": "build", "workflowName": "CI", "status": "COMPLETED", "conclusion": "FAILURE", "completedAt": "2026-10-02T10:00:00Z", "detailsUrl": "https://ci/old"},
                    {"__typename": "CheckRun", "name": "build", "workflowName": "CI", "status": "COMPLETED", "conclusion": "SUCCESS", "completedAt": "2026-10-02T11:00:00Z", "detailsUrl": "https://ci/new"},
                    {"__typename": "CheckRun", "name": "test", "workflowName": "UI", "status": "IN_PROGRESS", "conclusion": "", "startedAt": "2026-10-02T11:00:00Z"},
                    {"__typename": "CheckRun", "name": "test", "workflowName": "Engine", "status": "COMPLETED", "conclusion": "SKIPPED"},
                    {"__typename": "StatusContext", "context": "vercel", "state": "SUCCESS", "targetUrl": "https://vercel/x"}
                ]
            }),
        );
        let extra = serde_json::json!({
            "body": "## What\nA change.",
            "changedFiles": 6,
            "closedAt": null,
            "isCrossRepository": true,
            "comments": [
                {"id": "IC_2", "author": {"login": "dave"}, "body": "second", "createdAt": "2026-10-02T12:00:00Z", "url": "https://github.com/acme/zeron/pull/742#c2"},
                {"id": "IC_1", "author": {"login": "erin"}, "body": "first", "createdAt": "2026-10-01T12:00:00Z", "url": "https://github.com/acme/zeron/pull/742#c1"}
            ],
            "reviews": [
                {"id": "R_1", "author": {"login": "bob"}, "body": "", "state": "APPROVED", "submittedAt": "2026-10-01T13:00:00Z"},
                {"id": "R_2", "author": {"login": "bob"}, "body": "", "state": "COMMENTED", "submittedAt": "2026-10-01T14:00:00Z"},
                {"id": "R_3", "author": {"login": "frank"}, "body": "Needs work", "state": "CHANGES_REQUESTED", "submittedAt": "2026-10-02T20:00:00Z"},
                {"id": "R_4", "author": {"login": "gina"}, "body": "pending", "state": "PENDING"}
            ],
            "latestReviews": [
                {"author": {"login": "bob"}, "body": "", "state": "APPROVED", "submittedAt": "2026-10-01T13:00:00Z"},
                {"author": {"login": "frank"}, "body": "Needs work", "state": "CHANGES_REQUESTED", "submittedAt": "2026-10-02T20:00:00Z"},
                {"author": {"login": "hank"}, "body": "", "state": "DISMISSED", "submittedAt": "2026-10-02T20:00:00Z"}
            ],
            "commits": [
                {"oid": "aaaaaaa1111111", "messageHeadline": "First", "committedDate": "2026-10-01T09:00:00Z"},
                {"oid": "bbbbbbb2222222", "messageHeadline": "Second", "committedDate": "2026-10-02T09:00:00Z"}
            ]
        });
        if let (Some(row), Some(extra)) = (row.as_object_mut(), extra.as_object()) {
            for (key, value) in extra {
                row.insert(key.clone(), value.clone());
            }
        }
        row
    }

    #[tokio::test]
    async fn get_change_request_maps_checks_reviewers_and_comments() {
        let runner = ScriptedRunner::new(|request| {
            if let Some(output) = git_repo_script(request) {
                return output;
            }
            if request.args.first().map(String::as_str) == Some("api") {
                return command_success("alice\n");
            }
            assert_eq!(&request.args[..3], ["pr", "view", "742"]);
            assert_eq!(arg_after(request, "--repo"), Some("github.com/acme/zeron"));
            assert_eq!(arg_after(request, "--json"), Some(GITHUB_DETAIL_FIELDS));
            command_success(serde_json::to_vec(&detail_json()).unwrap())
        });
        let detail = list_resolver(runner)
            .get_change_request(
                &[PathBuf::from("/repo")],
                &GetChangeRequestRequest {
                    host: "GitHub.com".into(),
                    repository: "ACME/zeron".into(),
                    number: 742,
                },
            )
            .await
            .unwrap();

        assert_eq!(detail.item.number, 742);
        assert_eq!(detail.item.additions, 601);
        assert!(detail.item.viewer_review_requested);
        assert_eq!(
            detail.item.checks_state,
            Some(ChangeRequestChecksState::Pending)
        );
        assert_eq!(detail.body, "## What\nA change.");
        assert_eq!(detail.changed_files, 6);
        assert_eq!(detail.commit_count, 2);
        assert_eq!(
            detail
                .commits
                .iter()
                .map(|commit| (commit.oid.as_str(), commit.headline.as_str()))
                .collect::<Vec<_>>(),
            [("aaaaaaa1111111", "First"), ("bbbbbbb2222222", "Second")]
        );
        assert_eq!(detail.comment_count, 2);

        let checks: Vec<(&str, ChangeRequestCheckStatus, Option<&str>)> = detail
            .checks
            .iter()
            .map(|check| (check.name.as_str(), check.status, check.url.as_deref()))
            .collect();
        assert_eq!(
            checks,
            [
                (
                    "build",
                    ChangeRequestCheckStatus::Success,
                    Some("https://ci/new")
                ),
                ("UI / test", ChangeRequestCheckStatus::Pending, None),
                ("Engine / test", ChangeRequestCheckStatus::Skipped, None),
                (
                    "vercel",
                    ChangeRequestCheckStatus::Success,
                    Some("https://vercel/x")
                ),
            ]
        );

        let reviewers: Vec<(&str, Option<ChangeRequestVerdict>, bool)> = detail
            .reviewers
            .iter()
            .map(|reviewer| (reviewer.login.as_str(), reviewer.verdict, reviewer.stale))
            .collect();
        assert_eq!(
            reviewers,
            [
                ("alice", None, false),
                ("bob", Some(ChangeRequestVerdict::Approved), true),
                ("frank", Some(ChangeRequestVerdict::ChangesRequested), false),
            ]
        );

        let comments: Vec<(&str, ChangeRequestCommentKind)> = detail
            .comments
            .iter()
            .map(|comment| (comment.id.as_str(), comment.kind))
            .collect();
        assert_eq!(
            comments,
            [
                ("IC_1", ChangeRequestCommentKind::Comment),
                ("R_1", ChangeRequestCommentKind::Review),
                ("IC_2", ChangeRequestCommentKind::Comment),
                ("R_3", ChangeRequestCommentKind::Review),
            ]
        );
    }

    #[tokio::test]
    async fn get_change_request_refuses_a_repository_outside_the_spaces() {
        let runner = ScriptedRunner::new(|request| {
            if let Some(output) = git_repo_script(request) {
                return output;
            }
            panic!(
                "no gh call may run for a foreign repository: {:?}",
                request.args
            );
        });
        let error = list_resolver(runner)
            .get_change_request(
                &[PathBuf::from("/repo")],
                &GetChangeRequestRequest {
                    host: "github.com".into(),
                    repository: "someone/else".into(),
                    number: 1,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error, ChangeRequestError::UnsupportedRepository);
    }

    #[test]
    fn files_patch_synthesizes_git_headers_and_records_withheld_files() {
        let raw = serde_json::json!([
            {"filename": "src/new.rs", "status": "added", "additions": 1, "deletions": 0, "patch": "@@ -0,0 +1 @@\n+fn new() {}"},
            {"filename": "src/gone.rs", "status": "removed", "additions": 0, "deletions": 1, "patch": "@@ -1 +0,0 @@\n-fn gone() {}\n"},
            {"filename": "src/after.rs", "previous_filename": "src/before.rs", "status": "renamed", "additions": 0, "deletions": 0},
            {"filename": "assets/logo.png", "status": "modified", "additions": 3, "deletions": 2},
            {"status": "modified"}
        ]);
        let (slice, raw_count) = files_patch(raw.to_string().as_bytes()).unwrap();
        assert_eq!(raw_count, 5);
        assert_eq!(
            slice.patch,
            "diff --git a/src/new.rs b/src/new.rs\n\
             new file mode 100644\n\
             --- /dev/null\n\
             +++ b/src/new.rs\n\
             @@ -0,0 +1 @@\n\
             +fn new() {}\n\
             diff --git a/src/gone.rs b/src/gone.rs\n\
             deleted file mode 100644\n\
             --- a/src/gone.rs\n\
             +++ /dev/null\n\
             @@ -1 +0,0 @@\n\
             -fn gone() {}\n\
             diff --git a/src/before.rs b/src/after.rs\n\
             rename from src/before.rs\n\
             rename to src/after.rs\n\
             --- a/src/before.rs\n\
             +++ b/src/after.rs\n\
             diff --git a/assets/logo.png b/assets/logo.png\n\
             --- a/assets/logo.png\n\
             +++ b/assets/logo.png\n"
        );
        assert!(slice.truncated);
        assert_eq!(
            slice.omitted_file_stats,
            [ChangeRequestFileStat {
                path: "assets/logo.png".into(),
                additions: 3,
                deletions: 2,
            }]
        );
        assert_eq!(slice.next_cursor, None);
    }

    #[tokio::test]
    async fn get_change_request_diff_pages_files_and_scopes_commits() {
        let runner = ScriptedRunner::new(|request| {
            if let Some(output) = git_repo_script(request) {
                return output;
            }
            assert_eq!(request.args[..3], ["api", "--hostname", "github.com"]);
            assert_eq!(request.output_limit, GITHUB_LIST_OUTPUT_LIMIT);
            let files: Vec<serde_json::Value> = (0..DIFF_FILES_PAGE_SIZE)
                .map(|ix| serde_json::json!({"filename": format!("f{ix}.rs"), "status": "modified", "patch": "@@ -1 +1 @@\n-a\n+b"}))
                .collect();
            match request.args[3].as_str() {
                "repos/acme/zeron/pulls/7/files?per_page=100&page=2" => {
                    command_success(serde_json::to_vec(&files).unwrap())
                }
                "repos/acme/zeron/commits/abc1234?per_page=100&page=1" => {
                    assert_eq!(arg_after(request, "--jq"), Some(".files // []"));
                    command_success("[]")
                }
                other => panic!("unexpected gh api path {other}"),
            }
        });
        let resolver = list_resolver(runner.clone());
        let roots = [PathBuf::from("/repo")];
        let page = resolver
            .get_change_request_diff(
                &roots,
                &GetChangeRequestDiffRequest {
                    host: "github.com".into(),
                    repository: "acme/zeron".into(),
                    number: 7,
                    cursor: Some("2".into()),
                    commit: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(page.next_cursor.as_deref(), Some("3"));
        assert_eq!(
            page.patch.matches("diff --git").count(),
            DIFF_FILES_PAGE_SIZE
        );
        let commit = resolver
            .get_change_request_diff(
                &roots,
                &GetChangeRequestDiffRequest {
                    host: "github.com".into(),
                    repository: "acme/zeron".into(),
                    number: 7,
                    cursor: None,
                    commit: Some("abc1234".into()),
                },
            )
            .await
            .unwrap();
        assert_eq!(commit, ChangeRequestDiffSlice::default());
        for (cursor, commit) in [
            (Some("0"), None),
            (None, Some("HEAD~1")),
            (None, Some("abc")),
        ] {
            let error = resolver
                .get_change_request_diff(
                    &roots,
                    &GetChangeRequestDiffRequest {
                        host: "github.com".into(),
                        repository: "acme/zeron".into(),
                        number: 7,
                        cursor: cursor.map(str::to_owned),
                        commit: commit.map(str::to_owned),
                    },
                )
                .await
                .unwrap_err();
            assert_eq!(error, ChangeRequestError::Decode);
        }
        let foreign = resolver
            .get_change_request_diff(
                &roots,
                &GetChangeRequestDiffRequest {
                    host: "github.com".into(),
                    repository: "someone/else".into(),
                    number: 7,
                    cursor: None,
                    commit: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(foreign, ChangeRequestError::UnsupportedRepository);
        let gh_calls = runner
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.program == "gh")
            .count();
        assert_eq!(gh_calls, 2);
    }

    fn stdin_json(request: &ProcessRequest) -> serde_json::Value {
        serde_json::from_slice(request.stdin.as_deref().expect("stdin")).expect("json stdin")
    }

    fn reaction(content: &str, count: u64, viewer: bool) -> serde_json::Value {
        serde_json::json!({
            "content": content,
            "viewerHasReacted": viewer,
            "reactors": {"totalCount": count, "nodes": [{"login": "bob"}]}
        })
    }

    fn activity_json(thread_page: serde_json::Value) -> serde_json::Value {
        serde_json::json!({"data": {
            "viewer": {"login": "alice"},
            "repository": {"pullRequest": {
                "id": "PR_1",
                "viewerCanUpdate": true,
                "reactionGroups": [reaction("HEART", 2, true), reaction("EYES", 0, false)],
                "reviewThreads": thread_page,
                "comments": {"totalCount": 1, "nodes": [
                    {"id": "IC_1", "author": {"login": "bob"}, "body": "first", "createdAt": "2026-10-01T10:00:00Z", "url": "https://github.com/acme/zeron/pull/7#c1", "reactionGroups": [reaction("THUMBS_UP", 1, false)]}
                ]},
                "reviews": {"totalCount": 4, "nodes": [
                    {"id": "R_1", "author": {"login": "carol"}, "body": "", "state": "COMMENTED", "submittedAt": "2026-10-01T11:00:00Z"},
                    {"id": "R_2", "author": {"login": "carol"}, "body": "", "state": "APPROVED", "submittedAt": "2026-10-01T12:00:00Z"},
                    {"id": "R_3", "author": {"login": "dave"}, "body": "<!-- bot -->", "state": "DISMISSED", "submittedAt": "2026-10-01T09:00:00Z"},
                    {"id": "R_4", "author": {"login": "dave"}, "body": "pending", "state": "PENDING"}
                ]},
                "reviewDismissals": {"nodes": [{"dismissalMessage": "stale now", "review": {"id": "R_3"}}, {}]},
                "commits": {"nodes": [
                    {"commit": {"oid": "aaa1111", "messageHeadline": "Add", "committedDate": "2026-09-30T10:00:00Z", "additions": 5, "deletions": 1, "parents": {"totalCount": 1}, "authors": {"nodes": [{"name": "Bob", "user": {"login": "bob"}}]}}},
                    {"commit": {"oid": "bbb2222", "messageHeadline": "Merge main", "committedDate": "2026-10-02T10:00:00Z", "additions": 90, "deletions": 9, "parents": {"totalCount": 2}, "authors": {"nodes": [{"name": "No Login"}]}}}
                ]}
            }}
        }})
    }

    fn thread(id: &str, at: &str, more: bool) -> serde_json::Value {
        serde_json::json!({
            "id": id, "isResolved": id == "T_2", "isOutdated": false, "path": "src/lib.rs", "line": 12,
            "comments": {"totalCount": if more { 11 } else { 1 }, "pageInfo": {"hasNextPage": more}, "nodes": [
                {"id": format!("{id}_c"), "author": {"login": "erin"}, "body": "nit", "createdAt": at, "url": null}
            ]}
        })
    }

    #[tokio::test]
    async fn activity_reads_one_graphql_page_then_follows_review_threads() {
        let runner = ScriptedRunner::new(|request| {
            if let Some(output) = git_repo_script(request) {
                return output;
            }
            assert_eq!(
                request.args,
                ["api", "graphql", "--hostname", "github.com", "--input", "-"]
            );
            let input = stdin_json(request);
            assert_eq!(input["variables"]["owner"], "acme");
            assert_eq!(input["variables"]["name"], "zeron");
            assert_eq!(input["variables"]["number"], 7);
            let page = match input["variables"]["cursor"].as_str() {
                None => serde_json::json!({
                    "pageInfo": {"hasNextPage": true, "endCursor": "C1"},
                    "nodes": [thread("T_1", "2026-10-01T13:00:00Z", false)]
                }),
                Some("C1") => {
                    return command_success(
                        serde_json::to_vec(&serde_json::json!({"data": {"repository": {"pullRequest": {"reviewThreads": {
                            "pageInfo": {"hasNextPage": false},
                            "nodes": [thread("T_2", "2026-10-01T08:00:00Z", true)]
                        }}}}}))
                        .unwrap(),
                    );
                }
                other => panic!("unexpected cursor {other:?}"),
            };
            command_success(serde_json::to_vec(&activity_json(page)).unwrap())
        });
        let activity = list_resolver(runner.clone())
            .get_change_request_activity(
                &[PathBuf::from("/repo")],
                &GetChangeRequestRequest {
                    host: "github.com".into(),
                    repository: "acme/zeron".into(),
                    number: 7,
                },
            )
            .await
            .unwrap();
        assert_eq!(activity.viewer, "alice");
        assert_eq!(activity.subject_id, "PR_1");
        assert!(activity.viewer_can_update);
        assert_eq!(
            activity
                .reactions
                .iter()
                .map(|reaction| (
                    reaction.content,
                    reaction.count,
                    reaction.viewer_has_reacted
                ))
                .collect::<Vec<_>>(),
            [(ChangeRequestReactionContent::Heart, 2, true)]
        );
        let ids: Vec<(&str, ChangeRequestCommentKind)> = activity
            .comments
            .iter()
            .map(|comment| (comment.id.as_str(), comment.kind))
            .collect();
        assert_eq!(
            ids,
            [
                ("T_2_c", ChangeRequestCommentKind::ReviewComment),
                ("R_3", ChangeRequestCommentKind::Review),
                ("IC_1", ChangeRequestCommentKind::Comment),
                ("R_2", ChangeRequestCommentKind::Review),
                ("T_1_c", ChangeRequestCommentKind::ReviewComment),
            ]
        );
        let dismissed = &activity.comments[1];
        assert_eq!(dismissed.body, "stale now");
        assert_eq!(dismissed.review_state.as_deref(), Some("DISMISSED"));
        let remark = &activity.comments[0];
        assert_eq!(remark.path.as_deref(), Some("src/lib.rs"));
        assert_eq!(remark.line, Some(12));
        assert_eq!(remark.thread_id.as_deref(), Some("T_2"));
        assert!(remark.thread_resolved);
        assert_eq!(activity.comments[2].reactions.len(), 1);
        assert_eq!(activity.comment_count, 1 + 2 + 1 + 11);
        assert!(activity.truncated, "a thread with more comments truncates");
        assert_eq!(activity.commits.len(), 2);
        assert_eq!(activity.commits[0].additions, Some(5));
        assert_eq!(activity.commits[0].author.as_ref().unwrap().login, "bob");
        assert_eq!(
            activity.commits[1].additions, None,
            "merge commits carry no stats"
        );
        assert_eq!(
            activity.commits[1].author.as_ref().unwrap().login,
            "No Login"
        );
    }

    #[tokio::test]
    async fn comments_and_reactions_send_user_text_on_stdin_and_stay_in_scope() {
        const BODY: &str = "--body $(touch /tmp/zeron-pwned)";
        let runner = ScriptedRunner::new(|request| {
            if let Some(output) = git_repo_script(request) {
                return output;
            }
            if request.args.first().map(String::as_str) == Some("pr") {
                assert_eq!(
                    request.args,
                    [
                        "pr",
                        "comment",
                        "7",
                        "--repo",
                        "github.com/acme/zeron",
                        "--body-file",
                        "-"
                    ]
                );
                assert_eq!(request.stdin.as_deref(), Some(BODY.as_bytes()));
                return command_success("");
            }
            let input = stdin_json(request);
            let query = input["query"].as_str().unwrap();
            if query.starts_with("mutation") {
                assert!(query.contains("addReaction"));
                assert_eq!(input["variables"]["subjectId"], "PR_1");
                assert_eq!(input["variables"]["content"], "ROCKET");
                return command_success(
                    r#"{"data":{"addReaction":{"reaction":{"content":"ROCKET"}}}}"#,
                );
            }
            let node = match input["variables"]["subjectId"].as_str() {
                Some("IC_other") => {
                    serde_json::json!({"id": "IC_other", "pullRequest": {"id": "PR_9"}})
                }
                _ => serde_json::Value::Null,
            };
            command_success(
                serde_json::to_vec(&serde_json::json!({"data": {
                    "repository": {"pullRequest": {"id": "PR_1"}},
                    "node": node
                }}))
                .unwrap(),
            )
        });
        let resolver = list_resolver(runner.clone());
        let roots = [PathBuf::from("/repo")];
        let comment = |body: &str| PostChangeRequestCommentRequest {
            host: "github.com".into(),
            repository: "acme/zeron".into(),
            number: 7,
            body: body.into(),
        };
        resolver
            .post_change_request_comment(&roots, &comment(BODY))
            .await
            .unwrap();
        assert!(
            resolver
                .post_change_request_comment(&roots, &comment("  \n"))
                .await
                .is_err()
        );
        let reaction = |subject: Option<&str>| SetChangeRequestReactionRequest {
            host: "github.com".into(),
            repository: "acme/zeron".into(),
            number: 7,
            subject_id: subject.map(str::to_owned),
            content: ChangeRequestReactionContent::Rocket,
            add: true,
        };
        resolver
            .set_change_request_reaction(&roots, &reaction(None))
            .await
            .unwrap();
        let foreign = resolver
            .set_change_request_reaction(&roots, &reaction(Some("IC_other")))
            .await
            .unwrap_err();
        assert_eq!(foreign, ChangeRequestError::UnsupportedRepository);
        let requests = runner.requests.lock().unwrap();
        let mutations = requests
            .iter()
            .filter(|request| {
                request
                    .stdin
                    .as_deref()
                    .is_some_and(|stdin| String::from_utf8_lossy(stdin).contains("mutation"))
            })
            .count();
        assert_eq!(
            mutations, 1,
            "the out-of-scope subject never reaches a mutation"
        );
        assert!(
            requests
                .iter()
                .all(|request| !request.args.iter().any(|arg| arg.contains("zeron-pwned"))),
            "user text never rides argv"
        );
    }

    #[test]
    fn renders_empty_ignores_html_comments_only() {
        assert!(renders_empty("  <!-- a -->\n<!-- b -->  "));
        assert!(renders_empty("<!-- unterminated"));
        assert!(!renders_empty("<!-- a --> hi"));
    }

    fn permissions_json(permission: &str, state: &str) -> serde_json::Value {
        serde_json::json!({"data": {"repository": {
            "mergeCommitAllowed": true,
            "squashMergeAllowed": true,
            "rebaseMergeAllowed": false,
            "viewerPermission": permission,
            "defaultBranchRef": {"name": "main"},
            "pullRequest": {
                "state": state,
                "viewerCanUpdate": true,
                "viewerDidAuthor": false,
                "headRefName": "feature/x",
                "headRepositoryOwner": {"login": "fork"}
            }
        }}})
    }

    fn action_runner(
        permission: &'static str,
        merge_stderr: Option<&'static str>,
    ) -> Arc<ScriptedRunner> {
        ScriptedRunner::new(move |request| {
            if let Some(output) = git_repo_script(request) {
                return output;
            }
            if request.args[..2] == ["api", "graphql"] {
                let input = stdin_json(request);
                let query = input["query"].as_str().unwrap();
                if query.contains("viewerPermission") {
                    return command_success(
                        serde_json::to_vec(&permissions_json(permission, "OPEN")).unwrap(),
                    );
                }
                if query.contains("compare(headRef") {
                    assert_eq!(input["variables"]["headRef"], "fork:feature/x");
                    return command_success(
                        r#"{"data":{"repository":{"pullRequest":{"viewerCanUpdateBranch":true,"baseRef":{"compare":{"behindBy":3}}}}}}"#,
                    );
                }
                panic!("unexpected graphql {query}");
            }
            match request.args[1].as_str() {
                "comment" => command_success(""),
                "merge" | "close" => match merge_stderr {
                    Some(stderr) => command_failure(stderr),
                    None => command_success(""),
                },
                other => panic!("unexpected gh pr {other}"),
            }
        })
    }

    fn action(action: ChangeRequestAction, comment: Option<&str>) -> RunChangeRequestActionRequest {
        RunChangeRequestActionRequest {
            host: "github.com".into(),
            repository: "acme/zeron".into(),
            number: 7,
            action,
            comment: comment.map(str::to_owned),
        }
    }

    #[tokio::test]
    async fn permissions_follow_t3code_rules_and_read_the_base_comparison() {
        let runner = action_runner("WRITE", None);
        let permissions = list_resolver(runner)
            .get_change_request_permissions(
                &[PathBuf::from("/repo")],
                &GetChangeRequestRequest {
                    host: "github.com".into(),
                    repository: "acme/zeron".into(),
                    number: 7,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            permissions,
            ChangeRequestPermissions {
                merge_methods: vec![
                    ChangeRequestMergeMethod::Merge,
                    ChangeRequestMergeMethod::Squash
                ],
                can_write: true,
                can_triage: true,
                can_update: true,
                did_author: false,
                can_update_branch: true,
                behind_by: Some(3),
                default_branch: Some("main".into()),
            }
        );
        let triage = to_permissions(
            &RepositoryFlags {
                merge: false,
                squash: false,
                rebase: true,
                permission: "TRIAGE".into(),
            },
            &GhPermissionsPull {
                state: "OPEN".into(),
                viewer_can_update: None,
                viewer_did_author: Some(true),
                head_ref_name: String::new(),
                head_repository_owner: None,
            },
        );
        assert!(!triage.can_write && triage.can_triage && triage.can_update && triage.did_author);
    }

    #[tokio::test]
    async fn actions_are_gated_post_the_comment_first_and_return_host_words() {
        let roots = [PathBuf::from("/repo")];
        let runner = action_runner("WRITE", None);
        list_resolver(runner.clone())
            .run_change_request_action(
                &roots,
                &action(ChangeRequestAction::Close, Some("closing: superseded")),
            )
            .await
            .unwrap();
        let writes: Vec<Vec<String>> = runner
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.program == "gh" && request.args[0] == "pr")
            .map(|request| request.args.clone())
            .collect();
        assert_eq!(
            writes,
            [
                vec![
                    "pr",
                    "comment",
                    "7",
                    "--repo",
                    "github.com/acme/zeron",
                    "--body-file",
                    "-"
                ],
                vec!["pr", "close", "7", "--repo", "github.com/acme/zeron"],
            ]
            .map(|args| args.into_iter().map(str::to_owned).collect::<Vec<_>>())
        );

        let refused = list_resolver(action_runner(
            "WRITE",
            Some("GraphQL: Pull request is not mergeable (mergePullRequest)\n"),
        ))
        .run_change_request_action(
            &roots,
            &action(
                ChangeRequestAction::Merge {
                    method: ChangeRequestMergeMethod::Squash,
                },
                None,
            ),
        )
        .await
        .unwrap_err();
        assert_eq!(
            refused,
            ActionError::Refused(
                "GraphQL: Pull request is not mergeable (mergePullRequest)".into()
            )
        );

        for (permission, request, message) in [
            (
                "READ",
                ChangeRequestAction::Merge {
                    method: ChangeRequestMergeMethod::Merge,
                },
                "You need write access on this repository to merge.",
            ),
            (
                "WRITE",
                ChangeRequestAction::Merge {
                    method: ChangeRequestMergeMethod::Rebase,
                },
                "This host cannot merge with the rebase strategy.",
            ),
        ] {
            let runner = action_runner(permission, None);
            let error = list_resolver(runner.clone())
                .run_change_request_action(&roots, &action(request, None))
                .await
                .unwrap_err();
            assert_eq!(error, ActionError::Refused(message.into()));
            assert!(
                runner
                    .requests
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|request| request.args[0] != "pr"),
                "a refused action never reaches gh pr"
            );
        }
    }

    #[test]
    fn candidates_put_applied_and_requested_first() {
        let labels: GhLabelCandidatesRepository = serde_json::from_value(serde_json::json!({
            "labels": {"pageInfo": {"hasNextPage": true}, "nodes": [
                {"name": "bug", "color": "d73a4a", "description": "Something is broken"},
                {"name": "docs", "color": "0075ca", "description": ""}
            ]},
            "pullRequest": {"labels": {"nodes": [{"name": "docs"}, {"name": "old-label"}]}}
        }))
        .unwrap();
        let labels = to_label_candidates(labels);
        assert!(labels.truncated);
        assert_eq!(
            labels
                .labels
                .iter()
                .map(|label| (
                    label.name.as_str(),
                    label.applied,
                    label.description.is_some()
                ))
                .collect::<Vec<_>>(),
            [
                ("old-label", true, false),
                ("bug", false, true),
                ("docs", true, false)
            ]
        );
        let reviewers: GhReviewerCandidatesRepository = serde_json::from_value(serde_json::json!({
            "assignableUsers": {"pageInfo": {"hasNextPage": false}, "nodes": [
                {"login": "alice", "name": "Alice"},
                {"login": "Bob", "name": null},
                {"login": "carol", "name": "Carol"}
            ]},
            "pullRequest": {
                "author": {"login": "alice"},
                "reviewRequests": {"nodes": [
                    {"requestedReviewer": {"slug": "core", "name": "Core"}},
                    {"requestedReviewer": {"login": "bob", "name": "Bob B"}}
                ]}
            }
        }))
        .unwrap();
        let reviewers = to_reviewer_candidates(reviewers);
        assert_eq!(
            reviewers
                .reviewers
                .iter()
                .map(|reviewer| (reviewer.login.as_str(), reviewer.team, reviewer.requested))
                .collect::<Vec<_>>(),
            [
                ("core", true, true),
                ("bob", false, true),
                ("carol", false, false)
            ]
        );
    }

    #[tokio::test]
    async fn labels_need_triage_and_encode_names_in_the_delete_path() {
        let runner = ScriptedRunner::new(|request| {
            if let Some(output) = git_repo_script(request) {
                return output;
            }
            if request.args[..2] == ["api", "graphql"] {
                let input = stdin_json(request);
                let query = input["query"].as_str().unwrap();
                if query.contains("viewerPermission") {
                    return command_success(
                        serde_json::to_vec(&permissions_json("TRIAGE", "CLOSED")).unwrap(),
                    );
                }
                panic!("unexpected graphql");
            }
            command_success("")
        });
        list_resolver(runner.clone())
            .set_change_request_label(
                &[PathBuf::from("/repo")],
                &SetChangeRequestLabelRequest {
                    host: "github.com".into(),
                    repository: "acme/zeron".into(),
                    number: 7,
                    name: "good first issue/ä".into(),
                    add: false,
                },
            )
            .await
            .unwrap();
        let delete = runner
            .requests
            .lock()
            .unwrap()
            .iter()
            .find(|request| request.args.iter().any(|arg| arg == "DELETE"))
            .map(|request| request.args.last().unwrap().clone())
            .unwrap();
        assert_eq!(
            delete,
            "repos/acme/zeron/issues/7/labels/good%20first%20issue%2F%C3%A4"
        );
        assert_eq!(
            workflow_run_id("https://github.com/a/b/actions/runs/123/job/9"),
            Some("123".into())
        );
        assert_eq!(
            workflow_run_id("https://github.com/a/b/actions/runs/x"),
            None
        );
    }

    #[tokio::test]
    async fn title_edits_refuse_blank_titles_and_omit_untouched_fields() {
        let runner = ScriptedRunner::new(|request| {
            if let Some(output) = git_repo_script(request) {
                return output;
            }
            let input = stdin_json(request);
            let query = input["query"].as_str().unwrap();
            if query.starts_with("mutation") {
                assert!(input["variables"].get("title").is_none());
                assert_eq!(input["variables"]["body"], "new body");
                assert_eq!(input["variables"]["pullRequestId"], "PR_1");
                return command_success(
                    r#"{"data":{"updatePullRequest":{"pullRequest":{"id":"PR_1"}}}}"#,
                );
            }
            command_success(r#"{"data":{"repository":{"pullRequest":{"id":"PR_1"}}}}"#)
        });
        let resolver = list_resolver(runner);
        let request = |title: Option<&str>, body: Option<&str>| UpdateChangeRequestRequest {
            host: "github.com".into(),
            repository: "acme/zeron".into(),
            number: 7,
            title: title.map(str::to_owned),
            body: body.map(str::to_owned),
        };
        let roots = [PathBuf::from("/repo")];
        assert_eq!(
            resolver
                .update_change_request(&roots, &request(Some("  "), None))
                .await
                .unwrap_err(),
            ActionError::Refused("A title cannot be empty.".into())
        );
        resolver
            .update_change_request(&roots, &request(None, Some("new body")))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn reviews_and_threads_send_text_on_stdin_and_stay_in_scope() {
        let runner = ScriptedRunner::new(|request| {
            if let Some(output) = git_repo_script(request) {
                return output;
            }
            if request.args[..2] == ["api", "graphql"] {
                let input = stdin_json(request);
                let query = input["query"].as_str().unwrap();
                if query.contains("viewerPermission") {
                    let mut json = permissions_json("WRITE", "OPEN");
                    json["data"]["repository"]["pullRequest"]["viewerDidAuthor"] =
                        serde_json::Value::Bool(input["variables"]["number"] == 9);
                    return command_success(serde_json::to_vec(&json).unwrap());
                }
                if query.contains("compare(headRef") {
                    return command_success(r#"{"data":{"repository":{"pullRequest":{}}}}"#);
                }
                if query.contains("PullRequestReviewThread") {
                    let owner = if input["variables"]["subjectId"] == "T_7" {
                        "PR_7"
                    } else {
                        "PR_other"
                    };
                    return command_success(
                        serde_json::to_vec(&serde_json::json!({"data": {
                            "repository": {"pullRequest": {"id": "PR_7"}},
                            "node": {"id": input["variables"]["subjectId"], "pullRequest": {"id": owner}}
                        }}))
                        .unwrap(),
                    );
                }
                assert!(query.starts_with("mutation"), "{query}");
                return command_success(r#"{"data":{}}"#);
            }
            let input = stdin_json(request);
            assert_eq!(request.args.last().map(String::as_str), Some("-"));
            assert_eq!(input["event"], "REQUEST_CHANGES");
            assert_eq!(input["body"], "please fix");
            assert_eq!(
                input["comments"],
                serde_json::json!([{"path": "src/a.rs", "line": 4, "side": "LEFT", "body": "gone?"}])
            );
            command_success("{}")
        });
        let resolver = list_resolver(runner.clone());
        let roots = [PathBuf::from("/repo")];
        let review = |number: u64,
                      event,
                      body: &str,
                      comments: Vec<zeron_proto::ChangeRequestReviewComment>| {
            SubmitChangeRequestReviewRequest {
                host: "github.com".into(),
                repository: "acme/zeron".into(),
                number,
                event,
                body: body.into(),
                comments,
            }
        };
        resolver
            .submit_change_request_review(
                &roots,
                &review(
                    7,
                    ChangeRequestReviewEvent::RequestChanges,
                    "please fix",
                    vec![zeron_proto::ChangeRequestReviewComment {
                        path: "src/a.rs".into(),
                        line: 4,
                        left_side: true,
                        body: "gone?".into(),
                    }],
                ),
            )
            .await
            .unwrap();
        assert_eq!(
            resolver
                .submit_change_request_review(
                    &roots,
                    &review(7, ChangeRequestReviewEvent::Comment, " ", Vec::new())
                )
                .await
                .unwrap_err(),
            ActionError::Refused("A review needs a summary or at least one comment.".into())
        );
        assert_eq!(
            resolver
                .submit_change_request_review(
                    &roots,
                    &review(9, ChangeRequestReviewEvent::Approve, "", Vec::new())
                )
                .await
                .unwrap_err(),
            ActionError::Refused(
                "You cannot approve or request changes on your own pull request.".into()
            )
        );
        let reply = |thread: &str| ReplyToChangeRequestThreadRequest {
            host: "github.com".into(),
            repository: "acme/zeron".into(),
            number: 7,
            thread_id: thread.into(),
            body: "done".into(),
        };
        resolver
            .reply_to_change_request_thread(&roots, &reply("T_7"))
            .await
            .unwrap();
        assert_eq!(
            resolver
                .reply_to_change_request_thread(&roots, &reply("T_elsewhere"))
                .await
                .unwrap_err(),
            ActionError::Host(ChangeRequestError::UnsupportedRepository)
        );
        resolver
            .set_change_request_thread_resolved(
                &roots,
                &SetChangeRequestThreadResolvedRequest {
                    host: "github.com".into(),
                    repository: "acme/zeron".into(),
                    number: 7,
                    thread_id: "T_7".into(),
                    resolved: true,
                },
            )
            .await
            .unwrap();
        let requests = runner.requests.lock().unwrap();
        let mutations: Vec<String> = requests
            .iter()
            .filter_map(|request| request.stdin.as_deref())
            .map(|stdin| String::from_utf8_lossy(stdin).into_owned())
            .filter(|stdin| stdin.contains("mutation"))
            .collect();
        assert_eq!(mutations.len(), 2);
        assert!(mutations[0].contains("addPullRequestReviewThreadReply"));
        assert!(mutations[1].contains("resolveReviewThread"));
        assert!(
            requests
                .iter()
                .all(|request| !request.args.iter().any(|arg| arg.contains("please fix"))),
            "review text never rides argv"
        );
    }

    fn worktree_runner(
        listing: &'static str,
        existing: Option<&'static str>,
    ) -> Arc<ScriptedRunner> {
        ScriptedRunner::new(move |request| {
            if request.program == "gh" {
                if request.args.first().map(String::as_str) == Some("pr")
                    && request.args[1] == "view"
                {
                    let mut json = detail_json();
                    json["headRefName"] = serde_json::Value::String("feature/x".into());
                    json["isCrossRepository"] = serde_json::Value::Bool(false);
                    return command_success(serde_json::to_vec(&json).unwrap());
                }
                return command_success("");
            }
            let args: Vec<&str> = request.args.iter().map(String::as_str).collect();
            match args.as_slice() {
                ["fetch", "--quiet", "origin", "refs/pull/742/head"] => command_success(""),
                ["rev-parse", "FETCH_HEAD"] => command_success("abc123\n"),
                ["worktree", "list", "--porcelain"] => command_success(listing),
                ["rev-parse", "--verify", "--quiet", "refs/heads/feature/x"] => match existing {
                    Some(oid) => command_success(oid),
                    None => command_failure(""),
                },
                ["branch", "--", "feature/x", "abc123"] => command_success(""),
                ["worktree", "add", "--quiet", path, "feature/x"] => {
                    assert!(path.ends_with("/repo/pr-742"), "{path}");
                    command_success("")
                }
                ["rev-parse", "HEAD"] => command_success("abc123\n"),
                ["branch", "--show-current"] => command_success("feature/x\n"),
                _ => git_repo_script(request).expect("unexpected command"),
            }
        })
    }

    fn prepare(checkout: ChangeRequestCheckoutMode) -> PrepareChangeRequestChatRequest {
        PrepareChangeRequestChatRequest {
            host: "github.com".into(),
            repository: "acme/zeron".into(),
            number: 742,
            checkout,
        }
    }

    #[tokio::test]
    async fn worktree_checkout_materialises_the_head_branch_and_refuses_the_main_repo() {
        let roots = [PathBuf::from("/repo")];
        let worktrees =
            std::env::temp_dir().join(format!("zeron-pr-worktrees-{}", std::process::id()));
        let runner = worktree_runner("worktree /repo\nHEAD 1\nbranch refs/heads/main\n\n", None);
        let chat = list_resolver(runner.clone())
            .prepare_change_request_chat(
                &roots,
                &worktrees,
                &prepare(ChangeRequestCheckoutMode::Worktree),
            )
            .await
            .unwrap();
        assert!(chat.worktree && chat.on_head);
        assert_eq!(chat.branch.as_deref(), Some("feature/x"));
        assert!(chat.cwd.ends_with("/repo/pr-742"));
        assert!(
            runner.requests.lock().unwrap().iter().all(|request| request
                .args
                .first()
                .map(String::as_str)
                != Some("checkout")),
            "the main repo is never switched in worktree mode"
        );
        let checked_out = worktree_runner(
            "worktree /repo\nHEAD 1\nbranch refs/heads/feature/x\n\n",
            Some("abc123"),
        );
        let refused = list_resolver(checked_out)
            .prepare_change_request_chat(
                &roots,
                &worktrees,
                &prepare(ChangeRequestCheckoutMode::Worktree),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(refused, ActionError::Refused(message) if message.contains("already checked out in the main repo"))
        );
        let local = worktree_runner("", None);
        let chat = list_resolver(local.clone())
            .prepare_change_request_chat(
                &roots,
                &worktrees,
                &prepare(ChangeRequestCheckoutMode::Local),
            )
            .await
            .unwrap();
        assert_eq!(
            (chat.cwd.as_str(), chat.branch.as_deref(), chat.worktree),
            ("/repo", Some("feature/x"), false)
        );
        let gh: Vec<Vec<String>> = local
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.program == "gh")
            .map(|request| request.args.clone())
            .collect();
        assert_eq!(
            gh,
            [[
                "pr",
                "checkout",
                "742",
                "--repo",
                "github.com/acme/zeron",
                "--force"
            ]
            .map(str::to_owned)
            .to_vec()]
        );
        std::fs::remove_dir_all(&worktrees).ok();
    }

    #[test]
    fn worktree_listing_and_branch_names_are_parsed_defensively() {
        assert_eq!(
            parse_worktrees(
                "worktree /a\nHEAD 1\nbranch refs/heads/x\n\nworktree /b\nHEAD 2\ndetached\n\n"
            ),
            [("/a".to_string(), "refs/heads/x".to_string())]
        );
        assert!(valid_branch_name("feature/x-1.2"));
        for bad in ["-rf", "a..b", "a b", "x.lock", "/x", "a@{1}", ""] {
            assert!(!valid_branch_name(bad), "{bad}");
        }
    }

    #[test]
    fn rate_limit_pauses_double_to_fifteen_minutes_and_hosts_come_from_args() {
        assert_eq!(rate_limit_pause(1), Duration::from_secs(30));
        assert_eq!(rate_limit_pause(3), Duration::from_secs(120));
        assert_eq!(rate_limit_pause(12), Duration::from_secs(15 * 60));
        let args = |list: &[&str]| list.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            host_of_args(&args(&["api", "--hostname", "GHE.example.com", "user"])),
            "ghe.example.com"
        );
        assert_eq!(
            host_of_args(&args(&["pr", "view", "7", "--repo", "ghe.io/acme/zeron"])),
            "ghe.io"
        );
        assert_eq!(
            host_of_args(&args(&["pr", "view", "7", "--repo", "acme/zeron"])),
            "github.com"
        );
    }

    #[tokio::test]
    async fn a_rate_limited_host_stops_reads_until_the_pause_ends_but_writes_go_through() {
        let runner = ScriptedRunner::new(|request| {
            if request.args.first().map(String::as_str) == Some("pr") {
                return command_success("");
            }
            command_failure("API rate limit exceeded for user")
        });
        let github = GitHubCli::with_runner(runner.clone());
        let read = || {
            github.run_gh(
                Path::new("/repo"),
                ["api", "--hostname", "github.com", "user"]
                    .map(str::to_owned)
                    .to_vec(),
                GITHUB_TIMEOUT,
                GITHUB_OUTPUT_LIMIT,
            )
        };
        assert_eq!(read().await.unwrap_err(), ChangeRequestError::RateLimited);
        assert_eq!(read().await.unwrap_err(), ChangeRequestError::RateLimited);
        assert_eq!(
            runner.requests.lock().unwrap().len(),
            1,
            "the paused read never spawns gh"
        );
        github
            .run_gh_action(
                Path::new("/repo"),
                ["pr", "close", "7", "--repo", "github.com/acme/zeron"]
                    .map(str::to_owned)
                    .to_vec(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(runner.requests.lock().unwrap().len(), 2);
    }

    #[test]
    fn cached_responses_expire_and_clear_on_invalidate() {
        let key = format!("test:{}", std::process::id());
        remember_response(key.clone(), &serde_json::json!({"n": 1}));
        assert_eq!(
            cached_response(&key, Duration::from_secs(30)),
            Some(serde_json::json!({"n": 1}))
        );
        assert_eq!(cached_response(&key, Duration::ZERO), None);
        invalidate_responses();
        assert_eq!(cached_response(&key, Duration::from_secs(30)), None);
    }

    fn search_node(number: u64, author: &str, rollup: Option<&str>) -> serde_json::Value {
        serde_json::json!({
            "number": number,
            "title": format!("Change {number}"),
            "url": format!("https://github.com/acme/zeron/pull/{number}"),
            "author": {"login": author},
            "headRefName": format!("feature/{number}"),
            "baseRefName": "main",
            "state": "OPEN",
            "isDraft": false,
            "mergeable": "MERGEABLE",
            "reviewDecision": "APPROVED",
            "createdAt": "2026-09-01T00:00:00Z",
            "updatedAt": format!("2026-09-0{number}T00:00:00Z"),
            "mergedAt": null,
            "repository": {"nameWithOwner": "acme/zeron"},
            "reviewRequests": {"nodes": [{"requestedReviewer": {"login": "alice"}}, {"requestedReviewer": {}}]},
            "labels": {"nodes": [{"name": "bug", "color": "d73a4a"}]},
            "commits": {"nodes": [{"commit": {"statusCheckRollup": rollup.map(|state| serde_json::json!({"state": state}))}}]}
        })
    }

    #[tokio::test]
    async fn list_change_requests_reads_one_graphql_search_per_host() {
        let runner = ScriptedRunner::new(|request| {
            if let Some(output) = git_repo_script(request) {
                return output;
            }
            assert_eq!(request.args.first().map(String::as_str), Some("api"));
            if request.args.get(1).map(String::as_str) != Some("graphql") {
                return command_success("alice\n");
            }
            let query = arg_after(request, "-f")
                .and_then(|value| value.strip_prefix("q="))
                .expect("search terms");
            assert!(query.starts_with("is:pr is:open"), "{query}");
            assert!(
                query.ends_with("sort:updated-desc repo:acme/zeron"),
                "{query}"
            );
            let document = request
                .args
                .iter()
                .find_map(|arg| arg.strip_prefix("query="))
                .expect("document");
            assert!(document.contains("first: 3"), "{document}");
            let (nodes, has_next) = if query.contains("author:alice") {
                (vec![search_node(1, "alice", Some("SUCCESS"))], false)
            } else if query.contains("review-requested:alice") {
                (vec![search_node(2, "bob", Some("PENDING"))], false)
            } else {
                (
                    vec![
                        search_node(3, "carol", Some("FAILURE")),
                        serde_json::json!({}),
                        search_node(1, "alice", Some("SUCCESS")),
                    ],
                    true,
                )
            };
            command_success(
                serde_json::to_vec(&serde_json::json!({
                    "data": {"search": {"pageInfo": {"hasNextPage": has_next}, "nodes": nodes}}
                }))
                .unwrap(),
            )
        });
        let request = ListChangeRequestsRequest {
            limit: Some(2),
            ..Default::default()
        };
        let listing = list_resolver(runner.clone())
            .list_change_requests(&[PathBuf::from("/repo")], &request)
            .await;

        assert!(listing.errors.is_empty(), "{:?}", listing.errors);
        assert!(listing.truncated);
        let shape: Vec<(u64, Option<ChangeRequestChecksState>, bool)> = listing
            .items
            .iter()
            .map(|item| (item.number, item.checks_state, item.viewer_review_requested))
            .collect();
        assert_eq!(
            shape,
            [
                (3, Some(ChangeRequestChecksState::Failing), true),
                (2, Some(ChangeRequestChecksState::Pending), true),
                (1, Some(ChangeRequestChecksState::Passing), false),
            ]
        );
        assert_eq!(listing.items[0].labels[0].name, "bug");
        assert!(
            runner.requests.lock().unwrap().iter().all(|request| request
                .args
                .first()
                .map(String::as_str)
                != Some("pr"))
        );
    }

    #[tokio::test]
    async fn a_list_past_one_hundred_rows_follows_the_search_cursor() {
        let runner = ScriptedRunner::new(|request| {
            if let Some(output) = git_repo_script(request) {
                return output;
            }
            if request.args.get(1).map(String::as_str) != Some("graphql") {
                return command_success("alice\n");
            }
            let query = arg_after(request, "-f").unwrap_or_default();
            if !query.contains("is:pr is:open sort:updated-desc") {
                return command_success(
                    r#"{"data":{"search":{"pageInfo":{"hasNextPage":false},"nodes":[]}}}"#,
                );
            }
            let document = request
                .args
                .iter()
                .find_map(|arg| arg.strip_prefix("query="))
                .expect("document");
            let after = request
                .args
                .iter()
                .find_map(|arg| arg.strip_prefix("after="));
            let (range, first, page) = match after {
                None => (
                    1..=100,
                    "first: 100,",
                    serde_json::json!({"hasNextPage": true, "endCursor": "c1"}),
                ),
                Some("c1") => (
                    101..=151,
                    "first: 51,",
                    serde_json::json!({"hasNextPage": true, "endCursor": "c2"}),
                ),
                Some(other) => panic!("unexpected cursor {other}"),
            };
            assert!(document.contains(first), "{document}");
            let nodes: Vec<_> = range
                .map(|number| {
                    let mut node = search_node(number, "carol", None);
                    node["updatedAt"] = "2026-09-01T00:00:00Z".into();
                    node
                })
                .collect();
            command_success(
                serde_json::to_vec(&serde_json::json!({
                    "data": {"search": {"pageInfo": page, "nodes": nodes}}
                }))
                .unwrap(),
            )
        });
        let request = ListChangeRequestsRequest {
            limit: Some(150),
            ..Default::default()
        };
        let listing = list_resolver(runner)
            .list_change_requests(&[PathBuf::from("/repo")], &request)
            .await;

        assert!(listing.errors.is_empty(), "{:?}", listing.errors);
        assert_eq!(listing.items.len(), 150);
        assert!(listing.truncated);
    }

    #[tokio::test]
    async fn list_change_request_stats_batches_aliases_and_drops_foreign_refs() {
        let runner = ScriptedRunner::new(|request| {
            if let Some(output) = git_repo_script(request) {
                return output;
            }
            let document = request
                .args
                .iter()
                .find_map(|arg| arg.strip_prefix("query="))
                .expect("document");
            assert!(document.contains(r#"s0: repository(owner: "acme", name: "zeron") { pullRequest(number: 7) { additions deletions } }"#), "{document}");
            assert!(!document.contains("someone"), "{document}");
            command_success(
                r#"{"data":{"s0":{"pullRequest":{"additions":12,"deletions":3}},"s1":{"pullRequest":null}}}"#,
            )
        });
        let reference = |repository: &str, number: u64| ChangeRequestRef {
            host: "github.com".into(),
            repository: repository.into(),
            number,
        };
        let stats = list_resolver(runner)
            .list_change_request_stats(
                &[PathBuf::from("/repo")],
                &ListChangeRequestStatsRequest {
                    refs: vec![
                        reference("acme/zeron", 7),
                        reference("someone/else", 1),
                        reference("acme/zeron", 8),
                    ],
                },
            )
            .await;
        assert_eq!(
            stats.stats,
            [ChangeRequestStat {
                reference: reference("acme/zeron", 7),
                additions: 12,
                deletions: 3,
            }]
        );
    }

    #[test]
    fn filter_qualifiers_follow_github_search_spelling() {
        let filters = ChangeRequestListFilters {
            draft: Some(ChangeRequestDraftFilter::Hide),
            review: Some(ChangeRequestReviewFilter::ChangesRequested),
            checks: Some(ChangeRequestChecksState::Failing),
            author: Some("@me".into()),
            labels: vec![
                vec!["bug".into(), "needs \"design\"".into()],
                vec!["size:S".into()],
            ],
            excluded_labels: vec!["wontfix".into()],
            repository: Some("acme/zeron".into()),
        };
        assert_eq!(
            filter_qualifiers(&filters, "alice"),
            [
                r#"label:"bug","needs design""#,
                r#"label:"size:S""#,
                r#"-label:"wontfix""#,
                r#"author:"alice""#,
                "draft:false",
                "review:changes_requested",
                "status:failure",
            ]
        );
        assert!(filter_qualifiers(&ChangeRequestListFilters::default(), "alice").is_empty());
    }

    #[test]
    fn local_filters_match_the_search_qualifiers() {
        let mut item = to_list_item(
            &RepositoryIdentity {
                root: PathBuf::from("/repo"),
                remote: parse_git_remote("https://github.com/acme/zeron").unwrap(),
            },
            "alice",
            serde_json::from_value(list_row(
                1,
                "Alice",
                "2026-09-01T00:00:00Z",
                serde_json::json!({"labels": [{"name": "Bug"}, {"name": "size:S"}], "reviewDecision": "APPROVED"}),
            ))
            .unwrap(),
        )
        .unwrap();
        item.checks_state = Some(ChangeRequestChecksState::Passing);
        let check = |filters: ChangeRequestListFilters| matches_filters(&item, &filters, "alice");
        assert!(check(ChangeRequestListFilters::default()));
        assert!(check(ChangeRequestListFilters {
            author: Some("me".into()),
            labels: vec![vec!["bug".into(), "other".into()], vec!["size:s".into()]],
            review: Some(ChangeRequestReviewFilter::Approved),
            checks: Some(ChangeRequestChecksState::Passing),
            draft: Some(ChangeRequestDraftFilter::Hide),
            ..Default::default()
        }));
        assert!(!check(ChangeRequestListFilters {
            excluded_labels: vec!["BUG".into()],
            ..Default::default()
        }));
        assert!(!check(ChangeRequestListFilters {
            review: Some(ChangeRequestReviewFilter::None),
            ..Default::default()
        }));
        assert!(!check(ChangeRequestListFilters {
            draft: Some(ChangeRequestDraftFilter::Only),
            ..Default::default()
        }));
    }

    #[tokio::test]
    async fn search_text_is_one_quoted_phrase_and_never_falls_back() {
        let runner = ScriptedRunner::new(|request| {
            if let Some(output) = git_repo_script(request) {
                return output;
            }
            assert_eq!(request.args.first().map(String::as_str), Some("api"));
            if request.args.get(1).map(String::as_str) != Some("graphql") {
                return command_success("alice\n");
            }
            let query = arg_after(request, "-f")
                .and_then(|value| value.strip_prefix("q="))
                .expect("search terms");
            assert!(
                query.contains(r#""fix \"quoted\" is:merged" label:"bug" sort:updated-desc"#),
                "{query}"
            );
            command_success(r#"{"data":{"search":{"pageInfo":{"hasNextPage":false},"nodes":[]}}}"#)
        });
        let request = ListChangeRequestsRequest {
            query: Some(r#" fix "quoted" is:merged "#.into()),
            filters: ChangeRequestListFilters {
                labels: vec![vec!["bug".into()]],
                ..Default::default()
            },
            ..Default::default()
        };
        let listing = list_resolver(runner.clone())
            .list_change_requests(&[PathBuf::from("/repo")], &request)
            .await;
        assert!(listing.items.is_empty());
        assert!(listing.errors.is_empty());
        assert!(
            runner.requests.lock().unwrap().iter().all(|request| request
                .args
                .first()
                .map(String::as_str)
                != Some("pr"))
        );
    }

    #[tokio::test]
    async fn a_project_filter_searches_only_that_repository_and_lists_every_project() {
        let runner = ScriptedRunner::new(|request| {
            if request.program == "git" {
                let root = request.cwd.to_string_lossy().into_owned();
                return match request.args.first().map(String::as_str) {
                    Some("rev-parse") => command_success(format!("{root}\n")),
                    _ if root == "/other" => {
                        command_success("origin\thttps://github.com/Acme/Other.git (fetch)\n")
                    }
                    _ => command_success("origin\thttps://github.com/acme/zeron.git (fetch)\n"),
                };
            }
            if request.args.get(1).map(String::as_str) != Some("graphql") {
                return command_success("alice\n");
            }
            let query = arg_after(request, "-f")
                .and_then(|value| value.strip_prefix("q="))
                .expect("search terms");
            assert!(
                query.ends_with("sort:updated-desc repo:Acme/Other"),
                "{query}"
            );
            assert!(!query.contains("acme/zeron"), "{query}");
            let mut node = search_node(7, "bob", None);
            node["repository"] = serde_json::json!({"nameWithOwner": "Acme/Other"});
            command_success(
                serde_json::to_vec(&serde_json::json!({
                    "data": {"search": {"pageInfo": {"hasNextPage": false}, "nodes": [node]}}
                }))
                .unwrap(),
            )
        });
        let roots = [PathBuf::from("/repo"), PathBuf::from("/other")];
        let request = |repository: &str| ListChangeRequestsRequest {
            filters: ChangeRequestListFilters {
                repository: Some(repository.into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let resolver = list_resolver(runner.clone());
        let listing = resolver
            .list_change_requests(&roots, &request("acme/other"))
            .await;
        assert!(listing.errors.is_empty(), "{:?}", listing.errors);
        assert_eq!(listing.repositories, ["Acme/Other", "acme/zeron"]);
        assert_eq!(
            listing
                .projects
                .iter()
                .map(|project| (project.path.as_str(), project.repository.as_str()))
                .collect::<Vec<_>>(),
            [("/repo", "acme/zeron"), ("/other", "Acme/Other")]
        );
        assert_eq!(
            listing
                .items
                .iter()
                .map(|item| (item.repository.as_str(), item.number))
                .collect::<Vec<_>>(),
            [("Acme/Other", 7)]
        );
        let searched = |runner: &ScriptedRunner| {
            runner
                .requests
                .lock()
                .unwrap()
                .iter()
                .filter(|request| request.args.get(1).map(String::as_str) == Some("graphql"))
                .count()
        };
        assert!(searched(&runner) > 0);

        let before = searched(&runner);
        let unknown = resolver
            .list_change_requests(&roots, &request("acme/elsewhere"))
            .await;
        assert!(unknown.items.is_empty() && unknown.errors.is_empty());
        assert_eq!(unknown.repositories, ["Acme/Other", "acme/zeron"]);
        assert_eq!(searched(&runner), before);
    }

    fn create_request(title: &str, base: Option<&str>, draft: bool) -> CreateChangeRequestRequest {
        CreateChangeRequestRequest {
            target: zeron_proto::WorkspaceTarget {
                chat_id: None,
                space_id: Some("space-1".into()),
                checkout_path: None,
            },
            host: "github.com".into(),
            repository: "acme/zeron".into(),
            head: "feature/x".into(),
            base: base.map(str::to_owned),
            title: title.into(),
            body: "--body $(touch /tmp/zeron-pwned)\nSecond line.".into(),
            draft,
        }
    }

    fn requests_of(runner: &ScriptedRunner) -> Vec<ProcessRequest> {
        runner.requests.lock().unwrap().clone()
    }

    #[tokio::test]
    async fn creating_a_pull_request_runs_gh_pr_create_with_exact_arguments_and_the_body_on_stdin()
    {
        let runner = ScriptedRunner::new(|request| {
            if let Some(output) = git_repo_script(request) {
                return output;
            }
            command_success("Creating draft pull request\nhttps://github.com/acme/zeron/pull/42\n")
        });
        let resolver = list_resolver(runner.clone());
        let created = resolver
            .create_change_request(
                Path::new("/repo"),
                &create_request("  -t Fix the panel  ", Some("main"), true),
            )
            .await
            .unwrap();
        assert_eq!(
            created,
            CreatedChangeRequest {
                host: "github.com".into(),
                repository: "acme/zeron".into(),
                number: 42,
                url: "https://github.com/acme/zeron/pull/42".into(),
            }
        );
        let gh: Vec<ProcessRequest> = requests_of(&runner)
            .into_iter()
            .filter(|request| request.program == "gh")
            .collect();
        assert_eq!(gh.len(), 1);
        assert_eq!(
            gh[0].args,
            [
                "pr",
                "create",
                "--repo",
                "github.com/acme/zeron",
                "--head=feature/x",
                "--base=main",
                "--title=-t Fix the panel",
                "--body-file",
                "-",
                "--draft",
            ]
        );
        assert_eq!(
            gh[0].stdin.as_deref(),
            Some("--body $(touch /tmp/zeron-pwned)\nSecond line.".as_bytes())
        );
        assert_eq!(gh[0].env, [("GH_PROMPT_DISABLED".into(), "1".into())]);
        assert_eq!(gh[0].cwd, Path::new("/repo"));
        let json = serde_json::to_value(&created).unwrap();
        assert_eq!(json["number"], 42);
        assert_eq!(json["url"], "https://github.com/acme/zeron/pull/42");

        let runner = ScriptedRunner::new(|request| {
            if let Some(output) = git_repo_script(request) {
                return output;
            }
            command_success("https://github.com/acme/zeron/pull/43\n")
        });
        list_resolver(runner.clone())
            .create_change_request(Path::new("/repo"), &create_request("Ready", None, false))
            .await
            .unwrap();
        let gh: Vec<ProcessRequest> = requests_of(&runner)
            .into_iter()
            .filter(|request| request.program == "gh")
            .collect();
        assert_eq!(
            gh[0].args,
            [
                "pr",
                "create",
                "--repo",
                "github.com/acme/zeron",
                "--head=feature/x",
                "--title=Ready",
                "--body-file",
                "-",
            ],
            "no base means the repository default; no draft flag when draft is off"
        );
        assert!(
            gh.iter()
                .flat_map(|request| &request.args)
                .all(|arg| !arg.contains("force"))
        );
    }

    #[tokio::test]
    async fn creating_a_pull_request_refuses_bad_input_and_foreign_repositories_before_gh() {
        let runner = ScriptedRunner::new(|request| {
            if let Some(output) = git_repo_script(request) {
                return output;
            }
            panic!("gh must not run: {:?}", request.args);
        });
        let resolver = list_resolver(runner.clone());
        let mut cases = vec![
            (create_request("   ", None, true), "title"),
            (create_request(&"x".repeat(300), None, true), "too long"),
        ];
        let mut dash = create_request("T", None, true);
        dash.head = "--force".into();
        cases.push((dash, "branch name"));
        let mut spaced = create_request("T", Some("main branch"), true);
        spaced.head = "feature/x".into();
        cases.push((spaced, "branch name"));
        for (request, expected) in cases {
            match resolver
                .create_change_request(Path::new("/repo"), &request)
                .await
            {
                Err(ActionError::Refused(message)) => {
                    assert!(message.contains(expected), "{message}")
                }
                other => panic!("{expected}: {other:?}"),
            }
        }
        let mut foreign = create_request("T", None, true);
        foreign.repository = "someone/else".into();
        assert_eq!(
            resolver
                .create_change_request(Path::new("/repo"), &foreign)
                .await,
            Err(ActionError::Host(ChangeRequestError::UnsupportedRepository))
        );
    }

    #[tokio::test]
    async fn a_refused_create_reads_as_the_hosts_words() {
        let runner = ScriptedRunner::new(|request| {
            if let Some(output) = git_repo_script(request) {
                return output;
            }
            command_failure(
                "a pull request for branch \"feature/x\" into branch \"main\" already exists:\nhttps://github.com/acme/zeron/pull/7\n",
            )
        });
        match list_resolver(runner)
            .create_change_request(Path::new("/repo"), &create_request("T", None, true))
            .await
        {
            Err(ActionError::Refused(message)) => {
                assert!(message.contains("already exists"), "{message}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_real_checkout_creates_through_a_fake_gh_executable() {
        let temp = tempfile::tempdir().expect("fixture tempdir");
        let checkout = temp.path().join("checkout");
        std::fs::create_dir_all(&checkout).unwrap();
        run_git(&checkout, &["init", "-q", "-b", "feature/x"]);
        run_git(
            &checkout,
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/acme/zeron.git",
            ],
        );
        let log = temp.path().join("argv.txt");
        let body = temp.path().join("body.txt");
        let fake_gh = temp.path().join("gh");
        std::fs::write(
            &fake_gh,
            format!(
                "#!/bin/sh\nif [ \"$GH_PROMPT_DISABLED\" != \"1\" ]; then exit 2; fi\nprintf '%s\\n' \"$@\" > '{}'\ncat > '{}'\necho https://github.com/acme/zeron/pull/77\n",
                log.display(),
                body.display()
            ),
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&fake_gh).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&fake_gh, permissions).unwrap();
        let runner: Arc<dyn ProcessRunner> = Arc::new(ExecutableGhRunner {
            executable: fake_gh,
        });
        let resolver = ChangeRequestResolver {
            inspector: GitCheckoutInspector::new(runner.clone()),
            github: GitHubCli::with_runner(runner),
        };
        let created = resolver
            .create_change_request(&checkout, &create_request("Fix the panel", None, true))
            .await
            .unwrap();
        assert_eq!(created.number, 77);
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            "pr\ncreate\n--repo\ngithub.com/acme/zeron\n--head=feature/x\n--title=Fix the panel\n--body-file\n-\n--draft\n"
        );
        assert_eq!(
            std::fs::read_to_string(&body).unwrap(),
            "--body $(touch /tmp/zeron-pwned)\nSecond line."
        );
    }
}
