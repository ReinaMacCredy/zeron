use std::collections::HashMap;
use std::sync::Arc;

use zeron_proto::comments::ReviewComment;
use zeron_proto::review::{
    self, ChatReview, CommitPick, FinishedRead, Look, ReadOutcome, ReviewScope, ReviewSettings,
    ReviewerChoice, ReviewerOptions, RunStatus, StartedReview,
};
use zeron_proto::{ChatIndicator, CheckoutDiff, GitHistoryCommit, HarnessId, ReasoningLevel};
use zeron_rpc::methods;

use super::{CoreState, SessionCore, SessionHandle, lock};
use crate::catalog::ModelInfo;
use crate::client::{Client, ClientInner};
use crate::error::{ClientError, Result};

const COMMIT_LIST_LIMIT: usize = 30;

#[derive(Debug, Default)]
pub(super) struct ReviewState {
    chat: ChatReview,
    options: Option<ReviewerOptions>,
    settings: Option<ReviewSettings>,
    models: HashMap<HarnessId, Vec<ModelInfo>>,
    commits: Option<Vec<GitHistoryCommit>>,
    staged: Vec<ReviewComment>,
    options_loading: bool,
    hand_busy: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReviewChip {
    pub scope: ReviewScope,
    pub look: Look,
    pub picked: Option<String>,
    pub stoppable: bool,
    pub next_reviewer: Option<ReviewerChoice>,
}

#[derive(Debug, Clone)]
pub struct ReviewSnapshot {
    pub review: ChatReview,
    pub scope: ReviewScope,
    pub options: Option<ReviewerOptions>,
    pub settings: Option<ReviewSettings>,
    pub effective: ReviewerChoice,
    pub models: HashMap<HarnessId, Vec<ModelInfo>>,
    pub commits: Option<Vec<GitHistoryCommit>>,
    pub branch: Option<String>,
    pub harness: Option<HarnessId>,
}

#[derive(Debug, Clone)]
pub(super) struct Target {
    chat_id: String,
    device_id: String,
    cwd: String,
    branch: Option<String>,
    worktree: bool,
    harness: Option<HarnessId>,
}

pub(super) fn target(client: &ClientInner, chat_id: &str) -> Option<Target> {
    let chat = client.workspace.chat(chat_id)?;
    if chat.delegation.is_some() || chat.sidechat_source_chat_id.is_some() {
        return None;
    }
    let snapshot = client.workspace.snapshot();
    let project = snapshot
        .project(chat.space_id.as_deref()?)
        .filter(|project| project.git_detected && project.device_online)?;
    let cwd = chat.cwd.clone().unwrap_or_else(|| project.path.clone());
    Some(Target {
        chat_id: chat.id,
        device_id: chat.device_id,
        worktree: cwd != project.path,
        cwd,
        branch: chat.branch,
        harness: chat.config.map(|config| config.harness),
    })
}

pub(super) fn chip(st: &CoreState, target: Option<&Target>) -> Option<ReviewChip> {
    let target = target?;
    let review = &st.review.chat;
    let scope = review
        .scope
        .unwrap_or_else(|| ReviewScope::default_for(target.worktree));
    let look = review.look(scope)?;
    let effective = review.effective_reviewer(st.review.settings.as_ref());
    let busy = matches!(look, Look::Busy);
    Some(ReviewChip {
        scope,
        picked: review.commit.as_ref().map(|pick| pick.short().to_string()),
        stoppable: busy
            && review
                .run
                .as_ref()
                .is_some_and(review::ReviewRun::stoppable),
        next_reviewer: (!busy
            && !matches!(look, Look::Findings(_))
            && review.reviewer_changed(&effective))
        .then_some(effective),
        look,
    })
}

pub(super) fn staged(st: &CoreState) -> Vec<ReviewComment> {
    st.review.staged.clone()
}

fn client_of(inner: &Arc<ClientInner>) -> Client {
    Client {
        inner: inner.clone(),
    }
}

fn parse<T: serde::de::DeserializeOwned>(wire: &str) -> Option<T> {
    serde_json::from_value(serde_json::Value::String(wire.to_owned())).ok()
}

fn levels(wire: &[String]) -> Vec<ReasoningLevel> {
    wire.iter().filter_map(|level| parse(level)).collect()
}

pub fn model_levels(model: &ModelInfo) -> Vec<ReasoningLevel> {
    levels(&model.reasoning_levels)
}

impl Client {
    pub async fn reviewer_options(&self, device_id: &str) -> ReviewerOptions {
        let offered: Vec<(HarnessId, Vec<ReasoningLevel>)> = self
            .list_harnesses(device_id)
            .await
            .into_iter()
            .filter(|harness| harness.offered())
            .filter_map(|harness| Some((parse(&harness.id)?, levels(&harness.reasoning_levels))))
            .collect();
        ReviewerOptions {
            offered: offered.iter().map(|(id, _)| *id).collect(),
            levels: offered.into_iter().collect(),
            profiles: self
                .list_agent_profiles(device_id)
                .await
                .unwrap_or_default(),
        }
    }

