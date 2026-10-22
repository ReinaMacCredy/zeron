use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use zeron_proto::change_request_assessment::ChangeRequestCi;
use zeron_proto::git_tab::WORKING_TREE_CHANGED;
use zeron_proto::{
    ChangeRequestActor, ChangeRequestCheck, ChangeRequestComment, ChangeRequestCommentReference,
    ChangeRequestControls, ChangeRequestDetail, ChangeRequestFile, ChangeRequestLabelCandidate,
    ChangeRequestListItem, ChangeRequestMergeMethod, ChangeRequestMergeability, ChangeRequestPage,
    ChangeRequestReviewDecision, ChangeRequestReviewThread, ChangeRequestReviewerCandidate,
    ChangeRequestState, CheckoutDiff, CommitSummary, DiffFileSummary, GitFileState,
    GitHistoryCommit, GitHistoryPage, GitHistoryRef, GitHistoryRefKind, HarnessId, LineDelta,
    RunWorkingTreeActionRequest, StashEntry, SyncBranchReply, SyncBranchRequest, SyncDirection,
    WorkingTree, WorkingTreeAction, WorkingTreeActionReply, WorkingTreeFile, WorkingTreeFileDiff,
};
use zeron_rpc::methods as m;

use super::fixtures::MAC;
use crate::error::{ClientError, Result};

pub(crate) const REPOSITORY: &str = "zeron-sh/zeron";
pub(crate) const CHECKOUT: &str = "/Users/dev/zeron";
const VIEWER: &str = "reina";
const PAGE_SIZE: usize = 6;

struct DemoPr {
    item: ChangeRequestListItem,
    detail: ChangeRequestDetail,
    patch: String,
    labels: Vec<ChangeRequestLabelCandidate>,
    reviewers: Vec<ChangeRequestReviewerCandidate>,
}

pub(crate) struct GitDemo {
    prs: Vec<DemoPr>,
    tree: WorkingTree,
    history: Vec<GitHistoryCommit>,
    remote: Vec<GitHistoryCommit>,
    revision: u64,
    seq: u64,
    now: DateTime<Utc>,
}

fn reply(value: impl Serialize) -> Result<Value> {
    serde_json::to_value(value).map_err(|e| ClientError::Internal(e.to_string()))
}

fn param<T: serde::de::DeserializeOwned>(params: &Value) -> Result<T> {
    serde_json::from_value(params.clone()).map_err(|e| ClientError::InvalidArgument(e.to_string()))
}

fn text<'a>(params: &'a Value, key: &str) -> &'a str {
    params[key].as_str().unwrap_or_default()
}

fn sha(seed: u64) -> String {
    let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ 0x5bd1_e995;
    (0..5)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            format!("{:08x}", state as u32)
        })
        .collect()
}

fn actor(login: &str) -> ChangeRequestActor {
    ChangeRequestActor {
        login: login.into(),
    }
}

fn check(name: &str, conclusion: &str, status: &str) -> ChangeRequestCheck {
    ChangeRequestCheck {
        name: name.into(),
        status: status.into(),
        conclusion: conclusion.into(),
        details_url: format!("https://github.com/{REPOSITORY}/actions/runs/1"),
        ..Default::default()
    }
}

fn comment(id: &str, login: &str, body: &str, at: DateTime<Utc>) -> ChangeRequestComment {
    ChangeRequestComment {
        id: id.into(),
        url: format!("https://github.com/{REPOSITORY}/pull/90#{id}"),
        viewer_did_author: login == VIEWER,
        author: actor(login),
        body: body.into(),
        created_at: at.to_rfc3339(),
        submitted_at: at.to_rfc3339(),
        ..Default::default()
    }
}

fn file_patch(path: &str, start: u32, removed: &[&str], added: &[&str]) -> String {
    let mut out = format!(
        "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -{start},{} +{start},{} @@\n",
        removed.len() + 2,
        added.len() + 2
    );
    out.push_str(" \n");
    for line in removed {
        out.push_str(&format!("-{line}\n"));
    }
    for line in added {
        out.push_str(&format!("+{line}\n"));
    }
    out.push_str(" }\n");
    out
}

struct Spec {
    number: u64,
    title: &'static str,
    author: &'static str,
    branch: &'static str,
    draft: bool,
    decision: ChangeRequestReviewDecision,
    mergeability: ChangeRequestMergeability,
    checks: &'static [(&'static str, &'static str, &'static str)],
    review_requested: bool,
    hours_ago: i64,
    files: &'static [(
        &'static str,
        &'static [&'static str],
        &'static [&'static str],
    )],
}

const PASSING: &[(&str, &str, &str)] = &[
    ("build (macos)", "SUCCESS", "COMPLETED"),
    ("test (linux)", "SUCCESS", "COMPLETED"),
];

