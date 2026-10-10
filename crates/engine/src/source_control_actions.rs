use super::*;
use serde_json::{Value, json};
use zeron_proto::{
    ChangeRequestControls, ChangeRequestLabelCandidate, ChangeRequestLabelCandidates,
    ChangeRequestMergeMethod, ChangeRequestReviewEvent, ChangeRequestReviewerCandidate,
    ChangeRequestReviewerCandidates,
};

const CONTROLS_QUERY: &str = "query($owner:String!,$name:String!,$number:Int!) { repository(owner:$owner,name:$name) { mergeCommitAllowed squashMergeAllowed rebaseMergeAllowed viewerPermission pullRequest(number:$number) { viewerDidAuthor labels(first:100) { nodes { name color description } } reviewRequests(first:100) { nodes { requestedReviewer { ... on User { login name } ... on Team { slug name } ... on Bot { login } } } } } } }";
const LABEL_CANDIDATES_QUERY: &str = "query($owner:String!,$name:String!,$number:Int!) { repository(owner:$owner,name:$name) { labels(first:100,orderBy:{field:NAME,direction:ASC}) { pageInfo { hasNextPage } nodes { name color description } } pullRequest(number:$number) { labels(first:100) { nodes { name } } } } }";
const REVIEWER_CANDIDATES_QUERY: &str = "query($owner:String!,$name:String!,$number:Int!) { repository(owner:$owner,name:$name) { assignableUsers(first:100) { pageInfo { hasNextPage } nodes { login name } } pullRequest(number:$number) { author { login } reviewRequests(first:100) { nodes { requestedReviewer { ... on User { login name } ... on Team { slug name } ... on Bot { login } } } } } } }";
const THREAD_SCOPE_QUERY: &str = "query($owner:String!,$name:String!,$number:Int!,$id:ID!) { repository(owner:$owner,name:$name) { pullRequest(number:$number) { id } } node(id:$id) { ... on PullRequestReviewThread { pullRequest { id } } } }";
const BODY_LIMIT: usize = 60_000;

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

struct Target {
    url: String,
    owner: String,
    name: String,
    number: u64,
}

impl Target {
    fn parse(url: &str) -> Result<Self, ChangeRequestError> {
        let url = validated_pull_request_url(url)?;
        let parts: Vec<_> = url.split('/').collect();
        Ok(Self {
            owner: parts[3].to_owned(),
            name: parts[4].to_owned(),
            number: parts[6]
                .parse()
                .map_err(|_| ChangeRequestError::UnsupportedRepository)?,
            url,
        })
    }

    fn variables(&self) -> Value {
        json!({"owner": self.owner, "name": self.name, "number": self.number})
    }