    pub async fn review_settings(&self, device_id: &str) -> Result<ReviewSettings> {
        self.host_call(
            device_id,
            methods::GET_REVIEW_SETTINGS,
            serde_json::json!({}),
        )
        .await
    }

    pub async fn set_review_settings(
        &self,
        device_id: &str,
        settings: &ReviewSettings,
    ) -> Result<ReviewSettings> {
        self.host_call(
            device_id,
            methods::SET_REVIEW_SETTINGS,
            serde_json::to_value(settings).unwrap_or_default(),
        )
        .await
    }
}

async fn checkout_diff(
    client: &Arc<ClientInner>,
    target: &Target,
    scope: ReviewScope,
    commit: Option<&str>,
) -> std::result::Result<(CheckoutDiff, Option<String>), String> {
    let branches: Vec<String> = match scope {
        ReviewScope::Branch => serde_json::from_value(
            client
                .host_rpc(
                    &target.device_id,
                    methods::LIST_BRANCHES,
                    serde_json::json!({ "repoPath": target.cwd }),
                )
                .await
                .map_err(|err| err.to_string())?,
        )
        .unwrap_or_default(),
        _ => Vec::new(),
    };
    let base = review::diff_base(scope, commit, &branches, target.branch.as_deref())?;
    let value = client
        .host_rpc(
            &target.device_id,
            methods::GET_CHECKOUT_DIFF,
            review::checkout_diff_params(
                &target.cwd,
                &target.chat_id,
                scope,
                commit,
                base.as_deref(),
            ),
        )
        .await
        .map_err(|err| err.to_string())?;
    let diff = serde_json::from_value::<CheckoutDiff>(value).map_err(|err| err.to_string())?;
    Ok((diff, base))
}

impl SessionCore {
    fn review_core(&self, client: &Arc<ClientInner>) -> Option<Arc<SessionCore>> {
        client.session_core(&self.chat_id)
    }