fn specs() -> Vec<Spec> {
    use ChangeRequestMergeability::{Conflicting, Mergeable};
    use ChangeRequestReviewDecision::{Approved, ChangesRequested, ReviewRequired};
    vec![
        Spec {
            number: 90,
            title: "Stream pull request status on every client",
            author: VIEWER,
            branch: "feat/pr-status",
            draft: false,
            decision: ChangesRequested,
            mergeability: Mergeable,
            checks: &[
                ("build (macos)", "SUCCESS", "COMPLETED"),
                ("test (linux)", "FAILURE", "COMPLETED"),
                ("lint", "SUCCESS", "COMPLETED"),
            ],
            review_requested: false,
            hours_ago: 1,
            files: &[
                (
                    "crates/client/src/status.rs",
                    &["    let status = fetch_status(url)?;"],
                    &[
                        "    let status = cache.get(url).cloned();",
                        "    let status = status.map_or_else(|| fetch_status(url), Ok)?;",
                    ],
                ),
                (
                    "ios/Zeron/Sessions/StatusRow.swift",
                    &["        Text(status.label)"],
                    &["        Label(status.label, systemImage: status.symbol)"],
                ),
            ],
        },
        Spec {
            number: 97,
            title: "Retry relay calls once after a dropped socket",
            author: "mira",
            branch: "fix/relay-retry",
            draft: false,
            decision: ReviewRequired,
            mergeability: Conflicting,
            checks: PASSING,
            review_requested: true,
            hours_ago: 3,
            files: &[(
                "crates/client/src/live/relay.rs",
                &["        return Err(error);"],
                &[
                    "        if attempt == 0 { continue; }",
                    "        return Err(error);",
                ],
            )],
        },
        Spec {
            number: 95,
            title: "Group the board by what needs you",
            author: VIEWER,
            branch: "feat/board-groups",
            draft: false,
            decision: Approved,
            mergeability: Mergeable,
            checks: PASSING,
            review_requested: false,
            hours_ago: 5,
            files: &[(
                "crates/proto/src/pull_request/board.rs",
                &["    Group::Review"],
                &["    if approved { Group::Approved } else { Group::Review }"],
            )],
        },
        Spec {
            number: 99,
            title: "Show tool output inline on iPhone",
            author: "devon",
            branch: "feat/ios-tool-output",
            draft: false,
            decision: ReviewRequired,
            mergeability: Mergeable,
            checks: &[("build (ios)", "", "IN_PROGRESS")],
            review_requested: true,
            hours_ago: 8,
            files: &[(
                "ios/Zeron/Transcript/ToolRow.swift",
                &["        EmptyView()"],
                &["        ToolOutputView(output: tool.output)"],
            )],
        },
        Spec {
            number: 101,
            title: "Draft: paged history for large repositories",
            author: VIEWER,
            branch: "wip/history-paging",
            draft: true,
            decision: ChangeRequestReviewDecision::Unknown,
            mergeability: Mergeable,
            checks: &[],
            review_requested: false,
            hours_ago: 20,
            files: &[(
                "crates/engine/src/history.rs",
                &["    let limit = 500;"],
                &["    let limit = request.limit.min(200);"],
            )],
        },
        Spec {
            number: 93,
            title: "Cache the PR page per repository",
            author: "mira",
            branch: "feat/pr-cache",
            draft: false,
            decision: Approved,
            mergeability: Mergeable,
            checks: PASSING,
            review_requested: false,
            hours_ago: 26,
            files: &[(
                "crates/engine/src/change_requests/cache.rs",
                &["    None"],
                &["    self.pages.get(repository).cloned()"],
            )],
        },
        Spec {
            number: 92,
            title: "Use the device name in hand-off prompts",
            author: "devon",
            branch: "fix/handoff-device",
            draft: false,
            decision: ReviewRequired,
            mergeability: Mergeable,
            checks: PASSING,
            review_requested: false,
            hours_ago: 50,
            files: &[(
                "crates/proto/src/pull_request/handoff.rs",
                &["    \"this Mac\""],
                &["    device_name"],
            )],
        },
        Spec {
            number: 89,
            title: "Remove the legacy PR action note",
            author: VIEWER,
            branch: "chore/legacy-note",
            draft: false,
            decision: ReviewRequired,
            mergeability: Mergeable,
            checks: PASSING,
            review_requested: false,
            hours_ago: 80,
            files: &[(
                "crates/proto/src/badges.rs",
                &["pub const LEGACY_PR_ACTION_NOTE: &str = \"\";"],
                &[],
            )],
        },
    ]
}