    fn endpoint(&self, path: &str) -> String {
        format!("repos/{}/{}/{path}", self.owner, self.name)
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct GhConnection<T> {
    nodes: Vec<T>,
    page_info: GhMore,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct GhMore {
    has_next_page: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct GhLabel {
    name: String,
    color: Option<String>,
    description: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct GhActor {
    login: Option<String>,
    slug: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct GhReviewRequest {
    requested_reviewer: Option<GhActor>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct GhControlsRepository {
    merge_commit_allowed: bool,
    squash_merge_allowed: bool,
    rebase_merge_allowed: bool,
    viewer_permission: String,
    pull_request: Option<GhControlsPull>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct GhControlsPull {
    viewer_did_author: bool,
    labels: GhConnection<GhLabel>,
    review_requests: GhConnection<GhReviewRequest>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct GhLabelCandidatesRepository {
    labels: GhConnection<GhLabel>,
    pull_request: Option<GhLabelledPull>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct GhLabelledPull {
    labels: GhConnection<GhLabel>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct GhReviewerCandidatesRepository {
    assignable_users: GhConnection<GhActor>,
    pull_request: Option<GhRequestedPull>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct GhRequestedPull {
    author: Option<GhActor>,
    review_requests: GhConnection<GhReviewRequest>,
}

fn to_controls(repository: GhControlsRepository) -> ChangeRequestControls {
    let permission = repository.viewer_permission.trim().to_ascii_uppercase();
    let can_write = matches!(permission.as_str(), "ADMIN" | "MAINTAIN" | "WRITE");
    let pull = repository.pull_request.unwrap_or_default();
    ChangeRequestControls {
        merge_methods: [
            (
                repository.merge_commit_allowed,
                ChangeRequestMergeMethod::Merge,
            ),
            (
                repository.squash_merge_allowed,
                ChangeRequestMergeMethod::Squash,
            ),
            (
                repository.rebase_merge_allowed,
                ChangeRequestMergeMethod::Rebase,
            ),
        ]
        .into_iter()
        .filter_map(|(allowed, method)| allowed.then_some(method))
        .collect(),
        can_write,
        can_triage: can_write || permission == "TRIAGE",
        did_author: pull.viewer_did_author,
        labels: pull
            .labels
            .nodes
            .into_iter()
            .filter(|label| !label.name.trim().is_empty())
            .map(|label| ChangeRequestLabelCandidate {
                name: label.name.trim().to_owned(),
                color: label.color,
                description: label.description.filter(|text| !text.trim().is_empty()),
                applied: true,
            })
            .collect(),
        reviewers: requested_reviewers(pull.review_requests.nodes),
    }
}

fn requested_reviewers(requests: Vec<GhReviewRequest>) -> Vec<ChangeRequestReviewerCandidate> {
    requests
        .into_iter()
        .filter_map(|request| {
            let actor = request.requested_reviewer?;
            let (login, team) = match (actor.login, actor.slug) {
                (Some(login), _) => (login, false),
                (None, Some(slug)) => (slug, true),
                (None, None) => return None,
            };
            Some(ChangeRequestReviewerCandidate {
                login,
                name: actor.name.filter(|name| !name.trim().is_empty()),
                team,
                requested: true,
            })
        })
        .collect()
}

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
            applied: true,
            ..Default::default()
        })
        .collect();
    labels.splice(0..0, missing);
    ChangeRequestLabelCandidates {
        labels,
        truncated: repository.labels.page_info.has_next_page,
    }
}

fn to_reviewer_candidates(
    repository: GhReviewerCandidatesRepository,
) -> ChangeRequestReviewerCandidates {
    let pull = repository.pull_request.unwrap_or_default();
    let author = pull.author.and_then(|author| author.login);
    let mut reviewers = requested_reviewers(pull.review_requests.nodes);
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

fn merge_flag(method: ChangeRequestMergeMethod) -> &'static str {
    match method {
        ChangeRequestMergeMethod::Merge => "--merge",
        ChangeRequestMergeMethod::Squash => "--squash",
        ChangeRequestMergeMethod::Rebase => "--rebase",
    }
}

fn valid_login(login: &str) -> bool {
    (1..=100).contains(&login.len())
        && login
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '[' | ']'))
}

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

fn readable_failure(text: &[u8]) -> String {
    let text = String::from_utf8_lossy(text);
    let text = text.trim();
    if text.chars().count() > 320 {
        format!("{}…", text.chars().take(319).collect::<String>())
    } else {
        text.to_owned()
    }
}

fn body_text(body: &str, empty: &str) -> Result<(), ActionError> {
    if body.trim().is_empty() {
        return Err(ActionError::Refused(empty.into()));
    }
    if body.len() > BODY_LIMIT {
        return Err(ActionError::Refused(
            "That text is too long (maximum 60,000 bytes).".into(),
        ));
    }
    Ok(())
}

impl GitHubCli {
    async fn repository_query(&self, query: &str, target: &Target) -> PrResult {
        let mut budget = GITHUB_OUTPUT_LIMIT;
        let value = self
            .review_query(query.to_owned(), target.variables(), &mut budget)
            .await?;
        Ok(value["data"]["repository"].clone())
    }

    pub async fn change_request_controls(
        &self,
        url: &str,
    ) -> Result<ChangeRequestControls, ChangeRequestError> {
        let target = Target::parse(url)?;
        self.controls(&target).await
    }

    async fn controls(&self, target: &Target) -> Result<ChangeRequestControls, ChangeRequestError> {
        let repository = self.repository_query(CONTROLS_QUERY, target).await?;
        if repository.get("pullRequest").is_none() {
            return Err(ChangeRequestError::Decode);
        }
        serde_json::from_value(repository)
            .map(to_controls)
            .map_err(|_| ChangeRequestError::Decode)
    }

    pub async fn label_candidates(
        &self,
        url: &str,
    ) -> Result<ChangeRequestLabelCandidates, ChangeRequestError> {
        let target = Target::parse(url)?;
        let repository = self
            .repository_query(LABEL_CANDIDATES_QUERY, &target)
            .await?;
        serde_json::from_value(repository)
            .map(to_label_candidates)
            .map_err(|_| ChangeRequestError::Decode)
    }

    pub async fn reviewer_candidates(
        &self,
        url: &str,
    ) -> Result<ChangeRequestReviewerCandidates, ChangeRequestError> {
        let target = Target::parse(url)?;
        let repository = self
            .repository_query(REVIEWER_CANDIDATES_QUERY, &target)
            .await?;
        serde_json::from_value(repository)
            .map(to_reviewer_candidates)
            .map_err(|_| ChangeRequestError::Decode)
    }

    /// Explicit user confirmation only. Never retry a write after an ambiguous failure.
    pub async fn merge(
        &self,
        url: &str,
        method: ChangeRequestMergeMethod,
    ) -> Result<(), ActionError> {
        let target = Target::parse(url)?;
        let controls = self.controls(&target).await?;
        if !controls.can_write {
            return Err(ActionError::Refused(
                "You need write access on this repository to merge.".into(),
            ));
        }
        if !controls.merge_methods.contains(&method) {
            return Err(ActionError::Refused(format!(
                "This repository does not allow the {} strategy.",
                merge_flag(method).trim_start_matches("--")
            )));
        }
        self.write(
            vec![
                "pr".into(),
                "merge".into(),
                target.url.clone(),
                merge_flag(method).into(),
            ],
            None,
        )
        .await?;
        self.forget(&target, true).await;
        Ok(())
    }

    pub async fn set_label(&self, url: &str, name: &str, add: bool) -> Result<(), ActionError> {
        let name = name.trim();
        if name.is_empty() || name.len() > 100 {
            return Err(ActionError::Refused("A label needs a name.".into()));
        }
        let target = Target::parse(url)?;
        if !self.controls(&target).await?.can_triage {
            return Err(ActionError::Refused(
                "You need triage access on this repository to change its labels.".into(),
            ));
        }
        let path = target.endpoint(&format!("issues/{}/labels", target.number));
        let (args, input) = if add {
            (
                vec![
                    "api".into(),
                    "--method".into(),
                    "POST".into(),
                    path,
                    "--input".into(),
                    "-".into(),
                ],
                Some(json!({ "labels": [name] })),
            )
        } else {
            (
                vec![
                    "api".into(),
                    "--method".into(),
                    "DELETE".into(),
                    format!("{path}/{}", percent_encode(name)),
                ],
                None,
            )
        };
        self.write(args, input).await?;
        Ok(())
    }

    pub async fn set_reviewer(
        &self,
        url: &str,
        login: &str,
        team: bool,
        add: bool,
    ) -> Result<(), ActionError> {
        if !valid_login(login) {
            return Err(ActionError::Refused(
                "That reviewer name is not valid.".into(),
            ));
        }
        let target = Target::parse(url)?;
        if !self.controls(&target).await?.can_write {
            return Err(ActionError::Refused(
                "Asking someone to review needs write access on this repository.".into(),
            ));
        }
        let body = if team {
            json!({ "team_reviewers": [login] })
        } else {
            json!({ "reviewers": [login] })
        };
        self.write(
            vec![
                "api".into(),
                "--method".into(),
                if add { "POST" } else { "DELETE" }.into(),
                target.endpoint(&format!("pulls/{}/requested_reviewers", target.number)),
                "--input".into(),
                "-".into(),
            ],
            Some(body),
        )
        .await?;
        Ok(())
    }

    pub async fn submit_review(
        &self,
        url: &str,
        event: ChangeRequestReviewEvent,
        body: &str,
    ) -> Result<(), ActionError> {
        if event != ChangeRequestReviewEvent::Approve || body.len() > BODY_LIMIT {
            body_text(body, "A review needs a summary.")?;
        }
        let target = Target::parse(url)?;
        if event != ChangeRequestReviewEvent::Comment && self.controls(&target).await?.did_author {
            return Err(ActionError::Refused(
                "You cannot approve or request changes on your own pull request.".into(),
            ));
        }
        self.write(
            vec![
                "api".into(),
                "--method".into(),
                "POST".into(),
                target.endpoint(&format!("pulls/{}/reviews", target.number)),
                "--input".into(),
                "-".into(),
            ],
            Some(json!({
                "event": match event {
                    ChangeRequestReviewEvent::Comment => "COMMENT",
                    ChangeRequestReviewEvent::Approve => "APPROVE",
                    ChangeRequestReviewEvent::RequestChanges => "REQUEST_CHANGES",
                },
                "body": body,
            })),
        )
        .await?;
        self.forget(&target, true).await;
        Ok(())
    }

    pub async fn reply_to_thread(
        &self,
        url: &str,
        thread: &str,
        body: &str,
    ) -> Result<(), ActionError> {
        body_text(body, "A reply cannot be empty.")?;
        let target = Target::parse(url)?;
        self.thread_in_scope(&target, thread).await?;
        self.graphql_write(
            "mutation($threadId:ID!,$body:String!) { addPullRequestReviewThreadReply(input:{pullRequestReviewThreadId:$threadId,body:$body}) { comment { id } } }",
            json!({ "threadId": thread, "body": body }),
        )
        .await?;
        self.forget(&target, false).await;
        Ok(())
    }

    pub async fn set_thread_resolved(
        &self,
        url: &str,
        thread: &str,
        resolved: bool,
    ) -> Result<(), ActionError> {
        let target = Target::parse(url)?;
        let controls = self.controls(&target).await?;
        if !(controls.can_write || controls.did_author) {
            return Err(ActionError::Refused(
                "Resolving a conversation needs write access or authorship of this pull request."
                    .into(),
            ));
        }
        self.thread_in_scope(&target, thread).await?;
        self.graphql_write(
            if resolved {
                "mutation($threadId:ID!) { resolveReviewThread(input:{threadId:$threadId}) { thread { isResolved } } }"
            } else {
                "mutation($threadId:ID!) { unresolveReviewThread(input:{threadId:$threadId}) { thread { isResolved } } }"
            },
            json!({ "threadId": thread }),
        )
        .await?;
        self.forget(&target, false).await;
        Ok(())
    }

    async fn thread_in_scope(&self, target: &Target, thread: &str) -> Result<(), ActionError> {
        if thread.is_empty() || thread.len() > 256 {
            return Err(ChangeRequestError::UnsupportedRepository.into());
        }
        let mut variables = target.variables();
        variables["id"] = thread.into();
        let mut budget = GITHUB_OUTPUT_LIMIT;
        let value = self
            .review_query(THREAD_SCOPE_QUERY.to_owned(), variables, &mut budget)
            .await?;
        let pull = value["data"]["repository"]["pullRequest"]["id"].as_str();
        let owner = value["data"]["node"]["pullRequest"]["id"].as_str();
        if pull.is_none() || pull != owner {
            return Err(ChangeRequestError::UnsupportedRepository.into());
        }
        Ok(())
    }

    async fn graphql_write(&self, query: &str, variables: Value) -> Result<(), ActionError> {
        let stdout = self
            .write(
                vec!["api".into(), "graphql".into(), "--input".into(), "-".into()],
                Some(json!({ "query": query, "variables": variables })),
            )
            .await?;
        let value: Value = serde_json::from_slice(&stdout).unwrap_or_default();
        if let Some(message) = value["errors"][0]["message"].as_str() {
            return Err(ActionError::Refused(readable_failure(message.as_bytes())));
        }
        Ok(())
    }

    async fn write(&self, args: Vec<String>, input: Option<Value>) -> Result<Vec<u8>, ActionError> {
        let stdin = input
            .map(|input| serde_json::to_vec(&input))
            .transpose()
            .map_err(|_| ChangeRequestError::Decode)?;
        let output = self
            .runner
            .run(ProcessRequest {
                args,
                stdin,
                ..github_request()
            })
            .await
            .map_err(classify_run_error)?;
        if output.success {
            return Ok(output.stdout);
        }
        match classify_github_failure(&output.stderr) {
            ChangeRequestError::CommandFailed => {
                Err(ActionError::Refused(readable_failure(&output.stderr)))
            }
            other => Err(other.into()),
        }
    }

    async fn forget(&self, target: &Target, lists: bool) {
        self.evict_detail(&target.url).await;
        if lists {
            let prefix = format!(
                "list:{}/{}:",
                target.owner.to_ascii_lowercase(),
                target.name.to_ascii_lowercase()
            );
            let mut cache = self.pr_cache.lock().await;
            cache.entries.retain(|entry| !entry.0.starts_with(&prefix));
            cache
                .in_flight
                .retain(|entry, _| !entry.starts_with(&prefix));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{FakeProcessRunner, command_success};
    use super::*;

    const URL: &str = "https://github.com/acme/zeron/pull/7";

    fn command_failure(stderr: &str) -> Result<ProcessOutput, ProcessRunError> {
        Ok(ProcessOutput {
            success: false,
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
            stdout_truncated: false,
        })
    }

    fn controls_json(permission: &str, did_author: bool) -> Result<ProcessOutput, ProcessRunError> {
        command_success(
            serde_json::to_vec(&json!({"data": {"repository": {
                "mergeCommitAllowed": true,
                "squashMergeAllowed": true,
                "rebaseMergeAllowed": false,
                "viewerPermission": permission,
                "pullRequest": {
                    "viewerDidAuthor": did_author,
                    "labels": {"nodes": [{"name": "bug", "color": "d73a4a", "description": null}]},
                    "reviewRequests": {"nodes": [
                        {"requestedReviewer": {"slug": "core", "name": "Core"}},
                        {"requestedReviewer": null}
                    ]}
                }
            }}}))
            .unwrap(),
        )
    }

    fn stdin_json(request: &ProcessRequest) -> Value {
        serde_json::from_slice(request.stdin.as_deref().unwrap()).unwrap()
    }

    #[tokio::test]
    async fn controls_follow_repository_permission_and_list_current_labels_and_requests() {
        let runner = FakeProcessRunner::with_responses([controls_json("WRITE", false)]);
        let controls = GitHubCli::with_runner(runner.clone())
            .change_request_controls(URL)
            .await
            .unwrap();
        assert_eq!(
            controls,
            ChangeRequestControls {
                merge_methods: vec![
                    ChangeRequestMergeMethod::Merge,
                    ChangeRequestMergeMethod::Squash
                ],
                can_write: true,
                can_triage: true,
                did_author: false,
                labels: vec![ChangeRequestLabelCandidate {
                    name: "bug".into(),
                    color: Some("d73a4a".into()),
                    description: None,
                    applied: true,
                }],
                reviewers: vec![ChangeRequestReviewerCandidate {
                    login: "core".into(),
                    name: Some("Core".into()),
                    team: true,
                    requested: true,
                }],
            }
        );
        let request = &runner.requests()[0];
        assert_eq!(request.args, ["api", "graphql", "--input", "-"]);
        assert_eq!(
            stdin_json(request)["variables"],
            json!({"owner": "acme", "name": "zeron", "number": 7})
        );
        let triage = to_controls(GhControlsRepository {
            viewer_permission: "triage".into(),
            ..Default::default()
        });
        assert!(!triage.can_write && triage.can_triage);
        assert!(
            GitHubCli::with_runner(FakeProcessRunner::with_responses([]))
                .change_request_controls("https://evil.test/acme/zeron/pull/7")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn merges_are_gated_and_return_host_words() {
        let runner =
            FakeProcessRunner::with_responses([controls_json("WRITE", false), command_success("")]);
        GitHubCli::with_runner(runner.clone())
            .merge(URL, ChangeRequestMergeMethod::Squash)
            .await
            .unwrap();
        assert_eq!(runner.requests()[1].args, ["pr", "merge", URL, "--squash"]);
        assert_eq!(runner.requests()[1].cwd, None);

        let refused = GitHubCli::with_runner(FakeProcessRunner::with_responses([
            controls_json("WRITE", false),
            command_failure("GraphQL: Pull request is not mergeable (mergePullRequest)\n"),
        ]))
        .merge(URL, ChangeRequestMergeMethod::Merge)
        .await
        .unwrap_err();
        assert_eq!(
            refused,
            ActionError::Refused(
                "GraphQL: Pull request is not mergeable (mergePullRequest)".into()
            )
        );

        for (permission, method, message) in [
            (
                "READ",
                ChangeRequestMergeMethod::Merge,
                "You need write access on this repository to merge.",
            ),
            (
                "WRITE",
                ChangeRequestMergeMethod::Rebase,
                "This repository does not allow the rebase strategy.",
            ),
        ] {
            let runner = FakeProcessRunner::with_responses([controls_json(permission, false)]);
            let error = GitHubCli::with_runner(runner.clone())
                .merge(URL, method)
                .await
                .unwrap_err();
            assert_eq!(error, ActionError::Refused(message.into()));
            assert_eq!(
                runner.requests().len(),
                1,
                "a refused merge never reaches gh pr"
            );
        }
    }

    #[test]
    fn candidates_put_applied_and_requested_first() {
        let labels: GhLabelCandidatesRepository = serde_json::from_value(json!({
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
        let reviewers: GhReviewerCandidatesRepository = serde_json::from_value(json!({
            "assignableUsers": {"pageInfo": {"hasNextPage": false}, "nodes": [
                {"login": "alice", "name": "Alice"},
                {"login": "Bob"},
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
        let runner = FakeProcessRunner::with_responses([
            controls_json("TRIAGE", false),
            command_success(""),
            controls_json("TRIAGE", false),
            command_success("[]"),
        ]);
        let github = GitHubCli::with_runner(runner.clone());
        github
            .set_label(URL, "good first issue/ä", false)
            .await
            .unwrap();
        github.set_label(URL, " bug ", true).await.unwrap();
        let requests = runner.requests();
        assert_eq!(
            requests[1].args,
            [
                "api",
                "--method",
                "DELETE",
                "repos/acme/zeron/issues/7/labels/good%20first%20issue%2F%C3%A4"
            ]
        );
        assert_eq!(stdin_json(&requests[3]), json!({"labels": ["bug"]}));

        let runner = FakeProcessRunner::with_responses([controls_json("READ", false)]);
        assert_eq!(
            GitHubCli::with_runner(runner.clone())
                .set_label(URL, "bug", true)
                .await
                .unwrap_err(),
            ActionError::Refused(
                "You need triage access on this repository to change its labels.".into()
            )
        );
        assert_eq!(runner.requests().len(), 1);
    }

    #[tokio::test]
    async fn reviewers_need_write_access_and_teams_go_by_slug() {
        let runner = FakeProcessRunner::with_responses([
            controls_json("MAINTAIN", false),
            command_success("{}"),
        ]);
        GitHubCli::with_runner(runner.clone())
            .set_reviewer(URL, "core", true, false)
            .await
            .unwrap();
        let request = &runner.requests()[1];
        assert_eq!(
            request.args,
            [
                "api",
                "--method",
                "DELETE",
                "repos/acme/zeron/pulls/7/requested_reviewers",
                "--input",
                "-"
            ]
        );
        assert_eq!(stdin_json(request), json!({"team_reviewers": ["core"]}));
        let github = GitHubCli::with_runner(FakeProcessRunner::with_responses([controls_json(
            "TRIAGE", false,
        )]));
        assert!(matches!(
            github.set_reviewer(URL, "octocat", false, true).await,
            Err(ActionError::Refused(_))
        ));
        assert!(matches!(
            github.set_reviewer(URL, "bad login", false, true).await,
            Err(ActionError::Refused(_))
        ));
    }

    #[tokio::test]
    async fn reviews_and_threads_send_text_on_stdin_and_stay_in_scope() {
        let scope = |owner: &str| {
            command_success(
                serde_json::to_vec(&json!({"data": {
                    "repository": {"pullRequest": {"id": "PR_7"}},
                    "node": {"pullRequest": {"id": owner}}
                }}))
                .unwrap(),
            )
        };
        let runner = FakeProcessRunner::with_responses([
            controls_json("WRITE", false),
            command_success("{}"),
            scope("PR_7"),
            command_success(r#"{"data":{}}"#),
            scope("PR_other"),
            controls_json("READ", true),
            scope("PR_7"),
            command_success(r#"{"data":{}}"#),
            controls_json("WRITE", true),
        ]);
        let github = GitHubCli::with_runner(runner.clone());
        github
            .submit_review(URL, ChangeRequestReviewEvent::RequestChanges, "please fix")
            .await
            .unwrap();
        assert_eq!(
            github
                .submit_review(URL, ChangeRequestReviewEvent::Comment, " ")
                .await
                .unwrap_err(),
            ActionError::Refused("A review needs a summary.".into())
        );
        github.reply_to_thread(URL, "T_7", "done").await.unwrap();
        assert_eq!(
            github
                .reply_to_thread(URL, "T_elsewhere", "done")
                .await
                .unwrap_err(),
            ActionError::Host(ChangeRequestError::UnsupportedRepository)
        );
        github.set_thread_resolved(URL, "T_7", true).await.unwrap();
        assert_eq!(
            github
                .submit_review(URL, ChangeRequestReviewEvent::Approve, "")
                .await
                .unwrap_err(),
            ActionError::Refused(
                "You cannot approve or request changes on your own pull request.".into()
            )
        );
        let requests = runner.requests();
        assert_eq!(requests.len(), 9);
        assert_eq!(
            requests[1].args.last().map(String::as_str),
            Some("-"),
            "the review body rides stdin"
        );
        assert_eq!(
            stdin_json(&requests[1]),
            json!({"event": "REQUEST_CHANGES", "body": "please fix"})
        );
        assert!(
            stdin_json(&requests[3])["query"]
                .as_str()
                .unwrap()
                .contains("addPullRequestReviewThreadReply")
        );
        assert_eq!(stdin_json(&requests[3])["variables"]["body"], "done");
        assert!(
            stdin_json(&requests[7])["query"]
                .as_str()
                .unwrap()
                .contains("resolveReviewThread")
        );
        assert!(
            requests
                .iter()
                .all(|request| !request.args.iter().any(|arg| arg.contains("please fix"))),
            "review text never rides argv"
        );
    }

    #[tokio::test]
    async fn a_write_evicts_the_cached_detail_and_a_merge_the_board() {
        let runner =
            FakeProcessRunner::with_responses([controls_json("WRITE", false), command_success("")]);
        let github = GitHubCli::with_runner(runner);
        {
            let mut cache = github.pr_cache.lock().await;
            for key in [
                format!("false:{URL}:"),
                format!("true:{URL}:"),
                "list:acme/zeron:All:".to_owned(),
                "list:acme/other:All:".to_owned(),
            ] {
                cache.entries.push((key, Instant::now(), Ok(Value::Null)));
            }
        }
        github
            .merge(URL, ChangeRequestMergeMethod::Merge)
            .await
            .unwrap();
        let cache = github.pr_cache.lock().await;
        let keys: Vec<_> = cache.entries.iter().map(|entry| entry.0.as_str()).collect();
        assert_eq!(
            keys,
            [format!("true:{URL}:").as_str(), "list:acme/other:All:"]
        );
    }
}