    pub(super) fn sync_review(&self, client: &Arc<ClientInner>) {
        let Some(target) = target(client, &self.chat_id) else {
            return;
        };
        let (workspace, _) = client.workspace.state();
        let chats = &workspace.chats;
        let owner_working = client
            .workspace
            .snapshot()
            .session(&self.chat_id)
            .is_some_and(|row| row.host_indicator == ChatIndicator::Working);
        let owner = target.chat_id.as_str();
        let (changed, read, hand, stats, load_options) = {
            let mut st = lock(&self.state);
            let state = &mut st.review;
            let mut changed = state.chat.adopt(chats, owner);
            let read = state.chat.finished(chats, owner);
            if let (Some(_), Some(run)) = (&read, state.chat.run.as_mut()) {
                run.status = RunStatus::Reading;
                changed = true;
            }
            let delegation = state
                .chat
                .run
                .as_ref()
                .and_then(|run| run.task_id.as_deref())
                .and_then(|task| review::reviewer_child(chats, owner, Some(task)))
                .and_then(|child| child.delegation.clone());
            let mut hand = None;
            if let Some(run) = state.chat.run.as_mut() {
                changed |= run.sync_child(delegation, owner_working);
                if !state.hand_busy
                    && let (Some(task), Some(next)) = (run.task_id.clone(), run.next_hand())
                {
                    state.hand_busy = true;
                    hand = Some((task, next));
                }
            }
            let scope = state
                .chat
                .scope
                .unwrap_or_else(|| ReviewScope::default_for(target.worktree));
            let run_scope = state
                .chat
                .run
                .as_ref()
                .filter(|run| run.status == RunStatus::Done)
                .map(|run| run.scope);
            let commit = state.chat.commit.as_ref().map(|pick| pick.sha.clone());
            let key = format!(
                "{}|{scope:?}|{run_scope:?}|{owner_working}|{}",
                target.cwd,
                commit.clone().unwrap_or_default()
            );
            let stats = (state.chat.stats_key.as_deref() != Some(key.as_str())).then(|| {
                state.chat.stats_key = Some(key.clone());
                let mut scopes = vec![scope];
                scopes.extend(run_scope.filter(|run_scope| *run_scope != scope));
                (scopes, Some(key))
            });
            let load_options = state.options.is_none() && !state.options_loading;
            state.options_loading |= load_options;
            (changed, read, hand, stats, load_options)
        };
        if changed {
            self.recompute_composer(client);
        }
        let Some(core) = self.review_core(client) else {
            return;
        };
        if let Some(read) = read {
            core.clone()
                .read_finished(client.clone(), target.clone(), read);
        }
        if let Some((task, (ids, withdraw))) = hand {
            core.clone()
                .hand_findings(client.clone(), target.clone(), task, ids, withdraw);
        }
        if let Some((scopes, key)) = stats {
            core.clone()
                .fetch_review_stats(client.clone(), target.clone(), scopes, key);
        }
        if load_options {
            core.load_reviewer_options(client.clone(), target.device_id.clone());
        }
    }

    fn read_finished(
        self: Arc<Self>,
        client: Arc<ClientInner>,
        target: Target,
        read: FinishedRead,
    ) {
        crate::runtime::shared().spawn(async move {
            let FinishedRead {
                task_id,
                scope,
                reviewed,
                commit,
            } = read;
            let view = client
                .host_rpc(
                    &target.device_id,
                    methods::TASK_STATUS,
                    serde_json::json!({ "chatId": target.chat_id, "taskId": task_id }),
                )
                .await;
            let restored = reviewed.is_none();
            let reviewed = match reviewed {
                Some(patch) => Some(patch),
                None => checkout_diff(&client, &target, scope, commit.as_deref())
                    .await
                    .ok()
                    .map(|(diff, _)| diff.patch),
            };
            let outcome = match view {
                Err(err) => ReadOutcome::Failed(err.to_string()),
                Ok(view) => review::read_task_view(&view),
            };
            self.update_state(|st| {
                if let Some(run) = st.review.chat.run.as_mut()
                    && run.task_id.as_deref() == Some(task_id.as_str())
                {
                    run.apply_read(outcome, reviewed.as_deref(), restored);
                }
            });
        });
    }

    fn hand_findings(
        self: Arc<Self>,
        client: Arc<ClientInner>,
        target: Target,
        task_id: String,
        ids: Vec<String>,
        withdraw: bool,
    ) {
        crate::runtime::shared().spawn(async move {
            let handed = client
                .host_rpc(
                    &target.device_id,
                    methods::HAND_REVIEW_FINDINGS,
                    review::hand_params(&target.chat_id, &task_id, &ids, withdraw),
                )
                .await;
            if let Err(err) = &handed {
                tracing::warn!(error = %err, "review findings not handed");
            }
            self.update_state(|st| {
                st.review.hand_busy = false;
                if handed.is_err()
                    && let Some(run) = st.review.chat.run.as_mut()
                    && run.task_id.as_deref() == Some(task_id.as_str())
                {
                    run.hand_failed(&ids, withdraw);
                }
            });
        });
    }

