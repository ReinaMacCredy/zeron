use serde::de::DeserializeOwned;
use serde_json::json;
use zeron_proto::{
    ChangeRequestComment, ChangeRequestControls, ChangeRequestDetail, ChangeRequestFilter,
    ChangeRequestLabelCandidates, ChangeRequestListItem, ChangeRequestMergeMethod,
    ChangeRequestPage, ChangeRequestReviewEvent, ChangeRequestReviewerCandidates, CheckoutDiff,
    GenerateCommitMessageRequest, GeneratedCommitMessage, GetWorkingTreeFileDiffRequest,
    GetWorkingTreeRequest, GitAgentBusyPolicy, GitHistoryPage, RunWorkingTreeActionRequest,
    SyncBranchReply, SyncBranchRequest, SyncDirection, WorkingTree, WorkingTreeAction,
    WorkingTreeActionReply, WorkingTreeFileDiff, WorkspaceTarget,
};
use zeron_rpc::methods;

use super::Client;
use crate::error::{ClientError, Result};

impl Client {
    pub(crate) async fn host_call<T: DeserializeOwned>(
        &self,
        device_id: &str,
        method: &'static str,
        params: serde_json::Value,
    ) -> Result<T> {
        let value = self.inner.host_rpc(device_id, method, params).await?;
        serde_json::from_value(value).map_err(|e| ClientError::HostError(e.to_string()))
    }

    async fn host_write(
        &self,
        device_id: &str,
        method: &'static str,
        params: serde_json::Value,
    ) -> Result<()> {
        self.inner.host_rpc(device_id, method, params).await?;
        Ok(())
    }

    pub async fn pull_request_page(
        &self,
        device_id: &str,
        repository: &str,
        filter: ChangeRequestFilter,
        after: Option<String>,
        refresh: bool,
    ) -> Result<ChangeRequestPage> {
        self.host_call(
            device_id,
            methods::LIST_CHANGE_REQUEST_PAGE,
            json!({
                "repository": repository,
                "filter": filter,
                "after": after,
                "refresh": refresh,
                "cached": after.is_none() && !refresh,
            }),
        )
        .await
    }

    pub async fn checkout_repository(
        &self,
        device_id: &str,
        cwd: &str,
        repository: Option<&str>,
    ) -> Result<Option<String>> {
        self.host_call(
            device_id,
            methods::GET_CHANGE_REQUEST_REPOSITORY,
            json!({ "cwd": cwd, "repository": repository }),
        )
        .await
    }

    pub async fn pull_request_repositories(&self, device_id: &str) -> Vec<(String, String)> {
        let projects: Vec<(String, String)> = self
            .workspace()
            .projects
            .iter()
            .filter(|p| p.device_id == device_id && p.git_detected)
            .map(|p| (p.id.clone(), p.path.clone()))
            .collect();
        let mut found: Vec<(String, String)> = Vec::new();
        for (project_id, path) in projects {
            if let Ok(Some(repository)) = self.checkout_repository(device_id, &path, None).await
                && !found
                    .iter()
                    .any(|(known, _)| known.eq_ignore_ascii_case(&repository))
            {
                found.push((repository, project_id));
            }
        }
        found
    }

    pub(crate) async fn attention_candidates(
        &self,
        device_id: &str,
    ) -> Option<Vec<ChangeRequestListItem>> {
        let mut items = Vec::new();
        for (repository, _) in self.pull_request_repositories(device_id).await {
            for filter in [
                ChangeRequestFilter::Authored,
                ChangeRequestFilter::Reviewing,
            ] {
                let page = self
                    .pull_request_page(device_id, &repository, filter, None, false)
                    .await
                    .ok()?;
                items.extend(page.items);
            }
        }
        Some(items)
    }

    pub async fn pull_request(
        &self,
        device_id: &str,
        url: &str,
        refresh: bool,
    ) -> Result<ChangeRequestDetail> {
        self.host_call(
            device_id,
            methods::GET_CHANGE_REQUEST,
            json!({ "url": url, "refresh": refresh }),
        )
        .await
    }