fn build_pr(spec: &Spec, now: DateTime<Utc>) -> DemoPr {
    let url = format!("https://github.com/{REPOSITORY}/pull/{}", spec.number);
    let checks: Vec<_> = spec
        .checks
        .iter()
        .map(|(name, conclusion, status)| check(name, conclusion, status))
        .collect();
    let ci = ChangeRequestCi::from_checks(&checks);
    let head = sha(spec.number);
    let patch: String = spec
        .files
        .iter()
        .enumerate()
        .map(|(index, (path, removed, added))| {
            file_patch(path, 40 + index as u32 * 12, removed, added)
        })
        .collect();
    let files: Vec<_> = spec
        .files
        .iter()
        .map(|(path, removed, added)| ChangeRequestFile {
            path: (*path).into(),
            additions: added.len() as u64,
            deletions: removed.len() as u64,
        })
        .collect();
    let additions = files.iter().map(|f| f.additions).sum();
    let deletions = files.iter().map(|f| f.deletions).sum();
    let updated = now - Duration::hours(spec.hours_ago);
    let item = ChangeRequestListItem {
        provider: "github".into(),
        author: actor(spec.author),
        repository: REPOSITORY.into(),
        head_ref_oid: head.clone(),
        ci: ci.clone(),
        viewer_did_author: Some(spec.author == VIEWER),
        viewer_review_requested: Some(spec.review_requested),
        number: spec.number,
        title: spec.title.into(),
        url: url.clone(),
        state: ChangeRequestState::Open,
        is_draft: spec.draft,
        review_decision: spec.decision,
        additions,
        deletions,
        mergeability: spec.mergeability,
        created_at: updated - Duration::days(2),
        updated_at: updated,
    };
    let decision = match spec.decision {
        ChangeRequestReviewDecision::Approved => "APPROVED",
        ChangeRequestReviewDecision::ChangesRequested => "CHANGES_REQUESTED",
        ChangeRequestReviewDecision::ReviewRequired => "REVIEW_REQUIRED",
        ChangeRequestReviewDecision::Unknown => "",
    };
    let mergeable = match spec.mergeability {
        ChangeRequestMergeability::Mergeable => "MERGEABLE",
        ChangeRequestMergeability::Conflicting => "CONFLICTING",
        ChangeRequestMergeability::Unknown => "UNKNOWN",
    };
    let (comments, reviews, threads) = if spec.number == 90 {
        conversation(&files, now)
    } else {
        (Vec::new(), Vec::new(), Vec::new())
    };
    let detail = ChangeRequestDetail {
        title: spec.title.into(),
        body: format!(
            "{}.\n\nThe session row and the board read the same cached status, so every client shows one answer.",
            spec.title
        ),
        url,
        number: spec.number,
        author: actor(spec.author),
        base_ref_name: "main".into(),
        base_ref_oid: sha(1),
        head_ref_name: spec.branch.into(),
        head_ref_oid: head,
        ci,
        viewer_did_author: Some(spec.author == VIEWER),
        viewer_review_requested: Some(spec.review_requested),
        state: "OPEN".into(),
        is_draft: spec.draft,
        review_decision: decision.into(),
        mergeable: mergeable.into(),
        additions,
        deletions,
        comments,
        reviews,
        review_threads: Some(threads),
        files,
        status_check_rollup: checks,
        viewer_login: Some(VIEWER.into()),
    };
    let labels = [
        ("bug", "d73a4a", spec.number == 97),
        ("ios", "1d76db", spec.number == 90 || spec.number == 99),
        ("needs-tests", "fbca04", false),
    ]
    .into_iter()
    .map(|(name, color, applied)| ChangeRequestLabelCandidate {
        name: name.into(),
        color: Some(color.into()),
        description: None,
        applied,
    })
    .collect();
    let reviewers = [("mira", false), ("devon", false), ("zeron-sh/core", true)]
        .into_iter()
        .filter(|(login, _)| *login != spec.author)
        .map(|(login, team)| ChangeRequestReviewerCandidate {
            login: login.into(),
            name: None,
            team,
            requested: login == "mira" && spec.number == 90,
        })
        .collect();
    DemoPr {
        item,
        detail,
        patch,
        labels,
        reviewers,
    }
}

fn conversation(
    files: &[ChangeRequestFile],
    now: DateTime<Utc>,
) -> (
    Vec<ChangeRequestComment>,
    Vec<ChangeRequestComment>,
    Vec<ChangeRequestReviewThread>,
) {
    let comments = vec![comment(
        "c1",
        "devon",
        "Does the cache survive a relay reconnect? The board flickered for me on the VPS.",
        now - Duration::minutes(50),
    )];
    let mut review = comment(
        "r1",
        "mira",
        "Close. Two things inline, and the Linux test is red.",
        now - Duration::minutes(40),
    );
    review.state = "CHANGES_REQUESTED".into();
    let thread =
        |id: &str, path: &str, line: u64, body: &str, resolved: bool| ChangeRequestReviewThread {
            id: id.into(),
            is_resolved: resolved,
            is_outdated: false,
            path: path.into(),
            line: Some(line),
            start_line: None,
            original_line: Some(line),
            original_start_line: None,
            diff_side: "RIGHT".into(),
            start_diff_side: None,
            comments: vec![comment(
                &format!("{id}-1"),
                "mira",
                body,
                now - Duration::minutes(39),
            )],
        };
    let threads = vec![
        thread(
            "t1",
            &files[0].path,
            42,
            "This clones every status on each read. Return a reference?",
            false,
        ),
        thread(
            "t2",
            &files[1].path,
            53,
            "Give the symbol an accessibility label too.",
            true,
        ),
    ];
    (comments, vec![review], threads)
}