    fn fetch_review_stats(
        self: Arc<Self>,
        client: Arc<ClientInner>,
        target: Target,
        scopes: Vec<ReviewScope>,
        key: Option<String>,
    ) {
        let commit = lock(&self.state)
            .review
            .chat
            .commit
            .as_ref()
            .map(|pick| pick.sha.clone());
        crate::runtime::shared().spawn(async move {
            let mut fetched = Vec::new();
            for scope in scopes {
                fetched.push((
                    scope,
                    checkout_diff(&client, &target, scope, commit.as_deref()).await,
                ));
            }
            self.update_state(|st| {
                let review = &mut st.review.chat;
                if key.is_some() && review.stats_key != key {
                    return;
                }
                for (scope, result) in fetched {
                    review.apply_stats(scope, result);
                }
            });
        });
    }

    fn load_reviewer_options(self: Arc<Self>, client: Arc<ClientInner>, device_id: String) {
        crate::runtime::shared().spawn(async move {
            let host = client_of(&client);
            let options = host.reviewer_options(&device_id).await;
            let settings = host.review_settings(&device_id).await.unwrap_or_default();
            self.update_state(|st| {
                st.review.options = Some(options);
                st.review.settings = Some(settings);
                st.review.options_loading = false;
            });
        });
    }

    pub(super) fn take_staged_comments(&self, sent: &[ReviewComment]) {
        let mut st = lock(&self.state);
        st.review
            .staged
            .retain(|comment| !sent.iter().any(|s| s.id == comment.id));
    }
}

impl SessionHandle {
    fn review_target(&self) -> Result<(Arc<ClientInner>, Target)> {
        let client = self.core.client()?;
        let target = target(&client, &self.core.chat_id).ok_or_else(|| {
            ClientError::Unsupported("Review needs a git project on an online host".into())
        })?;
        Ok((client, target))
    }

    pub fn review_snapshot(&self) -> Option<ReviewSnapshot> {
        let (_, target) = self.review_target().ok()?;
        let st = lock(&self.core.state);
        let state = &st.review;
        Some(ReviewSnapshot {
            scope: state
                .chat
                .scope
                .unwrap_or_else(|| ReviewScope::default_for(target.worktree)),
            effective: state.chat.effective_reviewer(state.settings.as_ref()),
            review: state.chat.clone(),
            options: state.options.clone(),
            settings: state.settings.clone(),
            models: state.models.clone(),
            commits: state.commits.clone(),
            branch: target.branch,
            harness: target.harness,
        })
    }

    pub fn open_review_options(&self) -> Result<()> {
        let (client, target) = self.review_target()?;
        let load = {
            let mut st = lock(&self.core.state);
            let load = !st.review.options_loading;
            st.review.options_loading = true;
            load
        };
        let core = self.core.clone();
        core.clone().fetch_review_stats(
            client.clone(),
            target.clone(),
            ReviewScope::ALL.to_vec(),
            None,
        );
        if load {
            core.load_reviewer_options(client, target.device_id);
        }
        Ok(())
    }

    pub fn set_review_scope(&self, scope: ReviewScope) {
        self.core
            .update_state(|st| st.review.chat.scope = Some(scope));
    }

    pub fn set_reviewer(&self, choice: ReviewerChoice) {
        self.core
            .update_state(|st| st.review.chat.reviewer = Some(choice));
    }

    pub fn load_review_commits(&self) -> Result<()> {
        let (client, target) = self.review_target()?;
        let core = self.core.clone();
        crate::runtime::shared().spawn(async move {
            let page = client_of(&client)
                .git_history(&target.device_id, &target.cwd, 0, COMMIT_LIST_LIMIT)
                .await;
            core.update_state(|st| {
                st.review.commits = Some(page.map(|page| page.commits).unwrap_or_default());
            });
        });
        Ok(())
    }