    pub async fn pull_request_diff(
        &self,
        device_id: &str,
        detail: &ChangeRequestDetail,
        refresh: bool,
    ) -> Result<String> {
        let mut params = json!({ "url": detail.url, "refresh": refresh });
        let valid = zeron_proto::change_request_assessment::valid_head_oid;
        if valid(&detail.head_ref_oid) {
            params["headRefOid"] = detail.head_ref_oid.clone().into();
        }
        if valid(&detail.base_ref_oid) {
            params["baseRefOid"] = detail.base_ref_oid.clone().into();
        }
        self.host_call(device_id, methods::GET_CHANGE_REQUEST_DIFF, params)
            .await
    }

    pub async fn pull_request_controls(
        &self,
        device_id: &str,
        url: &str,
    ) -> Result<ChangeRequestControls> {
        self.host_call(
            device_id,
            methods::GET_CHANGE_REQUEST_CONTROLS,
            json!({ "url": url }),
        )
        .await
    }

    pub async fn pull_request_label_candidates(
        &self,
        device_id: &str,
        url: &str,
    ) -> Result<ChangeRequestLabelCandidates> {
        self.host_call(
            device_id,
            methods::LIST_CHANGE_REQUEST_LABEL_CANDIDATES,
            json!({ "url": url }),
        )
        .await
    }

    pub async fn pull_request_reviewer_candidates(
        &self,
        device_id: &str,
        url: &str,
    ) -> Result<ChangeRequestReviewerCandidates> {
        self.host_call(
            device_id,
            methods::LIST_CHANGE_REQUEST_REVIEWER_CANDIDATES,
            json!({ "url": url }),
        )
        .await
    }

    pub async fn post_pull_request_comment(
        &self,
        device_id: &str,
        url: &str,
        body: &str,
    ) -> Result<ChangeRequestComment> {
        self.host_call(
            device_id,
            methods::POST_CHANGE_REQUEST_COMMENT,
            json!({ "url": url, "body": body }),
        )
        .await
    }

    pub async fn reply_to_pull_request_thread(
        &self,
        device_id: &str,
        url: &str,
        thread_id: &str,
        body: &str,
    ) -> Result<()> {
        self.host_write(
            device_id,
            methods::REPLY_TO_CHANGE_REQUEST_THREAD,
            json!({ "url": url, "threadId": thread_id, "body": body }),
        )
        .await
    }

    pub async fn set_pull_request_thread_resolved(
        &self,
        device_id: &str,
        url: &str,
        thread_id: &str,
        resolved: bool,
    ) -> Result<()> {
        self.host_write(
            device_id,
            methods::SET_CHANGE_REQUEST_THREAD_RESOLVED,
            json!({ "url": url, "threadId": thread_id, "resolved": resolved }),
        )
        .await
    }

    pub async fn submit_pull_request_review(
        &self,
        device_id: &str,
        url: &str,
        event: ChangeRequestReviewEvent,
        body: &str,
    ) -> Result<()> {
        self.host_write(
            device_id,
            methods::SUBMIT_CHANGE_REQUEST_REVIEW,
            json!({ "url": url, "event": event, "body": body }),
        )
        .await
    }

    pub async fn set_pull_request_label(
        &self,
        device_id: &str,
        url: &str,
        name: &str,
        add: bool,
    ) -> Result<()> {
        self.host_write(
            device_id,
            methods::SET_CHANGE_REQUEST_LABEL,
            json!({ "url": url, "name": name, "add": add }),
        )
        .await
    }

    pub async fn set_pull_request_reviewer(
        &self,
        device_id: &str,
        url: &str,
        login: &str,
        team: bool,
        add: bool,
    ) -> Result<()> {
        self.host_write(
            device_id,
            methods::SET_CHANGE_REQUEST_REVIEWER,
            json!({ "url": url, "login": login, "team": team, "add": add }),
        )
        .await
    }