const HISTORY: &[&str] = &[
    "Stream status on the session row",
    "Merge branch 'fix/relay-timeout'",
    "Cache the PR page per repository",
    "Group the board by what needs you",
    "Add the pull request board",
    "Time out relay calls after 20 seconds",
    "Bump version to 0.2.114",
    "Render math as Unicode text",
    "Keep the sidebar order across restarts",
    "Show live tokens per second",
    "Fold settled turns under Worked for",
    "Add the device panel",
    "Persist stars per pull request",
    "Paint the spinner without layout",
    "Read the working tree off the main thread",
    "Show the commit graph in History",
    "Load older history on scroll",
    "Name hand-off sessions after the PR",
    "Explain capability errors in words",
    "Retry the relay handshake once",
    "Trim the demo fixtures",
    "Add the Usage page",
    "Show cost per session",
    "Open PR links in the app",
    "Add swipe navigation",
    "Wrap long branch names",
    "Measure frame stats behind a flag",
    "First commit",
];

fn history(now: DateTime<Utc>) -> Vec<GitHistoryCommit> {
    let count = HISTORY.len();
    HISTORY
        .iter()
        .enumerate()
        .map(|(index, subject)| {
            let parents = match index {
                1 => vec![sha(1000 + 2), sha(1000 + 5)],
                4 => vec![sha(1000 + 6)],
                i if i + 1 < count => vec![sha(1000 + i as u64 + 1)],
                _ => Vec::new(),
            };
            let refs = match index {
                0 => vec![GitHistoryRef {
                    kind: GitHistoryRefKind::Branch,
                    label: "feat/pr-status".into(),
                }],
                1 => vec![GitHistoryRef {
                    kind: GitHistoryRefKind::Remote,
                    label: "origin/feat/pr-status".into(),
                }],
                6 => vec![
                    GitHistoryRef {
                        kind: GitHistoryRefKind::Branch,
                        label: "main".into(),
                    },
                    GitHistoryRef {
                        kind: GitHistoryRefKind::Tag,
                        label: "v0.2.114".into(),
                    },
                ],
                _ => Vec::new(),
            };
            let (name, email) = if index % 3 == 1 {
                ("Mira Chen", "mira@example.com")
            } else {
                ("Reina", "reina@example.com")
            };
            GitHistoryCommit {
                sha: sha(1000 + index as u64),
                parent_shas: parents,
                subject: (*subject).into(),
                author_name: name.into(),
                author_email: email.into(),
                authored_at: (now - Duration::hours(3 * index as i64 + 1)).to_rfc3339(),
                refs,
            }
        })
        .collect()
}

fn tree_file(
    path: &str,
    index: GitFileState,
    worktree: GitFileState,
    staged: (u32, u32),
    unstaged: (u32, u32),
) -> WorkingTreeFile {
    let delta = |(additions, deletions): (u32, u32)| {
        (additions + deletions > 0).then_some(LineDelta {
            additions,
            deletions,
            binary: false,
        })
    };
    WorkingTreeFile {
        path: path.into(),
        old_path: None,
        index,
        worktree,
        staged: delta(staged),
        unstaged: delta(unstaged),
    }
}

