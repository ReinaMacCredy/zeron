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
    ChangeRequestActor, ChangeRequestCheck, ChangeRequestCheckStatus, ChangeRequestChecksState,
    ChangeRequestComment, ChangeRequestCommentKind, ChangeRequestDetail, ChangeRequestFailure,
    ChangeRequestInvolvement, ChangeRequestLabel, ChangeRequestListError, ChangeRequestListItem,
    ChangeRequestListState, ChangeRequestListing, ChangeRequestMergeability, ChangeRequestRef,
    ChangeRequestReviewDecision, ChangeRequestReviewer, ChangeRequestStat, ChangeRequestState,
    ChangeRequestStats, ChangeRequestSummary, ChangeRequestVerdict, GetChangeRequestRequest,
    ListChangeRequestStatsRequest, ListChangeRequestsRequest,
};

const GIT_TIMEOUT: Duration = Duration::from_secs(10);
const GITHUB_TIMEOUT: Duration = Duration::from_secs(20);
const GITHUB_LIST_TIMEOUT: Duration = Duration::from_secs(30);
const GIT_OUTPUT_LIMIT: usize = 64 * 1024;
const GITHUB_OUTPUT_LIMIT: usize = 1024 * 1024;
const GITHUB_LIST_OUTPUT_LIMIT: usize = 8 * 1024 * 1024;
const GITHUB_RESULT_LIMIT: &str = "20";
const GITHUB_JSON_FIELDS: &str = "number,title,url,state,baseRefName,headRefName,updatedAt,isCrossRepository,headRepositoryOwner";
const GITHUB_DETAIL_FIELDS: &str = "number,title,url,author,headRefName,baseRefName,state,isDraft,mergeable,reviewDecision,createdAt,updatedAt,mergedAt,reviewRequests,labels,additions,deletions,statusCheckRollup,body,changedFiles,closedAt,isCrossRepository,comments,reviews,latestReviews,commits";
const GITHUB_LIST_FIELDS: &str = "number,title,url,author,headRefName,baseRefName,state,isDraft,mergeable,reviewDecision,createdAt,updatedAt,mergedAt,reviewRequests,labels";
const GITHUB_SEARCH_FIELDS: &str = "number title url author { login } headRefName baseRefName state isDraft mergeable reviewDecision createdAt updatedAt mergedAt repository { nameWithOwner } reviewRequests(first: 20) { nodes { requestedReviewer { ... on User { login } } } } labels(first: 20) { nodes { name color } } commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }";
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
        let identities = futures::future::join_all(
            roots
                .iter()
                .map(|root| self.inspector.repository_identity(root)),
        )
        .await;
        let identity = identities
            .into_iter()
            .filter_map(Result::ok)
            .find(|identity| {
                identity.remote.host.eq_ignore_ascii_case(&request.host)
                    && identity
                        .name_with_owner()
                        .eq_ignore_ascii_case(&request.repository)
            })
            .ok_or(ChangeRequestError::UnsupportedRepository)?;
        let (viewer, view) = futures::try_join!(
            self.github
                .viewer_login(&identity.remote.host, &identity.root),
            self.github.view_pull_request(&identity, request.number),
        )?;
        to_detail(&identity, &viewer, view).ok_or(ChangeRequestError::Decode)
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

/// GitHub provider backed by the host's existing `gh` installation and auth.
#[derive(Clone)]
pub struct GitHubCli {
    runner: Arc<dyn ProcessRunner>,
}

impl GitHubCli {
    pub fn new() -> Self {
        Self::with_runner(Arc::new(SystemProcessRunner))
    }

    fn with_runner(runner: Arc<dyn ProcessRunner>) -> Self {
        Self { runner }
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
        let request = ProcessRequest {
            program: "gh".into(),
            args,
            cwd: cwd.to_owned(),
            env: vec![("GH_PROMPT_DISABLED".into(), "1".into())],
            timeout,
            output_limit,
        };
        let output = self.runner.run(request).await.map_err(classify_run_error)?;
        if !output.success {
            return Err(classify_github_failure(&output.stderr));
        }
        if output.stdout_truncated {
            return Err(ChangeRequestError::Decode);
        }
        Ok(output.stdout)
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
            if let Some(mut item) = to_list_item(identity, viewer, row) {
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

    async fn search(
        &self,
        root: &Path,
        host: &str,
        query: String,
        rows: u32,
    ) -> Result<(Vec<GhSearchPullRequest>, usize, bool), ChangeRequestError> {
        let document = format!(
            "query($q: String!) {{ search(query: $q, type: ISSUE, first: {rows}) {{ pageInfo {{ hasNextPage }} nodes {{ ... on PullRequest {{ {GITHUB_SEARCH_FIELDS} }} }} }} }}"
        );
        let stdout = self
            .run_gh(
                root,
                vec![
                    "api".into(),
                    "graphql".into(),
                    "--hostname".into(),
                    host.into(),
                    "-f".into(),
                    format!("q={query}"),
                    "-f".into(),
                    format!("query={document}"),
                ],
                GITHUB_LIST_TIMEOUT,
                GITHUB_LIST_OUTPUT_LIMIT,
            )
            .await?;
        let response: GhSearchResponse =
            serde_json::from_slice(&stdout).map_err(|_| ChangeRequestError::Decode)?;
        let search = response
            .data
            .ok_or(ChangeRequestError::CommandFailed)?
            .search;
        let raw_count = search.nodes.len();
        let rows = search
            .nodes
            .into_iter()
            .filter_map(|node| serde_json::from_value(node).ok())
            .collect();
        Ok((rows, raw_count, search.page_info.has_next_page))
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
        let query = |extra: &str| {
            [
                format!("is:pr {state}"),
                extra.to_owned(),
                "sort:updated-desc".into(),
                repos.clone(),
            ]
            .into_iter()
            .filter(|part| !part.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" ")
        };
        let rows = (limit + 1).min(100);
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
        if raw_count == 0 && request.involvement == ChangeRequestInvolvement::All {
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPageInfo {
    #[serde(default)]
    has_next_page: bool,
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

fn primary_fetch_remote(output: &str) -> Option<&str> {
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
        .map(|(_, url)| *url)
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
    committed_date: Option<DateTime<Utc>>,
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
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command
            .spawn()
            .map_err(|error| ProcessRunError::Spawn(error.kind()))?;
        let stdout = child.stdout.take().ok_or(ProcessRunError::Io)?;
        let stderr = child.stderr.take().ok_or(ProcessRunError::Io)?;
        let completed = tokio::time::timeout(request.timeout, async {
            tokio::try_join!(
                child.wait(),
                read_capped(stdout, request.output_limit),
                read_capped(stderr, request.output_limit),
            )
        })
        .await;

        let (status, (stdout, stdout_truncated), (stderr, _stderr_truncated)) = match completed {
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
                {"committedDate": "2026-10-01T09:00:00Z"},
                {"committedDate": "2026-10-02T09:00:00Z"}
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
}