    pub async fn merge_pull_request(
        &self,
        device_id: &str,
        url: &str,
        method: ChangeRequestMergeMethod,
    ) -> Result<()> {
        self.host_write(
            device_id,
            methods::MERGE_CHANGE_REQUEST,
            json!({ "url": url, "method": method }),
        )
        .await
    }

    pub async fn working_tree(
        &self,
        device_id: &str,
        target: WorkspaceTarget,
    ) -> Result<WorkingTree> {
        let request = GetWorkingTreeRequest { target };
        self.host_call(device_id, methods::GET_WORKING_TREE, to_params(&request))
            .await
    }

    pub async fn working_tree_file_diff(
        &self,
        device_id: &str,
        target: WorkspaceTarget,
        path: String,
        staged: bool,
    ) -> Result<WorkingTreeFileDiff> {
        let request = GetWorkingTreeFileDiffRequest {
            target,
            path,
            staged,
        };
        self.host_call(
            device_id,
            methods::GET_WORKING_TREE_FILE_DIFF,
            to_params(&request),
        )
        .await
    }

    // The phone never discards, drops a stash, checks out or cherry-picks (decision 1).
    pub async fn run_working_tree_action(
        &self,
        device_id: &str,
        target: WorkspaceTarget,
        expected_revision: String,
        action: WorkingTreeAction,
    ) -> Result<WorkingTreeActionReply> {
        if !matches!(
            action,
            WorkingTreeAction::Stage { .. }
                | WorkingTreeAction::Unstage { .. }
                | WorkingTreeAction::StageAll
                | WorkingTreeAction::UnstageAll
                | WorkingTreeAction::Commit { .. }
        ) {
            return Err(ClientError::InvalidArgument(
                "this Git action is not available here".into(),
            ));
        }
        let request = RunWorkingTreeActionRequest {
            target,
            expected_revision,
            busy_policy: GitAgentBusyPolicy::default(),
            action,
        };
        self.host_call(
            device_id,
            methods::RUN_WORKING_TREE_ACTION,
            to_params(&request),
        )
        .await
    }

    pub async fn sync_branch(
        &self,
        device_id: &str,
        target: WorkspaceTarget,
        expected_head_sha: String,
        direction: SyncDirection,
    ) -> Result<SyncBranchReply> {
        let request = SyncBranchRequest {
            target,
            expected_head_sha,
            busy_policy: GitAgentBusyPolicy::default(),
            direction,
        };
        self.host_call(device_id, methods::SYNC_BRANCH, to_params(&request))
            .await
    }

    pub async fn generate_commit_message(
        &self,
        device_id: &str,
        target: WorkspaceTarget,
    ) -> Result<GeneratedCommitMessage> {
        let request = GenerateCommitMessageRequest {
            target,
            instruction: None,
            harness: None,
            model: None,
        };
        self.host_call(
            device_id,
            methods::GENERATE_COMMIT_MESSAGE,
            to_params(&request),
        )
        .await
    }

    pub async fn git_history(
        &self,
        device_id: &str,
        cwd: &str,
        cursor: usize,
        limit: usize,
    ) -> Result<GitHistoryPage> {
        self.host_call(
            device_id,
            methods::LIST_GIT_HISTORY,
            json!({ "cwd": cwd, "cursor": cursor, "limit": limit }),
        )
        .await
    }

    pub async fn commit_diff(&self, device_id: &str, cwd: &str, sha: &str) -> Result<CheckoutDiff> {
        self.host_call(
            device_id,
            methods::GET_CHECKOUT_DIFF,
            json!({ "cwd": cwd, "mode": "commit", "commitSha": sha }),
        )
        .await
    }
}

fn to_params(request: &impl serde::Serialize) -> serde_json::Value {
    serde_json::to_value(request).unwrap_or_default()
}