impl GitDemo {
    pub(crate) fn new() -> Self {
        use GitFileState::{Added, Modified, Unchanged, Untracked};
        let now = Utc::now();
        let history = history(now);
        let remote = ["Bump gpui", "Fix the iPad split view"]
            .iter()
            .enumerate()
            .map(|(index, subject)| GitHistoryCommit {
                sha: sha(2000 + index as u64),
                parent_shas: Vec::new(),
                subject: (*subject).into(),
                author_name: "Devon Park".into(),
                author_email: "devon@example.com".into(),
                authored_at: (now - Duration::minutes(20 + index as i64)).to_rfc3339(),
                refs: Vec::new(),
            })
            .collect();
        let tree = WorkingTree {
            checkout_id: "co-space-zeron".into(),
            root: CHECKOUT.into(),
            branch: Some("feat/pr-status".into()),
            head_sha: Some(history[0].sha.clone()),
            upstream: Some("origin/feat/pr-status".into()),
            ahead: 1,
            behind: 2,
            push_remote: Some("origin".into()),
            default_branch: Some("main".into()),
            revision: "rev-1".into(),
            truncated: false,
            agent_active: false,
            cherry_pick_head: None,
            files: vec![
                tree_file(
                    "crates/client/src/lib.rs",
                    Modified,
                    Unchanged,
                    (1, 0),
                    (0, 0),
                ),
                tree_file(
                    "crates/client/src/status.rs",
                    Unchanged,
                    Modified,
                    (0, 0),
                    (12, 3),
                ),
                tree_file(
                    "ios/Zeron/Sessions/StatusRow.swift",
                    Modified,
                    Modified,
                    (4, 1),
                    (2, 2),
                ),
                tree_file("docs/status.md", Unchanged, Untracked, (0, 0), (18, 0)),
                tree_file(
                    "crates/client/src/cache.rs",
                    Added,
                    Unchanged,
                    (31, 0),
                    (0, 0),
                ),
            ],
            stashes: vec![StashEntry {
                oid: sha(3000),
                message: "On feat/pr-status: try a shorter poll".into(),
                created_at: Some(now - Duration::days(1)),
            }],
        };
        Self {
            prs: specs().iter().map(|spec| build_pr(spec, now)).collect(),
            tree,
            history,
            remote,
            revision: 1,
            seq: 0,
            now,
        }
    }

    pub(crate) fn handle(&mut self, method: &str, params: &Value) -> Option<Result<Value>> {
        Some(match method {
            m::LIST_CHANGE_REQUEST_PAGE => self.page(params),
            m::GET_CHANGE_REQUEST_REPOSITORY => {
                let known = text(params, "cwd").starts_with(CHECKOUT);
                let asked = params["repository"].as_str();
                reply(
                    known
                        .then_some(REPOSITORY)
                        .filter(|repo| asked.is_none_or(|a| a == *repo)),
                )
            }
            m::GET_CHANGE_REQUEST => self.pr(params).map(|pr| pr.detail.clone()).and_then(reply),
            m::GET_CHANGE_REQUEST_DIFF => {
                self.pr(params).map(|pr| pr.patch.clone()).and_then(reply)
            }
            m::GET_CHANGE_REQUEST_CONTROLS => self.pr(params).and_then(|pr| {
                reply(ChangeRequestControls {
                    merge_methods: vec![
                        ChangeRequestMergeMethod::Squash,
                        ChangeRequestMergeMethod::Merge,
                        ChangeRequestMergeMethod::Rebase,
                    ],
                    can_write: true,
                    can_triage: true,
                    did_author: pr.item.author.login == VIEWER,
                    labels: pr.labels.clone(),
                    reviewers: pr.reviewers.clone(),
                })
            }),
            m::LIST_CHANGE_REQUEST_LABEL_CANDIDATES => self
                .pr(params)
                .and_then(|pr| reply(json!({ "labels": pr.labels, "truncated": false }))),
            m::LIST_CHANGE_REQUEST_REVIEWER_CANDIDATES => self
                .pr(params)
                .and_then(|pr| reply(json!({ "reviewers": pr.reviewers, "truncated": false }))),
            m::POST_CHANGE_REQUEST_COMMENT => self.post_comment(params),
            m::REPLY_TO_CHANGE_REQUEST_THREAD => self.reply_to_thread(params),
            m::SET_CHANGE_REQUEST_THREAD_RESOLVED => self.resolve_thread(params),
            m::SUBMIT_CHANGE_REQUEST_REVIEW => self.submit_review(params),
            m::SET_CHANGE_REQUEST_LABEL => self.set_label(params),
            m::SET_CHANGE_REQUEST_REVIEWER => self.set_reviewer(params),
            m::MERGE_CHANGE_REQUEST => self.merge(params),
            m::GET_WORKING_TREE => reply(&self.tree),
            m::GET_WORKING_TREE_FILE_DIFF => self.file_diff(params),
            m::RUN_WORKING_TREE_ACTION => param(params).and_then(|r| self.run_action(r)),
            m::SYNC_BRANCH => param(params).and_then(|r| self.sync(r)),
            m::GENERATE_COMMIT_MESSAGE => reply(json!({
                "message": "Read PR status from the shared cache\n\nThe session row and the board now ask one cache, so a reconnect no longer refetches every status.",
                "harness": HarnessId::ClaudeCode,
                "model": "haiku",
            })),
            m::LIST_GIT_HISTORY => self.history_page(params),
            m::GET_CHECKOUT_DIFF => self.commit_diff(params),
            _ => return None,
        })
    }