    pub fn pick_review_commit(&self, sha: &str, subject: &str) -> Result<()> {
        let (client, target) = self.review_target()?;
        self.core.update_state(|st| {
            let review = &mut st.review.chat;
            review.commit = Some(CommitPick {
                sha: sha.to_owned(),
                subject: subject.to_owned(),
            });
            review.scope = Some(ReviewScope::Commit);
            review.stats.remove(&ReviewScope::Commit);
        });
        self.core
            .clone()
            .fetch_review_stats(client, target, vec![ReviewScope::Commit], None);
        Ok(())
    }

    pub fn load_reviewer_models(&self, harness: HarnessId) -> Result<()> {
        let (client, target) = self.review_target()?;
        let Some(wire) = serde_json::to_value(harness)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
        else {
            return Ok(());
        };
        let core = self.core.clone();
        crate::runtime::shared().spawn(async move {
            let models = client_of(&client)
                .list_models(&target.device_id, &wire)
                .await;
            core.update_state(|st| {
                st.review.models.insert(harness, models);
            });
        });
        Ok(())
    }

    pub async fn start_review(&self, scope: ReviewScope) -> Result<()> {
        let (client, target) = self.review_target()?;
        let (previous, chosen, commit) = {
            let mut st = lock(&self.core.state);
            let state = &mut st.review;
            let effective = state.chat.effective_reviewer(state.settings.as_ref());
            let previous = state.chat.begin(scope, effective);
            (
                previous,
                state.chat.reviewer.clone(),
                state.chat.run.as_ref().and_then(|run| run.commit.clone()),
            )
        };
        self.core.recompute_composer(&client);
        let started = client
            .host_rpc(
                &target.device_id,
                methods::START_REVIEW,
                review::start_params(&target.chat_id, scope, commit.as_deref(), chosen.as_ref()),
            )
            .await
            .map_err(|err| err.to_string())
            .and_then(|value| {
                serde_json::from_value::<StartedReview>(value)
                    .ok()
                    .filter(|started| !started.task_id.is_empty())
                    .ok_or_else(|| "The reviewer did not start".to_string())
            });
        let failed = started.as_ref().err().cloned();
        self.core.update_state(|st| {
            if let Some(run) = st.review.chat.run.as_mut() {
                run.apply_start(started, previous);
            }
        });
        match failed {
            Some(message) => Err(ClientError::HostError(message)),
            None => Ok(()),
        }
    }

    pub async fn stop_review(&self) -> Result<()> {
        let task = lock(&self.core.state)
            .review
            .chat
            .run
            .as_ref()
            .filter(|run| run.status == RunStatus::Running)
            .and_then(|run| run.task_id.clone());
        match task {
            Some(task) => self.stop_background_run(&task, true).await,
            None => Ok(()),
        }
    }

    pub fn fix_review_findings(&self, only: Option<&str>) -> usize {
        let mut staged = 0;
        self.core.update_state(|st| {
            let comments = st.review.chat.take_fixable(only);
            staged = comments.len();
            for comment in comments {
                if !st.review.staged.iter().any(|known| known.id == comment.id) {
                    st.review.staged.push(comment);
                }
            }
        });
        self.sync_review_now();
        staged
    }

    pub fn dismiss_review_finding(&self, id: &str) {
        self.core.update_state(|st| st.review.chat.dismiss(id));
    }

    pub fn remove_staged_comment(&self, id: &str) {
        self.core.update_state(|st| {
            st.review.staged.retain(|comment| comment.id != id);
            st.review.chat.withdraw_comment(id);
        });
        self.sync_review_now();
    }

    fn sync_review_now(&self) {
        if let Ok(client) = self.core.client() {
            self.core.sync_review(&client);
        }
    }
}