    fn page(&self, params: &Value) -> Result<Value> {
        let filter = text(params, "filter");
        let matching: Vec<_> = self
            .prs
            .iter()
            .filter(|pr| pr.item.state == ChangeRequestState::Open)
            .filter(|pr| match filter {
                "authored" => pr.item.viewer_did_author == Some(true),
                "reviewing" => pr.item.viewer_review_requested == Some(true),
                _ => true,
            })
            .map(|pr| pr.item.clone())
            .collect();
        let start: usize = text(params, "after").parse().unwrap_or(0);
        let end = (start + PAGE_SIZE).min(matching.len());
        reply(ChangeRequestPage {
            items: matching[start.min(end)..end].to_vec(),
            next_cursor: (end < matching.len()).then(|| end.to_string()),
            total_count: Some(matching.len() as u64),
            fetched_at: Some(self.now),
        })
    }

    fn index_of(&self, params: &Value) -> Result<usize> {
        let url = text(params, "url");
        self.prs
            .iter()
            .position(|pr| pr.item.url == url)
            .ok_or_else(|| ClientError::HostError("Check the link and try again.".into()))
    }

    fn pr(&self, params: &Value) -> Result<&DemoPr> {
        self.index_of(params).map(|index| &self.prs[index])
    }

    fn pr_mut(&mut self, params: &Value) -> Result<&mut DemoPr> {
        let index = self.index_of(params)?;
        Ok(&mut self.prs[index])
    }

    fn next_comment(&mut self, body: &str) -> ChangeRequestComment {
        self.seq += 1;
        comment(&format!("demo-{}", self.seq), VIEWER, body, Utc::now())
    }

    fn post_comment(&mut self, params: &Value) -> Result<Value> {
        let body = text(params, "body").trim().to_owned();
        if body.is_empty() {
            return Err(ClientError::InvalidArgument(
                "Write a comment first.".into(),
            ));
        }
        let new = self.next_comment(&body);
        self.pr_mut(params)?.detail.comments.push(new.clone());
        reply(new)
    }

    fn thread_mut<'a>(
        pr: &'a mut DemoPr,
        params: &Value,
    ) -> Result<&'a mut ChangeRequestReviewThread> {
        let id = text(params, "threadId");
        pr.detail
            .review_threads
            .get_or_insert_default()
            .iter_mut()
            .find(|thread| thread.id == id)
            .ok_or_else(|| ClientError::NotFound(id.to_owned()))
    }

    fn reply_to_thread(&mut self, params: &Value) -> Result<Value> {
        let mut new = self.next_comment(text(params, "body"));
        let pr = self.pr_mut(params)?;
        let thread = Self::thread_mut(pr, params)?;
        new.reply_to = thread
            .comments
            .first()
            .map(|first| ChangeRequestCommentReference {
                id: first.id.clone(),
            });
        thread.comments.push(new);
        Ok(Value::Null)
    }

    fn resolve_thread(&mut self, params: &Value) -> Result<Value> {
        let resolved = params["resolved"].as_bool().unwrap_or(true);
        Self::thread_mut(self.pr_mut(params)?, params)?.is_resolved = resolved;
        Ok(Value::Null)
    }

    fn submit_review(&mut self, params: &Value) -> Result<Value> {
        let event = text(params, "event").to_owned();
        let mut review = self.next_comment(text(params, "body"));
        let (state, decision) = match event.as_str() {
            "approve" => ("APPROVED", Some(ChangeRequestReviewDecision::Approved)),
            "requestChanges" => (
                "CHANGES_REQUESTED",
                Some(ChangeRequestReviewDecision::ChangesRequested),
            ),
            _ => ("COMMENTED", None),
        };
        review.state = state.into();
        let pr = self.pr_mut(params)?;
        if pr.item.author.login == VIEWER && decision.is_some() {
            return Err(ClientError::HostError(
                "You can't approve or request changes on your own pull request.".into(),
            ));
        }
        pr.detail.reviews.push(review);
        if let Some(decision) = decision {
            pr.item.review_decision = decision;
            pr.detail.review_decision = state.into();
        }
        Ok(Value::Null)
    }

    fn set_label(&mut self, params: &Value) -> Result<Value> {
        let (name, add) = (
            text(params, "name"),
            params["add"].as_bool().unwrap_or(true),
        );
        let pr = self.pr_mut(params)?;
        match pr.labels.iter_mut().find(|label| label.name == name) {
            Some(label) => label.applied = add,
            None if add => pr.labels.push(ChangeRequestLabelCandidate {
                name: name.into(),
                applied: true,
                ..Default::default()
            }),
            None => {}
        }
        Ok(Value::Null)
    }

    fn set_reviewer(&mut self, params: &Value) -> Result<Value> {
        let (login, add) = (
            text(params, "login"),
            params["add"].as_bool().unwrap_or(true),
        );
        let pr = self.pr_mut(params)?;
        if let Some(reviewer) = pr.reviewers.iter_mut().find(|r| r.login == login) {
            reviewer.requested = add;
        }
        Ok(Value::Null)
    }

    fn merge(&mut self, params: &Value) -> Result<Value> {
        let pr = self.pr_mut(params)?;
        if pr.item.mergeability == ChangeRequestMergeability::Conflicting {
            return Err(ClientError::HostError(
                "This branch has conflicts that must be resolved first.".into(),
            ));
        }
        pr.item.state = ChangeRequestState::Merged;
        pr.detail.state = "MERGED".into();
        Ok(Value::Null)
    }

    fn file_diff(&self, params: &Value) -> Result<Value> {
        let path = text(params, "path");
        let staged = params["staged"].as_bool().unwrap_or(false);
        let file = self
            .tree
            .files
            .iter()
            .find(|file| file.path == path)
            .ok_or_else(|| ClientError::NotFound(path.to_owned()))?;
        let delta = if staged { &file.staged } else { &file.unstaged };
        let delta = delta.unwrap_or_default();
        let removed: Vec<String> = (0..delta.deletions)
            .map(|i| format!("    let before_{i} = old_value({i});"))
            .collect();
        let added: Vec<String> = (0..delta.additions)
            .map(|i| format!("    let after_{i} = new_value({i});"))
            .collect();
        let removed: Vec<&str> = removed.iter().map(String::as_str).collect();
        let added: Vec<&str> = added.iter().map(String::as_str).collect();
        reply(WorkingTreeFileDiff {
            checkout_id: self.tree.checkout_id.clone(),
            path: path.into(),
            staged,
            patch: file_patch(path, 18, &removed, &added),
            binary: false,
            truncated: false,
        })
    }

    fn bump(&mut self) {
        self.revision += 1;
        self.tree.revision = format!("rev-{}", self.revision);
    }

    fn run_action(&mut self, request: RunWorkingTreeActionRequest) -> Result<Value> {
        if request.expected_revision != self.tree.revision {
            return Err(ClientError::HostError(WORKING_TREE_CHANGED.into()));
        }
        let mut commit = None;
        match request.action {
            WorkingTreeAction::Stage { paths } => self.stage(|file| paths.contains(&file.path)),
            WorkingTreeAction::StageAll => self.stage(|_| true),
            WorkingTreeAction::Unstage { paths } => self.unstage(|file| paths.contains(&file.path)),
            WorkingTreeAction::UnstageAll => self.unstage(|_| true),
            WorkingTreeAction::Commit {
                message,
                amend,
                all,
            } => commit = Some(self.commit(&message, amend, all)?),
            _ => {
                return Err(ClientError::HostError(
                    "This Git action is not available from the phone.".into(),
                ));
            }
        }
        self.bump();
        reply(WorkingTreeActionReply {
            commit,
            conflicts: Vec::new(),
        })
    }

    fn stage(&mut self, pick: impl Fn(&WorkingTreeFile) -> bool) {
        for file in self.tree.files.iter_mut().filter(|file| pick(file)) {
            if file.worktree == GitFileState::Unchanged {
                continue;
            }
            file.index = match (file.index, file.worktree) {
                (GitFileState::Unchanged, GitFileState::Untracked) => GitFileState::Added,
                (GitFileState::Unchanged, state) => state,
                (index, _) => index,
            };
            let (staged, unstaged) = (
                file.staged.unwrap_or_default(),
                file.unstaged.unwrap_or_default(),
            );
            file.staged = Some(LineDelta {
                additions: staged.additions + unstaged.additions,
                deletions: staged.deletions + unstaged.deletions,
                binary: false,
            });
            file.worktree = GitFileState::Unchanged;
            file.unstaged = None;
        }
    }

    fn unstage(&mut self, pick: impl Fn(&WorkingTreeFile) -> bool) {
        for file in self.tree.files.iter_mut().filter(|file| pick(file)) {
            if !file.is_staged() {
                continue;
            }
            let (staged, unstaged) = (
                file.staged.unwrap_or_default(),
                file.unstaged.unwrap_or_default(),
            );
            file.worktree = match file.index {
                GitFileState::Added => GitFileState::Untracked,
                index => index,
            };
            file.index = GitFileState::Unchanged;
            file.unstaged = Some(LineDelta {
                additions: staged.additions + unstaged.additions,
                deletions: staged.deletions + unstaged.deletions,
                binary: false,
            });
            file.staged = None;
        }
    }

    fn commit(&mut self, message: &str, amend: bool, all: bool) -> Result<CommitSummary> {
        let subject = message.lines().next().unwrap_or_default().trim().to_owned();
        if subject.is_empty() {
            return Err(ClientError::HostError(
                "Write a commit message first.".into(),
            ));
        }
        if all {
            self.stage(|file| file.worktree != GitFileState::Untracked);
        }
        if !amend && !self.tree.files.iter().any(WorkingTreeFile::is_staged) {
            return Err(ClientError::HostError("Stage a change to commit.".into()));
        }
        for file in &mut self.tree.files {
            file.index = GitFileState::Unchanged;
            file.staged = None;
        }
        self.tree
            .files
            .retain(|file| file.worktree != GitFileState::Unchanged);
        self.seq += 1;
        let head = self.history.first().cloned();
        let parents = match (&head, amend) {
            (Some(head), true) => head.parent_shas.clone(),
            (Some(head), false) => vec![head.sha.clone()],
            (None, _) => Vec::new(),
        };
        let refs = head
            .as_ref()
            .map(|head| head.refs.clone())
            .unwrap_or_default();
        let new = GitHistoryCommit {
            sha: sha(5000 + self.seq),
            parent_shas: parents,
            subject: subject.clone(),
            author_name: "Reina".into(),
            author_email: "reina@example.com".into(),
            authored_at: Utc::now().to_rfc3339(),
            refs,
        };
        if let Some(head) = self.history.first_mut() {
            head.refs.retain(|r| r.kind != GitHistoryRefKind::Branch);
        }
        if amend && !self.history.is_empty() {
            self.history[0] = new.clone();
        } else {
            self.history.insert(0, new.clone());
            self.tree.ahead += 1;
        }
        self.tree.head_sha = Some(new.sha.clone());
        Ok(CommitSummary {
            sha: new.sha,
            branch: self.tree.branch.clone(),
            subject,
        })
    }

    fn sync(&mut self, request: SyncBranchRequest) -> Result<Value> {
        if self.tree.head_sha.as_deref() != Some(request.expected_head_sha.as_str()) {
            return Err(ClientError::HostError(WORKING_TREE_CHANGED.into()));
        }
        let commits = match request.direction {
            SyncDirection::Push { .. } => std::mem::take(&mut self.tree.ahead),
            SyncDirection::Pull => {
                let pulled = std::mem::take(&mut self.remote);
                let count = pulled.len() as u32;
                let mut parent = self.tree.head_sha.clone();
                let branch_refs = self
                    .history
                    .first_mut()
                    .map(|head| std::mem::take(&mut head.refs))
                    .unwrap_or_default();
                for mut commit in pulled.into_iter().rev() {
                    commit.parent_shas = parent.into_iter().collect();
                    parent = Some(commit.sha.clone());
                    self.history.insert(0, commit);
                }
                if let Some(head) = self.history.first_mut() {
                    head.refs = branch_refs;
                }
                self.tree.head_sha = parent;
                self.tree.behind = 0;
                count
            }
        };
        self.bump();
        reply(SyncBranchReply {
            head_sha: self.tree.head_sha.clone().unwrap_or_default(),
            upstream: self.tree.upstream.clone(),
            ahead: self.tree.ahead,
            behind: self.tree.behind,
            commits,
        })
    }

    fn history_page(&self, params: &Value) -> Result<Value> {
        let cursor = params["cursor"].as_u64().unwrap_or(0) as usize;
        let limit = params["limit"].as_u64().unwrap_or(50).max(1) as usize;
        let end = (cursor + limit).min(self.history.len());
        reply(GitHistoryPage {
            commits: self.history[cursor.min(end)..end].to_vec(),
            branch_tips: Vec::new(),
            head_sha: self.tree.head_sha.clone(),
            next_cursor: (end < self.history.len()).then_some(end),
            total_count: Some(self.history.len()),
            head_commit_count: Some(self.history.len()),
            comparison: None,
        })
    }

    fn commit_diff(&self, params: &Value) -> Result<Value> {
        let wanted = text(params, "commitSha");
        let commit = self
            .history
            .iter()
            .find(|commit| commit.sha == wanted)
            .ok_or_else(|| ClientError::NotFound(wanted.to_owned()))?;
        let slug: String = commit
            .subject
            .to_lowercase()
            .split_whitespace()
            .take(3)
            .collect::<Vec<_>>()
            .join("_");
        let path = format!("crates/client/src/{slug}.rs");
        let removed = ["    todo!()"];
        let added = [
            "    let value = read()?;".to_owned(),
            format!("    log::debug!(\"{}\");", commit.subject),
            "    Ok(value)".to_owned(),
        ];
        let added: Vec<&str> = added.iter().map(String::as_str).collect();
        reply(CheckoutDiff {
            checkout_id: self.tree.checkout_id.clone(),
            device_id: MAC.into(),
            cwd: CHECKOUT.into(),
            patch: file_patch(&path, 7, &removed, &added),
            files: vec![DiffFileSummary {
                path,
                old_path: None,
                status: "modified".into(),
                additions: added.len() as u32,
                deletions: removed.len() as u32,
                binary: false,
            }],
            additions: added.len() as u32,
            deletions: removed.len() as u32,
            truncated: false,
            checksum: commit.sha.clone(),
            updated_at: Utc::now(),
        })
    }
}
