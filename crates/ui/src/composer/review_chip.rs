use std::collections::HashMap;

use gpui::{
    AnyElement, App, Bounds, Context, Pixels, Point, SharedString, Task, div, prelude::*, px,
};
use zeron_proto::{CheckoutDiff, HarnessId, Model};
use zeron_rpc::methods;

use super::{Composer, ComposerEvent};
use crate::changes::{default_base_ref, parse_patch};
use crate::popover::{self, Popup};
use crate::review::{
    self, FindingState, ReviewRun, ReviewScope, ReviewSettings, ReviewerChoice, ReviewerOptions,
    RunStatus,
};
use crate::theme::Theme;

const CHIP_HEIGHT: f32 = 28.0;
const COMMITS_ROW: usize = usize::MAX;
const COMMIT_LIST_LIMIT: usize = 30;

#[derive(Clone, Copy, PartialEq)]
enum Submenu {
    Models(HarnessId),
    Commits,
}

#[derive(Default)]
pub(super) struct ReviewChipUi {
    pub(super) menu: Popup<()>,
    options: Option<ReviewerOptions>,
    settings: Option<ReviewSettings>,
    options_task: Option<Task<()>>,
    submenu: Option<(usize, Submenu)>,
    submenu_intent: popover::HoverIntent<usize>,
    submenu_bounds: Option<Bounds<Pixels>>,
    submenu_left: bool,
    commits: Option<Vec<zeron_proto::GitHistoryCommit>>,
    commits_task: Option<Task<()>>,
    commit_filter: Option<gpui::Entity<super::ComposerInput>>,
    models: HashMap<HarnessId, Vec<Model>>,
    models_task: Option<Task<()>>,
    busy_hover: bool,
    thinking: crate::pickers::LevelSlider,
    hand_task: Option<Task<()>>,
    debug_opened: bool,
}

#[derive(Clone)]
struct Target {
    chat_id: String,
    cwd: String,
    branch: Option<String>,
    worktree: bool,
    harness: Option<HarnessId>,
}

enum Look {
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

async fn checkout_diff(
    engine: &crate::state::EngineHandle,
    target: &Target,
    scope: ReviewScope,
    commit: Option<&str>,
) -> Result<(CheckoutDiff, Option<String>), String> {
    if scope == ReviewScope::Commit && commit.is_none() {
        return Err("Pick a commit".to_string());
    }
    let base = match scope {
        ReviewScope::Branch => {
            let branches = engine
                .client()
                .call(
                    methods::LIST_BRANCHES,
                    serde_json::json!({ "repoPath": target.cwd }),
                )
                .await
                .map_err(|err| err.to_string())?;
            let branches: Vec<String> = serde_json::from_value(branches).unwrap_or_default();
            Some(
                default_base_ref(&branches, target.branch.as_deref())
                    .ok_or_else(|| "No base branch to compare against".to_string())?,
            )
        }
        _ => None,
    };
    let mut params = serde_json::json!({
        "cwd": target.cwd,
        "mode": scope.mode(),
        "chatId": target.chat_id,
        "commitSha": commit,
    });
    if let Some(base) = &base {
        params["baseRef"] = serde_json::Value::String(base.clone());
    }
    let value = engine
        .client()
        .call(methods::GET_CHECKOUT_DIFF, params)
        .await
        .map_err(|err| err.to_string())?;
    let diff = serde_json::from_value::<CheckoutDiff>(value).map_err(|err| err.to_string())?;
    Ok((diff, base))
}

fn chip_look(review: &review::ChatReview, scope: ReviewScope) -> Option<Look> {
    let counts = review.changes(scope).filter(|(add, del)| add + del > 0);
    let idle = || match counts {
        Some((add, del)) => Some(Look::Idle(add, del)),
        None => review.scope.is_some().then(|| {
            Look::Empty(match (review.changes(scope), review.unavailable(scope)) {
                (Some(_), _) => "No changes".to_string(),
                (None, Some(message)) => capitalize(message),
                (None, None) => "…".to_string(),
            })
        }),
    };
    let Some(run) = &review.run else {
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

fn outcome_counts(run: &ReviewRun) -> String {
    let count = |states: &[FindingState]| {
        run.findings
            .iter()
            .filter(|finding| states.contains(&run.state(&finding.id)))
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

fn badge_look(
    state: FindingState,
    theme: &Theme,
) -> (gpui::Hsla, Option<gpui::Hsla>, Option<gpui::Hsla>) {
    match state {
        FindingState::Fixed => (theme.success, Some(theme.success.opacity(0.12)), None),
        FindingState::Skipped => (theme.warning, Some(theme.warning.opacity(0.12)), None),
        FindingState::NoChange => (theme.text_muted, Some(crate::theme::ink(0.08)), None),
        FindingState::Open => (theme.text_muted, None, Some(theme.border_strong)),
        FindingState::AgentOnIt => (theme.accent, None, Some(theme.accent.opacity(0.6))),
    }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

async fn launch_review(
    engine: &crate::state::EngineHandle,
    target: &Target,
    scope: ReviewScope,
    commit: Option<&str>,
    choice: Option<&ReviewerChoice>,
) -> Result<(String, String, Option<String>), String> {
    let started = engine
        .client()
        .call(
            methods::START_REVIEW,
            serde_json::json!({
                "ownerChatId": target.chat_id,
                "scope": scope.key(),
                "commitSha": commit,
                "reviewer": choice,
            }),
        )
        .await
        .map_err(|err| err.to_string())?;
    let task_id = started
        .get("taskId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "The reviewer did not start".to_string())?
        .to_string();
    let patch = started
        .get("patch")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let note = started
        .get("note")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    Ok((task_id, patch, note))
}

impl Composer {
    fn hosts_review(&self) -> bool {
        self.chat_override.is_none() && !self.side_chat
    }

    fn review_target(&self, cx: &App) -> Option<Target> {
        let state = self.state.read(cx);
        let chat = state
            .chats
            .iter()
            .find(|chat| chat.id == self.current_key)?;
        if chat.delegation.is_some() || state.local_device_id.as_deref() != Some(&chat.device_id) {
            return None;
        }
        let space = state
            .space_for_chat(chat)
            .filter(|space| space.git_detected)?;
        let cwd = chat.cwd.clone().unwrap_or_else(|| space.path.clone());
        Some(Target {
            chat_id: chat.id.clone(),
            worktree: cwd != space.path,
            cwd,
            branch: chat.branch.clone(),
            harness: chat.config.as_ref().map(|config| config.harness),
        })
    }

    fn review_scope(&self, target: &Target, cx: &App) -> ReviewScope {
        self.state
            .read(cx)
            .reviews
            .get(&target.chat_id)
            .and_then(|review| review.scope)
            .unwrap_or_else(|| ReviewScope::default_for(target.worktree))
    }

    fn reviewer_child(
        &self,
        chat_id: &str,
        task_id: Option<&str>,
        cx: &App,
    ) -> Option<zeron_proto::Chat> {
        self.state
            .read(cx)
            .chats
            .iter()
            .filter(|chat| {
                chat.delegation
                    .as_ref()
                    .is_some_and(|d| d.delegated_by_chat_id == chat_id && review::is_review(d))
            })
            .filter(|chat| task_id.is_none_or(|id| chat.id == id))
            .max_by_key(|chat| chat.delegation.as_ref().map(|d| d.started_at))
            .cloned()
    }

    pub(super) fn sync_review(&mut self, cx: &mut Context<Self>) {
        if !self.hosts_review() {
            return;
        }
        let Some(target) = self.review_target(cx) else {
            return;
        };
        let current = self
            .state
            .read(cx)
            .reviews
            .get(&target.chat_id)
            .and_then(|review| review.run.as_ref())
            .map(|run| run.task_id.clone());
        let started_at = |chat: &zeron_proto::Chat| chat.delegation.as_ref().map(|d| d.started_at);
        let adoptable = |child: &zeron_proto::Chat| match &current {
            None => true,
            Some(None) => false,
            Some(Some(task)) => {
                task != &child.id
                    && self
                        .reviewer_child(&target.chat_id, Some(task), cx)
                        .is_none_or(|own| started_at(child) > started_at(&own))
            }
        };
        if let Some(child) = self.reviewer_child(&target.chat_id, None, cx)
            && adoptable(&child)
            && let Some((scope, commit)) = child
                .delegation
                .as_ref()
                .and_then(|d| d.client_request_id.as_deref())
                .and_then(review::parse_request)
        {
            let mut run = ReviewRun::new(scope);
            run.task_id = Some(child.id.clone());
            run.commit = commit.clone();
            run.status = RunStatus::Running;
            self.state.update(cx, |state, _| {
                let review = state.reviews.entry(target.chat_id.clone()).or_default();
                if current.is_some() {
                    review.scope = Some(scope);
                    if let Some(sha) = commit {
                        review.commit = Some(review::CommitPick {
                            subject: sha.clone(),
                            sha,
                        });
                    }
                }
                review.run = Some(run);
            });
        }
        self.read_finished_review(&target, cx);
        self.sync_review_outcomes(&target, cx);
        self.debug_open_findings(&target, cx);
        if self.review.options.is_none() && self.review.options_task.is_none() {
            self.load_reviewer_options(cx);
        }

        let scope = self.review_scope(&target, cx);
        let (run_scope, marker) = {
            let state = self.state.read(cx);
            let run_scope = state
                .reviews
                .get(&target.chat_id)
                .and_then(|review| review.run.as_ref())
                .filter(|run| run.status == RunStatus::Done)
                .map(|run| run.scope);
            let marker = state
                .session_for(&target.chat_id)
                .map(|session| format!("{:?}|{:?}", session.status, session.started_at))
                .unwrap_or_default();
            (run_scope, marker)
        };
        let commit = self
            .state
            .read(cx)
            .reviews
            .get(&target.chat_id)
            .and_then(|review| review.commit.as_ref().map(|pick| pick.sha.clone()))
            .unwrap_or_default();
        let key = format!(
            "{}|{}|{scope:?}|{run_scope:?}|{marker}|{commit}",
            target.chat_id, target.cwd
        );
        let current = self
            .state
            .read(cx)
            .reviews
            .get(&target.chat_id)
            .and_then(|review| review.stats_key.clone());
        if current.as_deref() != Some(key.as_str()) {
            let mut scopes = vec![scope];
            scopes.extend(run_scope.filter(|run_scope| *run_scope != scope));
            self.fetch_review_stats(target, scopes, Some(key), cx);
        }
    }

    fn fetch_review_stats(
        &mut self,
        target: Target,
        scopes: Vec<ReviewScope>,
        key: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        if let Some(key) = &key {
            self.state.update(cx, |state, _| {
                state
                    .reviews
                    .entry(target.chat_id.clone())
                    .or_default()
                    .stats_key = Some(key.clone());
            });
        }
        let commit = self
            .state
            .read(cx)
            .reviews
            .get(&target.chat_id)
            .and_then(|review| review.commit.as_ref().map(|pick| pick.sha.clone()));
        cx.spawn(async move |this, cx| {
            let mut fetched = Vec::new();
            for scope in scopes {
                fetched.push((
                    scope,
                    checkout_diff(&engine, &target, scope, commit.as_deref()).await,
                ));
            }
            this.update(cx, |composer, cx| {
                composer.state.update(cx, |state, cx| {
                    let review = state.reviews.entry(target.chat_id.clone()).or_default();
                    if key.is_some() && review.stats_key != key {
                        return;
                    }
                    for (scope, result) in fetched {
                        let (diff, base) = match result {
                            Ok(fetched) => fetched,
                            Err(message) => {
                                review.stats.insert(scope, Err(message));
                                continue;
                            }
                        };
                        review
                            .stats
                            .insert(scope, Ok((diff.additions, diff.deletions)));
                        if base.is_some() {
                            review.base = base;
                        }
                        if let Some(run) = review
                            .run
                            .as_mut()
                            .filter(|run| run.scope == scope && run.status == RunStatus::Done)
                        {
                            review::refresh(&mut run.findings, &parse_patch(&diff.patch));
                        }
                    }
                    cx.notify();
                });
            })
            .ok();
        })
        .detach();
    }

    fn read_finished_review(&mut self, target: &Target, cx: &mut Context<Self>) {
        let Some((task_id, scope, reviewed, commit)) = self
            .state
            .read(cx)
            .reviews
            .get(&target.chat_id)
            .and_then(|review| review.run.as_ref())
            .filter(|run| run.status == RunStatus::Running)
            .and_then(|run| {
                Some((
                    run.task_id.clone()?,
                    run.scope,
                    run.reviewed.clone(),
                    run.commit.clone(),
                ))
            })
        else {
            return;
        };
        let finished = self
            .reviewer_child(&target.chat_id, Some(&task_id), cx)
            .and_then(|child| child.delegation)
            .is_some_and(|d| !d.is_running());
        if !finished {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let chat_id = target.chat_id.clone();
        self.set_run(&chat_id, cx, |run| run.status = RunStatus::Reading);
        let target = target.clone();
        cx.spawn(async move |this, cx| {
            let view = engine
                .client()
                .call(
                    methods::TASK_STATUS,
                    serde_json::json!({ "chatId": chat_id, "taskId": task_id }),
                )
                .await;
            let restored = reviewed.is_none();
            let reviewed = match reviewed {
                Some(patch) => Some(patch),
                None => checkout_diff(&engine, &target, scope, commit.as_deref())
                    .await
                    .ok()
                    .map(|(diff, _)| diff.patch),
            };
            let outcome = match view {
                Err(err) => review::ReadOutcome::Failed(err.to_string()),
                Ok(view) => review::read_outcome(
                    view.get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default(),
                    view.get("summary")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default(),
                ),
            };
            this.update(cx, |composer, cx| {
                composer.set_run(&chat_id, cx, |run| {
                    if run.task_id.as_deref() != Some(task_id.as_str()) {
                        return;
                    }
                    match outcome {
                        review::ReadOutcome::Stopped => run.status = RunStatus::Stopped,
                        review::ReadOutcome::Failed(message) => {
                            run.status = RunStatus::Failed(message)
                        }
                        review::ReadOutcome::Read(None) => run.status = RunStatus::Unreadable,
                        review::ReadOutcome::Read(Some(mut findings)) => {
                            let files = parse_patch(reviewed.as_deref().unwrap_or_default());
                            if restored {
                                review::refresh(&mut findings, &files);
                            } else {
                                review::place_new(&mut findings, &files);
                            }
                            run.reported = findings.len();
                            run.findings = findings;
                            run.status = RunStatus::Done;
                        }
                    }
                });
            })
            .ok();
        })
        .detach();
    }

    fn debug_open_findings(&mut self, target: &Target, cx: &mut Context<Self>) {
        if self.review.debug_opened
            || std::env::var("ZERON_OPEN_DIALOG").as_deref() != Ok("review-findings")
        {
            return;
        }
        let listed = self
            .state
            .read(cx)
            .reviews
            .get(&target.chat_id)
            .and_then(|review| review.run.as_ref())
            .is_some_and(|run| run.status == RunStatus::Done && !run.findings.is_empty());
        if listed {
            self.review.debug_opened = true;
            self.open_review_findings(cx);
        }
    }

    fn sync_review_outcomes(&mut self, target: &Target, cx: &mut Context<Self>) {
        let Some(task_id) = self
            .state
            .read(cx)
            .reviews
            .get(&target.chat_id)
            .and_then(|review| review.run.as_ref())
            .and_then(|run| run.task_id.clone())
        else {
            return;
        };
        let delegation = self
            .reviewer_child(&target.chat_id, Some(&task_id), cx)
            .and_then(|child| child.delegation);
        let owner_working = self
            .state
            .read(cx)
            .session_for(&target.chat_id)
            .is_some_and(|session| session.status == zeron_proto::SessionStatus::Working);
        let unsent = self.state.update(cx, |state, cx| {
            let run = state
                .reviews
                .get_mut(&target.chat_id)
                .and_then(|review| review.run.as_mut())?;
            if run.delegation != delegation || run.owner_working != owner_working {
                run.delegation = delegation;
                run.owner_working = owner_working;
                cx.notify();
            }
            let withdrawals = run.unsent_withdrawals();
            if withdrawals.is_empty() {
                Some((run.unsent_hands(), false)).filter(|(ids, _)| !ids.is_empty())
            } else {
                Some((withdrawals, true))
            }
        });
        let Some((ids, withdraw)) = unsent.filter(|_| self.review.hand_task.is_none()) else {
            return;
        };
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let owner = target.chat_id.clone();
        self.review.hand_task = Some(cx.spawn(async move |this, cx| {
            let handed = engine
                .client()
                .call(
                    methods::HAND_REVIEW_FINDINGS,
                    serde_json::json!({ "ownerChatId": owner, "taskId": task_id, "ids": ids, "withdraw": withdraw }),
                )
                .await;
            this.update(cx, |composer, cx| {
                composer.review.hand_task = None;
                if let Err(err) = handed {
                    tracing::warn!(error = %err, "review findings not handed");
                    composer.set_run(&owner, cx, |run| {
                        if run.task_id.as_deref() == Some(task_id.as_str()) {
                            let sent = if withdraw {
                                &mut run.withdrawn
                            } else {
                                &mut run.handed_here
                            };
                            sent.retain(|id| !ids.contains(id));
                        }
                    });
                }
            })
            .ok();
        }));
    }

    fn set_run(&self, chat_id: &str, cx: &mut Context<Self>, f: impl FnOnce(&mut ReviewRun)) {
        self.state.update(cx, |state, cx| {
            if let Some(run) = state
                .reviews
                .get_mut(chat_id)
                .and_then(|review| review.run.as_mut())
            {
                f(run);
                cx.notify();
            }
        });
    }

    fn start_review(&mut self, scope: ReviewScope, cx: &mut Context<Self>) {
        let Some(target) = self.review_target(cx) else {
            return;
        };
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.close_review_menu(cx);
        let chat_id = target.chat_id.clone();
        let choice = self.reviewer_choice(&chat_id, cx);
        let (chosen, commit) = self
            .state
            .read(cx)
            .reviews
            .get(&chat_id)
            .map(|review| {
                (
                    review.reviewer.clone(),
                    review.commit.as_ref().map(|pick| pick.sha.clone()),
                )
            })
            .unwrap_or_default();
        let commit = commit.filter(|_| scope == ReviewScope::Commit);
        let mut run = ReviewRun::new(scope);
        run.choice = Some(choice.clone());
        run.commit = commit.clone();
        let previous = self.state.update(cx, |state, cx| {
            let previous = state
                .reviews
                .entry(chat_id.clone())
                .or_default()
                .run
                .replace(run);
            cx.notify();
            previous.filter(|run| run.status == RunStatus::Done && !run.findings.is_empty())
        });
        cx.spawn(async move |this, cx| {
            let started =
                launch_review(&engine, &target, scope, commit.as_deref(), chosen.as_ref()).await;
            this.update(cx, |composer, cx| {
                composer.set_run(&chat_id, cx, |run| {
                    if run.status != RunStatus::Starting {
                        return;
                    }
                    match started {
                        Ok((task_id, patch, note)) => {
                            run.task_id = Some(task_id);
                            run.reviewed = Some(patch);
                            run.note = note;
                            run.status = RunStatus::Running;
                        }
                        Err(message) => match previous {
                            Some(mut previous) => {
                                previous.retry_error = Some(message);
                                *run = previous;
                            }
                            None => run.status = RunStatus::Failed(message),
                        },
                    }
                });
            })
            .ok();
        })
        .detach();
    }

    fn reviewer_choice(&self, chat_id: &str, cx: &App) -> ReviewerChoice {
        self.state
            .read(cx)
            .reviews
            .get(chat_id)
            .and_then(|review| review.reviewer.clone())
            .or_else(|| {
                self.review
                    .settings
                    .as_ref()
                    .map(|settings| settings.reviewer.clone())
            })
            .unwrap_or_default()
    }

    fn set_reviewer(&mut self, choice: ReviewerChoice, cx: &mut Context<Self>) {
        self.store_reviewer(choice, cx);
        self.close_review_menu(cx);
    }

    fn store_reviewer(&mut self, choice: ReviewerChoice, cx: &mut Context<Self>) {
        let key = self.current_key.clone();
        self.state.update(cx, |state, cx| {
            state.reviews.entry(key).or_default().reviewer = Some(choice);
            cx.notify();
        });
    }

    fn close_review_menu(&mut self, cx: &mut Context<Self>) {
        self.review.submenu_intent.reset();
        self.review.submenu = None;
        self.review.submenu_bounds = None;
        if self.review.menu.begin_close() {
            popover::reap_popup(cx, |composer: &mut Self| &mut composer.review.menu);
            cx.notify();
        }
    }

    pub(crate) fn open_review_menu(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.review_target(cx) else {
            return;
        };
        self.review.menu.open(());
        self.fetch_review_stats(target, ReviewScope::ALL.to_vec(), None, cx);
        self.load_reviewer_options(cx);
        cx.notify();
    }

    fn load_reviewer_options(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.review.options_task = Some(cx.spawn(async move |this, cx| {
            let options = review::reviewer_options(&engine).await;
            let settings = review::review_settings(&engine).await;
            this.update(cx, |composer, cx| {
                composer.review.options = Some(options);
                composer.review.settings = Some(settings);
                cx.notify();
            })
            .ok();
        }));
    }

    fn set_review_scope(&mut self, scope: ReviewScope, cx: &mut Context<Self>) {
        let Some(target) = self.review_target(cx) else {
            return;
        };
        self.state.update(cx, |state, cx| {
            let review = state.reviews.entry(target.chat_id).or_default();
            review.scope = Some(scope);
            cx.notify();
        });
        self.close_review_menu(cx);
    }

    fn review_look(&self, target: &Target, scope: ReviewScope, cx: &App) -> Option<Look> {
        chip_look(self.state.read(cx).reviews.get(&target.chat_id)?, scope)
    }

    pub(crate) fn render_review_chip(
        &mut self,
        theme: &Theme,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let chip = self.review_chip(theme, window, cx);
        if chip.is_none() {
            self.drop_review_menu();
        }
        chip
    }

    fn review_chip(
        &mut self,
        theme: &Theme,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.hosts_review() {
            return None;
        }
        let target = self.review_target(cx)?;
        let scope = self.review_scope(&target, cx);
        let look = self.review_look(&target, scope, cx)?;
        let picked = self
            .state
            .read(cx)
            .reviews
            .get(&target.chat_id)
            .and_then(|review| review.commit.as_ref().map(|pick| pick.short().to_string()));
        let counts = |add: u32, del: u32| {
            div()
                .flex()
                .gap(px(4.0))
                .text_size(crate::typography::ui_rems(11.0))
                .child(
                    div()
                        .text_color(crate::changes::add_color(theme))
                        .child(format!("+{add}")),
                )
                .child(
                    div()
                        .text_color(crate::changes::del_color(theme))
                        .child(format!("−{del}")),
                )
        };
        let review_label: SharedString = match (scope, &picked) {
            (ReviewScope::Commit, Some(short)) => format!("Review {short}").into(),
            _ => "Review".into(),
        };
        let (icon, label, color, border, fill, tip): (&str, SharedString, _, _, _, SharedString) =
            match &look {
                Look::Idle(..) => (
                    crate::icons::CHECKLIST,
                    review_label,
                    theme.text,
                    theme.border,
                    None,
                    format!(
                        "Review {} with a fresh agent",
                        scope.label(None).to_lowercase()
                    )
                    .into(),
                ),
                Look::Empty(_) => (
                    crate::icons::CHECKLIST,
                    review_label,
                    theme.text_muted,
                    theme.border,
                    None,
                    format!(
                        "Nothing to review in {}. Pick another scope from the menu.",
                        scope.label(None).to_lowercase()
                    )
                    .into(),
                ),
                Look::Busy => (
                    crate::icons::CHECKLIST,
                    "Reviewing".into(),
                    theme.text_muted,
                    theme.border,
                    None,
                    "The reviewer is working. Its progress is in the agents pill.".into(),
                ),
                Look::Findings(n) => (
                    crate::icons::CHECKLIST,
                    format!("{n} finding{}", if *n == 1 { "" } else { "s" }).into(),
                    theme.danger,
                    theme.danger.opacity(0.45),
                    Some(theme.danger.opacity(0.10)),
                    "Open the findings".into(),
                ),
                Look::Again(..) => (
                    crate::icons::REFRESH,
                    "Review again".into(),
                    theme.text_muted,
                    theme.border,
                    None,
                    "The code changed since the review. Some findings are outdated.".into(),
                ),
                Look::Stopped(..) => (
                    crate::icons::REFRESH,
                    "Review again".into(),
                    theme.text_muted,
                    theme.border,
                    None,
                    "The review was stopped. Click to run it again.".into(),
                ),
                Look::Clean => (
                    crate::icons::LUCIDE_CIRCLE_CHECK,
                    "No issues".into(),
                    theme.success,
                    theme.success.opacity(0.40),
                    None,
                    "The reviewer found no problems. Click to review again.".into(),
                ),
                Look::Unreadable => (
                    crate::icons::CHECKLIST,
                    "Couldn’t read findings".into(),
                    theme.text_muted,
                    theme.border,
                    None,
                    "The reviewer did not end with a findings block. Click to review again; its thread is in the agents pill."
                        .into(),
                ),
                Look::Failed(message) => (
                    crate::icons::CHECKLIST,
                    "Review failed".into(),
                    theme.danger,
                    theme.danger.opacity(0.45),
                    None,
                    message.clone().into(),
                ),
            };
        let busy = matches!(look, Look::Busy);
        let stoppable = busy
            && self
                .state
                .read(cx)
                .reviews
                .get(&target.chat_id)
                .is_some_and(|review| {
                    review.run.as_ref().is_some_and(|run| {
                        run.status == RunStatus::Running && run.task_id.is_some()
                    })
                });
        let stopping = stoppable && self.review.busy_hover;
        let (icon, label, color) = if stopping {
            (
                crate::icons::STOP,
                SharedString::from("Stop review"),
                theme.text,
            )
        } else {
            (icon, label, color)
        };
        let next_reviewer = (!busy && !matches!(look, Look::Findings(_)))
            .then(|| self.next_reviewer(&target, cx))
            .flatten();
        let inert = matches!(look, Look::Empty(_));
        let trailing_counts = match &look {
            Look::Idle(add, del) | Look::Again(add, del) | Look::Stopped(add, del) => {
                Some(counts(*add, *del))
            }
            Look::Empty(note) => Some(
                div()
                    .text_size(crate::typography::ui_rems(11.0))
                    .text_color(theme.text_faint)
                    .child(note.clone()),
            ),
            _ => None,
        };
        let main = div()
            .id("composer-review")
            .debug_selector(|| "composer-review".into())
            .role(gpui::Role::Button)
            .aria_label(label.clone())
            .h_full()
            .pl(px(9.0))
            .pr(px(7.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .when((!busy && !inert) || stoppable, |el| {
                el.cursor_pointer().hover(|s| s.bg(crate::theme::ink(0.06)))
            })
            .when(busy, |el| {
                el.on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                    if this.review.busy_hover != *hovered {
                        this.review.busy_hover = *hovered;
                        cx.notify();
                    }
                }))
            })
            .tooltip(crate::settings::widgets::text_tooltip(if stoppable {
                SharedString::from("Stop the reviewer. The chat keeps running.")
            } else {
                tip
            }))
            .child(crate::icons::icon(icon).size(px(14.0)).text_color(color))
            .child(label)
            .children(next_reviewer.map(|(name, mark, tint)| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .text_color(theme.text_muted)
                    .child("·")
                    .child(
                        crate::icons::icon(mark)
                            .size(px(12.0))
                            .text_color(tint.unwrap_or(theme.text_muted)),
                    )
                    .child(name)
            }))
            .children(trailing_counts)
            .when(!busy && !inert, |el| {
                el.on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.on_review_click(scope, cx);
                }))
            })
            .when(stoppable, |el| {
                el.on_click(cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    this.stop_review(cx);
                }))
            });
        let caret = div()
            .id("composer-review-menu")
            .debug_selector(|| "composer-review-menu".into())
            .role(gpui::Role::Button)
            .aria_label("Review options")
            .aria_expanded(self.review.menu.is_open())
            .h_full()
            .px(px(5.0))
            .flex()
            .items_center()
            .border_l_1()
            .border_color(border)
            .cursor_pointer()
            .hover(|s| s.bg(crate::theme::ink(0.06)))
            .child(
                crate::icons::icon(crate::icons::ALT_ARROW_DOWN)
                    .size(px(12.0))
                    .text_color(theme.text_muted),
            )
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, _| this.review.menu.note_trigger_press()),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                if this.review.menu.take_press_was_open() {
                    this.close_review_menu(cx);
                } else {
                    this.open_review_menu(cx);
                }
            }));
        let chip = div()
            .flex_none()
            .h(px(CHIP_HEIGHT))
            .flex()
            .flex_row()
            .items_stretch()
            .rounded_full()
            .overflow_hidden()
            .border_1()
            .border_color(border)
            .when_some(fill, |el, fill| el.bg(fill))
            .when(matches!(look, Look::Again(..) | Look::Stopped(..)), |el| {
                el.border_dashed()
            })
            .when(busy && !stopping, |el| el.opacity(0.55))
            .text_size(crate::typography::ui_rems(12.0))
            .text_color(color)
            .child(main)
            .child(caret);
        let menu = self
            .review
            .menu
            .get()
            .is_some()
            .then(|| self.render_review_menu(&target, scope, theme, window, cx));
        Some(
            div()
                .relative()
                .flex_none()
                .child(chip)
                .children(menu)
                .into_any_element(),
        )
    }

    fn on_review_click(&mut self, scope: ReviewScope, cx: &mut Context<Self>) {
        let Some(target) = self.review_target(cx) else {
            return;
        };
        match self.review_look(&target, scope, cx) {
            Some(Look::Findings(_)) => self.open_review_findings(cx),
            Some(Look::Busy | Look::Empty(_)) | None => {}
            Some(_) => self.start_review(scope, cx),
        }
    }

    fn stop_review(&mut self, cx: &mut Context<Self>) {
        let task = self
            .state
            .read(cx)
            .reviews
            .get(&self.current_key)
            .and_then(|review| review.run.as_ref())
            .filter(|run| run.status == RunStatus::Running)
            .and_then(|run| run.task_id.clone());
        if let Some(task) = task {
            self.review.busy_hover = false;
            self.stop_agent_run(task, true, cx);
        }
    }

    fn next_reviewer(
        &self,
        target: &Target,
        cx: &App,
    ) -> Option<(String, &'static str, Option<gpui::Hsla>)> {
        let last = self
            .state
            .read(cx)
            .reviews
            .get(&target.chat_id)?
            .run
            .as_ref()?
            .choice
            .clone()?;
        let next = self.reviewer_choice(&target.chat_id, cx);
        if next == last {
            return None;
        }
        let options = self.review.options.clone().unwrap_or_default();
        let name = match next {
            ReviewerChoice::Auto if self.review.options.is_some() => {
                review::resolve_reviewer(&next, target.harness, &options).name()
            }
            _ => options.label(&next),
        };
        let (mark, tint) = review::brand(&options, &next, target.harness);
        Some((name, mark, tint))
    }

    fn open_review_findings(&mut self, cx: &mut Context<Self>) {
        self.open_roster(cx);
    }

    fn render_review_menu(
        &mut self,
        target: &Target,
        selected: ReviewScope,
        theme: &Theme,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let popup = theme.for_popup();
        let review = self
            .state
            .read(cx)
            .reviews
            .get(&target.chat_id)
            .cloned()
            .unwrap_or_default();
        let busy = review.run.as_ref().is_some_and(ReviewRun::busy);
        let on_base = target.branch.is_some() && target.branch == review.base;
        let rows = ReviewScope::ALL
            .into_iter()
            .filter(|scope| !(on_base && *scope == ReviewScope::Branch))
            .map(|scope| {
                let counts = review.changes(scope);
                let unavailable = review.unavailable(scope).map(capitalize);
                let empty =
                    counts.is_some_and(|(add, del)| add + del == 0) || unavailable.is_some();
                let id = SharedString::from(format!("composer-review-scope-{}", scope.key()));
                let selector = id.clone();
                popover::menu_row(&popup, false, id.clone())
                    .id(id)
                    .debug_selector(move || selector.to_string())
                    .role(gpui::Role::MenuItemRadio)
                    .aria_label(scope.label(review.base.as_deref()))
                    .gap(px(8.0))
                    .child(crate::settings::widgets::select_check(
                        &popup,
                        scope == selected,
                    ))
                    .child(div().flex_1().child(scope.label(review.base.as_deref())))
                    .child(match counts {
                        Some((add, del)) => div()
                            .flex()
                            .gap(px(4.0))
                            .text_size(crate::typography::ui_rems(11.0))
                            .child(
                                div()
                                    .text_color(crate::changes::add_color(&popup))
                                    .child(format!("+{add}")),
                            )
                            .child(
                                div()
                                    .text_color(crate::changes::del_color(&popup))
                                    .child(format!("−{del}")),
                            )
                            .into_any_element(),
                        None => div()
                            .text_size(crate::typography::ui_rems(11.0))
                            .text_color(popup.text_faint)
                            .child(unavailable.clone().unwrap_or_else(|| "…".to_string()))
                            .into_any_element(),
                    })
                    .when(empty, |el| el.opacity(0.5))
                    .when(unavailable.is_none(), |el| {
                        el.on_click(
                            cx.listener(move |this, _, _, cx| this.set_review_scope(scope, cx)),
                        )
                    })
            });
        let mut rows: Vec<AnyElement> = rows.map(IntoElement::into_any_element).collect();
        rows.push(self.render_commit_row(&review, selected, &popup, window, cx));
        let effective = self.reviewer_choice(&target.chat_id, cx);
        let options = self.review.options.clone().unwrap_or_default();
        let auto_name = self.review.options.as_ref().map(|options| {
            review::resolve_reviewer(&ReviewerChoice::Auto, target.harness, options).name()
        });
        let same_slot = |a: &ReviewerChoice, b: &ReviewerChoice| match (a, b) {
            (
                ReviewerChoice::Harness { harness: a, .. },
                ReviewerChoice::Harness { harness: b, .. },
            ) => a == b,
            _ => a == b,
        };
        let mut choices = options.choices();
        if !choices.iter().any(|choice| same_slot(choice, &effective)) {
            choices.insert(1, effective.clone());
        }
        let open_models = self.review.submenu;
        let reviewer_rows: Vec<AnyElement> = choices
            .into_iter()
            .enumerate()
            .map(|(ix, choice)| {
                let harness = match &choice {
                    ReviewerChoice::Harness { harness, .. } => Some(*harness),
                    _ => None,
                };
                let selected = same_slot(&choice, &effective);
                let label = match harness {
                    Some(harness) => options.label(&ReviewerChoice::Harness {
                        harness,
                        model: None,
                        reasoning: None,
                    }),
                    None => options.label(&choice),
                };
                let detail = match (&choice, &effective) {
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
                        let parts: Vec<String> =
                            model
                                .as_deref()
                                .map(|model| self.model_label(*h, model))
                                .into_iter()
                                .chain(reasoning.map(|level| {
                                    crate::pickers::reasoning_label(level).to_string()
                                }))
                                .collect();
                        Some(parts.join(" · "))
                    }
                    _ => None,
                };
                let (mark, tint) = review::brand(&options, &choice, target.harness);
                let id = SharedString::from(format!("composer-reviewer-{ix}"));
                let open = harness.is_some() && open_models.map(|(key, _)| key) == Some(ix);
                let mut row = popover::menu_row(&popup, open, id.clone())
                    .id(id)
                    .relative()
                    .role(gpui::Role::MenuItemRadio)
                    .aria_label(format!("Reviewer {}", options.label(&choice)))
                    .gap(px(8.0))
                    .child(crate::settings::widgets::select_check(&popup, selected))
                    .child(
                        crate::icons::icon(mark)
                            .size(px(14.0))
                            .flex_none()
                            .text_color(tint.unwrap_or(popup.text_muted)),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(label))
                    .when_some(detail, |el, detail| {
                        el.child(
                            div()
                                .flex_none()
                                .text_size(crate::typography::ui_rems(11.0))
                                .text_color(popup.text_faint)
                                .child(detail),
                        )
                    })
                    .when(harness.is_some(), |el| {
                        el.child(
                            crate::icons::icon(crate::icons::ALT_ARROW_RIGHT)
                                .size(px(12.0))
                                .flex_none()
                                .text_color(popup.text_muted),
                        )
                    })
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.set_reviewer(choice.clone(), cx)),
                    );
                if let Some(harness) = harness {
                    row = row.child(self.submenu_probe(open, cx));
                    if open {
                        row = row.child(popover::nested_menu(
                            format!("composer-reviewer-models-{ix}"),
                            self.render_model_menu(harness, &effective, theme, window, cx),
                            self.review.submenu_left,
                        ));
                    }
                }
                div()
                    .id(("composer-reviewer-hover", ix))
                    .on_hover(cx.listener(move |this, hovered: &bool, window, cx| {
                        if *hovered {
                            this.hover_menu_row(
                                ix,
                                harness.map(Submenu::Models),
                                window.mouse_position(),
                                false,
                                cx,
                            );
                        } else {
                            this.review.submenu_intent.leave(&ix);
                        }
                    }))
                    .on_mouse_move(
                        cx.listener(move |this, event: &gpui::MouseMoveEvent, _, cx| {
                            this.hover_menu_row(
                                ix,
                                harness.map(Submenu::Models),
                                event.position,
                                true,
                                cx,
                            );
                        }),
                    )
                    .child(row)
                    .into_any_element()
            })
            .collect();
        let review_label = match (selected, &review.commit) {
            (ReviewScope::Commit, Some(pick)) => format!("Review commit {}", pick.short()),
            _ => format!(
                "Review {}",
                selected.label(review.base.as_deref()).to_lowercase()
            ),
        };
        let review_now = popover::menu_row(&popup, false, "composer-review-now")
            .id("composer-review-now")
            .debug_selector(|| "composer-review-now".into())
            .role(gpui::Role::MenuItem)
            .aria_label(review_label.clone())
            .gap(px(8.0))
            .child(
                crate::icons::icon(crate::icons::CHECKLIST)
                    .size(px(14.0))
                    .text_color(popup.text_muted),
            )
            .child(review_label)
            .when(busy, |el| el.opacity(0.5).cursor_default())
            .when(!busy, |el| {
                el.on_click(cx.listener(move |this, _, _, cx| this.start_review(selected, cx)))
            });
        let menu = popover::popover_card(&popup)
            .id("composer-review-popup-card")
            .w(px(270.0))
            .p(px(4.0))
            .flex()
            .flex_col()
            .gap(px(popover::MENU_GAP))
            .on_mouse_down_out(cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                // The model submenu floats outside this card: a press on it is inside the menu.
                if this
                    .review
                    .submenu_bounds
                    .is_some_and(|bounds| bounds.contains(&event.position))
                {
                    return;
                }
                this.close_review_menu(cx);
            }))
            .child(popover::menu_heading(&popup, "Scope"))
            .children(rows)
            .child(popover::menu_separator())
            .child(popover::menu_heading(&popup, "Reviewer"))
            .children(reviewer_rows)
            .child(popover::menu_separator())
            .child(review_now)
            .into_any_element();
        popover::anchored_menu_below_end(
            "composer-review-popup",
            menu,
            self.review.menu.closing_since(),
        )
    }

    fn model_label(&self, harness: HarnessId, model: &str) -> String {
        self.review
            .models
            .get(&harness)
            .and_then(|models| models.iter().find(|m| m.id == model))
            .map_or_else(|| model.to_string(), |m| m.label.clone())
    }

    fn set_submenu(&mut self, open: Option<(usize, Submenu)>, cx: &mut Context<Self>) {
        if self.review.submenu != open {
            self.review.submenu = open;
            self.review.submenu_bounds = None;
            cx.notify();
        }
        let harness = match open {
            Some((_, Submenu::Models(harness))) => harness,
            Some((_, Submenu::Commits)) => return self.load_commits(cx),
            None => return,
        };
        if self.review.models.contains_key(&harness) {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.review.models_task = Some(cx.spawn(async move |this, cx| {
            let models = review::list_models(&engine, harness).await;
            this.update(cx, |composer, cx| {
                composer.review.models.insert(harness, models);
                cx.notify();
            })
            .ok();
        }));
    }

    fn hover_menu_row(
        &mut self,
        key: usize,
        target: Option<Submenu>,
        pointer: Point<Pixels>,
        moved: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.review.menu.is_open() {
            return;
        }
        let current = self.review.submenu.map(|(open, _)| open);
        let (bounds, left) = (self.review.submenu_bounds, self.review.submenu_left);
        let action = if moved {
            self.review
                .submenu_intent
                .moved(current.as_ref(), &key, pointer, bounds, left)
        } else {
            self.review
                .submenu_intent
                .enter(current.as_ref(), &key, pointer, bounds, left)
        };
        let target = target.map(|submenu| (key, submenu));
        match action {
            popover::HoverAction::None => {}
            popover::HoverAction::Open => {
                self.set_submenu(target, cx);
                self.review.submenu_intent.record_origin(pointer);
            }
            popover::HoverAction::Defer => {
                self.review.submenu_intent.defer(cx, move |this, cx| {
                    if this.review.menu.is_open()
                        && this.review.submenu_intent.pending() == Some(&key)
                    {
                        this.review.submenu_intent.cancel();
                        this.set_submenu(target, cx);
                        this.review.submenu_intent.record_origin(pointer);
                    }
                });
            }
        }
    }

    fn submenu_probe(&self, open: bool, cx: &mut Context<Self>) -> AnyElement {
        let entity = cx.entity().downgrade();
        let exit = entity.clone();
        gpui::canvas(
            move |bounds, window, cx| {
                let left = bounds.right() + px(220.0) > window.viewport_size().width;
                let _ = entity.update(cx, |this, cx| {
                    if this.review.submenu_left != left {
                        this.review.submenu_left = left;
                        cx.notify();
                    }
                });
            },
            move |trigger, _, window, _| {
                if !open {
                    return;
                }
                window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, _, cx| {
                    if phase != gpui::DispatchPhase::Bubble {
                        return;
                    }
                    let _ = exit.update(cx, |this, cx| {
                        let (bounds, left) = (this.review.submenu_bounds, this.review.submenu_left);
                        if this.review.submenu_intent.contains_pointer(
                            trigger,
                            bounds,
                            event.position,
                            left,
                        ) {
                            return;
                        }
                        this.review.submenu_intent.reset();
                        this.set_submenu(None, cx);
                    });
                });
            },
        )
        .absolute()
        .inset_0()
        .into_any_element()
    }

    fn render_model_menu(
        &self,
        harness: HarnessId,
        effective: &ReviewerChoice,
        theme: &Theme,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let popup = theme.for_popup();
        let (current, reasoning) = match effective {
            ReviewerChoice::Harness {
                harness: h,
                model,
                reasoning,
            } if *h == harness => (Some(model.clone()), *reasoning),
            _ => (None, None),
        };
        let row = |ix: usize, label: String, model: Option<String>| {
            let id = SharedString::from(format!("composer-reviewer-model-{ix}"));
            let checked = current.as_ref() == Some(&model);
            popover::menu_row(&popup, false, id.clone())
                .id(id)
                .role(gpui::Role::MenuItemRadio)
                .aria_label(format!("Model {label}"))
                .gap(px(8.0))
                .child(crate::settings::widgets::select_check(&popup, checked))
                .child(div().flex_1().min_w_0().truncate().child(label))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_reviewer(
                        ReviewerChoice::Harness {
                            harness,
                            model: model.clone(),
                            reasoning,
                        },
                        cx,
                    )
                }))
                .into_any_element()
        };
        let models = self.review.models.get(&harness);
        let mut rows = vec![row(0, "Default".to_string(), None)];
        match models {
            Some(models) => rows.extend(
                models
                    .iter()
                    .enumerate()
                    .map(|(ix, m)| row(ix + 1, m.label.clone(), Some(m.id.clone()))),
            ),
            None => rows.push(
                div()
                    .px(px(8.0))
                    .py(px(6.0))
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(popup.text_faint)
                    .child("Loading…")
                    .into_any_element(),
            ),
        }
        let thinking = self.render_thinking(
            harness,
            current.clone().flatten(),
            reasoning,
            &popup,
            window,
            cx,
        );
        let entity = cx.entity().downgrade();
        popover::popover_card(&popup)
            .id("composer-reviewer-models-card")
            .w(px(210.0))
            .relative()
            .p(px(4.0))
            .flex()
            .flex_col()
            .gap(px(popover::MENU_GAP))
            .child(
                div()
                    .id("composer-reviewer-models-list")
                    .max_h(px(300.0))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(popover::MENU_GAP))
                    .children(rows),
            )
            .children(thinking)
            .child(
                gpui::canvas(
                    move |bounds, _, cx| {
                        let _ =
                            entity.update(cx, |this, _| this.review.submenu_bounds = Some(bounds));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .into_any_element()
    }

    fn render_thinking(
        &self,
        harness: HarnessId,
        model: Option<String>,
        reasoning: Option<zeron_proto::ReasoningLevel>,
        popup: &Theme,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let options = self.review.options.as_ref()?;
        let model_entry = model.as_deref().and_then(|id| {
            self.review
                .models
                .get(&harness)
                .and_then(|models| models.iter().find(|m| m.id == id))
        });
        let levels = review::reasoning_stops(options, harness, model_entry);
        if levels.is_empty() {
            return None;
        }
        let labels: Vec<SharedString> = std::iter::once(SharedString::from("Default"))
            .chain(
                levels
                    .iter()
                    .map(|level| SharedString::from(crate::pickers::reasoning_label(*level))),
            )
            .collect();
        let selected = reasoning
            .and_then(|level| levels.iter().position(|l| *l == level))
            .map_or(0, |ix| ix + 1);
        let value = labels[selected].clone();
        let slider = self.review.thinking.render(
            "composer-reviewer-thinking",
            "Reviewer thinking",
            &labels,
            selected,
            |composer: &mut Self| &mut composer.review.thinking,
            move |this, ix, cx| {
                this.store_reviewer(
                    ReviewerChoice::Harness {
                        harness,
                        model: model.clone(),
                        reasoning: ix.checked_sub(1).and_then(|ix| levels.get(ix).copied()),
                    },
                    cx,
                );
            },
            popup,
            window,
            cx,
        );
        Some(
            div()
                .flex()
                .flex_col()
                .child(popover::menu_separator())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .child(popover::menu_heading(popup, "Thinking")),
                        )
                        .child(
                            div()
                                .pr(px(8.0))
                                .pt(px(2.0))
                                .text_size(crate::typography::ui_rems(11.0))
                                .text_color(popup.text_muted)
                                .child(value),
                        ),
                )
                .child(div().px(px(8.0)).pt(px(4.0)).pb(px(8.0)).child(slider))
                .into_any_element(),
        )
    }

    fn render_commit_row(
        &self,
        review: &review::ChatReview,
        selected: ReviewScope,
        popup: &Theme,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = matches!(self.review.submenu, Some((_, Submenu::Commits)));
        let picked = review.commit.clone();
        let has_pick = picked.is_some();
        let mut row = popover::menu_row(popup, open, "composer-review-scope-commit")
            .id("composer-review-scope-commit")
            .relative()
            .role(gpui::Role::MenuItemRadio)
            .aria_label("Commit")
            .gap(px(8.0))
            .child(crate::settings::widgets::select_check(
                popup,
                selected == ReviewScope::Commit,
            ))
            .child(div().flex_1().child("Commit"))
            .children(picked.map(|pick| {
                div()
                    .font_family(popup.font_mono.clone())
                    .text_size(crate::typography::ui_rems(11.0))
                    .text_color(popup.text_faint)
                    .child(pick.short().to_string())
            }))
            .child(
                crate::icons::icon(crate::icons::ALT_ARROW_RIGHT)
                    .size(px(12.0))
                    .flex_none()
                    .text_color(popup.text_muted),
            )
            .child(self.submenu_probe(open, cx))
            .on_click(cx.listener(move |this, _, _, cx| {
                if has_pick {
                    this.set_review_scope(ReviewScope::Commit, cx);
                } else {
                    this.set_submenu(Some((COMMITS_ROW, Submenu::Commits)), cx);
                }
            }));
        if open {
            row = row.child(popover::nested_menu(
                "composer-review-commits",
                self.render_commit_menu(review, popup, window, cx),
                self.review.submenu_left,
            ));
        }
        div()
            .id("composer-review-commit-hover")
            .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
                if *hovered {
                    this.hover_menu_row(
                        COMMITS_ROW,
                        Some(Submenu::Commits),
                        window.mouse_position(),
                        false,
                        cx,
                    );
                } else {
                    this.review.submenu_intent.leave(&COMMITS_ROW);
                }
            }))
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                this.hover_menu_row(
                    COMMITS_ROW,
                    Some(Submenu::Commits),
                    event.position,
                    true,
                    cx,
                );
            }))
            .child(row)
            .into_any_element()
    }

    fn load_commits(&mut self, cx: &mut Context<Self>) {
        if self.review.commit_filter.is_none() {
            let input =
                cx.new(|cx| super::ComposerInput::new("Filter commits…", cx).with_single_line());
            cx.subscribe(&input, |_, _, _: &super::ComposerInputEvent, cx| {
                cx.notify()
            })
            .detach();
            self.review.commit_filter = Some(input);
        }
        let Some(target) = self.review_target(cx) else {
            return;
        };
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.review.commits_task = Some(cx.spawn(async move |this, cx| {
            let page = engine
                .client()
                .call_as::<zeron_proto::GitHistoryPage>(
                    methods::LIST_GIT_HISTORY,
                    serde_json::json!({
                        "cwd": target.cwd,
                        "cursor": 0,
                        "limit": COMMIT_LIST_LIMIT,
                    }),
                )
                .await;
            this.update(cx, |composer, cx| {
                composer.review.commits = Some(page.map(|page| page.commits).unwrap_or_default());
                cx.notify();
            })
            .ok();
        }));
    }

    fn pick_commit(&mut self, pick: review::CommitPick, cx: &mut Context<Self>) {
        let Some(target) = self.review_target(cx) else {
            return;
        };
        self.state.update(cx, |state, cx| {
            let review = state.reviews.entry(target.chat_id.clone()).or_default();
            review.commit = Some(pick);
            review.scope = Some(ReviewScope::Commit);
            review.stats.remove(&ReviewScope::Commit);
            cx.notify();
        });
        self.review.submenu_intent.reset();
        self.set_submenu(None, cx);
        self.fetch_review_stats(target, vec![ReviewScope::Commit], None, cx);
    }

    fn render_commit_menu(
        &self,
        review: &review::ChatReview,
        popup: &Theme,
        _window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let query = self
            .review
            .commit_filter
            .as_ref()
            .map(|input| input.read(cx).text().trim().to_lowercase())
            .unwrap_or_default();
        let current = review.commit.as_ref().map(|pick| pick.sha.clone());
        let now = chrono::Utc::now();
        let rows: Vec<AnyElement> = match &self.review.commits {
            None => vec![
                div()
                    .px(px(8.0))
                    .py(px(6.0))
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(popup.text_faint)
                    .child("Loading…")
                    .into_any_element(),
            ],
            Some(commits) => commits
                .iter()
                .filter(|commit| {
                    query.is_empty()
                        || commit.subject.to_lowercase().contains(&query)
                        || commit.sha.starts_with(&query)
                })
                .enumerate()
                .map(|(ix, commit)| {
                    let pick = review::CommitPick {
                        sha: commit.sha.clone(),
                        subject: commit.subject.clone(),
                    };
                    let age = chrono::DateTime::parse_from_rfc3339(&commit.authored_at)
                        .map(|at| {
                            crate::pull_requests::relative_time(at.with_timezone(&chrono::Utc), now)
                        })
                        .unwrap_or_default();
                    let id = SharedString::from(format!("composer-review-commit-{ix}"));
                    popover::menu_row(popup, false, id.clone())
                        .id(id)
                        .role(gpui::Role::MenuItemRadio)
                        .aria_label(format!("Commit {}", commit.subject))
                        .gap(px(8.0))
                        .child(crate::settings::widgets::select_check(
                            popup,
                            current.as_deref() == Some(commit.sha.as_str()),
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(commit.subject.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .font_family(popup.font_mono.clone())
                                .text_size(crate::typography::ui_rems(10.5))
                                .text_color(popup.text_faint)
                                .child(format!("{} · {age}", pick.short())),
                        )
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.pick_commit(pick.clone(), cx)),
                        )
                        .into_any_element()
                })
                .collect(),
        };
        let empty = rows.is_empty();
        let entity = cx.entity().downgrade();
        popover::popover_card(popup)
            .id("composer-review-commits-card")
            .w(px(360.0))
            .relative()
            .p(px(4.0))
            .flex()
            .flex_col()
            .gap(px(popover::MENU_GAP))
            .children(self.review.commit_filter.clone().map(|input| {
                div()
                    .mx(px(4.0))
                    .mt(px(2.0))
                    .mb(px(4.0))
                    .px(px(8.0))
                    .py(px(5.0))
                    .rounded(px(7.0))
                    .border_1()
                    .border_color(popup.border)
                    .text_size(crate::typography::ui_rems(12.0))
                    .child(input)
            }))
            .child(
                div()
                    .id("composer-review-commits-list")
                    .max_h(px(320.0))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(popover::MENU_GAP))
                    .children(rows)
                    .when(empty, |el| {
                        el.child(
                            div()
                                .px(px(8.0))
                                .py(px(6.0))
                                .text_size(crate::typography::ui_rems(12.0))
                                .text_color(popup.text_faint)
                                .child("No matching commits"),
                        )
                    }),
            )
            .child(
                gpui::canvas(
                    move |bounds, _, cx| {
                        let _ =
                            entity.update(cx, |this, _| this.review.submenu_bounds = Some(bounds));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .into_any_element()
    }

    pub(crate) fn drop_review_menu(&mut self) {
        self.review.menu = Default::default();
        self.review.submenu_intent.reset();
        self.review.submenu = None;
        self.review.submenu_bounds = None;
    }

    pub(crate) fn review_menu_open(&self) -> bool {
        self.review.menu.is_open()
    }

    fn review_run_for(&self, doc_id: &str, cx: &App) -> Option<ReviewRun> {
        if !self.hosts_review() {
            return None;
        }
        self.state
            .read(cx)
            .reviews
            .get(&self.current_key)?
            .run
            .clone()
            .filter(|run| run.task_id.as_deref() == Some(doc_id))
    }

    pub(super) fn review_pill_suffix(&self, doc_id: &str, cx: &App) -> Option<String> {
        let run = self.review_run_for(doc_id, cx)?;
        let suffix = match run.status {
            RunStatus::Unreadable => "couldn’t read findings".to_string(),
            RunStatus::Stopped => "stopped".to_string(),
            RunStatus::Failed(_) => "failed".to_string(),
            RunStatus::Done if run.reported == 0 => "no issues".to_string(),
            RunStatus::Done if run.findings.is_empty() => format!("{} handled", run.reported),
            RunStatus::Done => {
                let mut text = outcome_counts(&run);
                if run.outdated() > 0 {
                    text.push_str(&format!(" · {} outdated", run.outdated()));
                }
                text
            }
            _ if run.note.is_some() => "working".to_string(),
            _ => return None,
        };
        Some(match run.note {
            Some(_) => format!("{suffix} · fallback"),
            None => suffix,
        })
    }

    pub(super) fn render_review_findings(
        &self,
        doc_id: &str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let run = self.review_run_for(doc_id, cx)?;
        let listed = run.status == RunStatus::Done && !run.findings.is_empty();
        if !listed && run.note.is_none() {
            return None;
        }
        let note = run
            .note
            .clone()
            .map(|note| div().pb(px(4.0)).text_color(theme.text_faint).child(note));
        let retry_error = run.retry_error.clone().map(|message| {
            div()
                .pb(px(4.0))
                .text_color(theme.danger)
                .child(format!("Review again failed: {message}"))
        });
        let rows = run.findings.iter().map(|finding| {
            let (tint, wash) = finding.severity.colors(theme);
            let outdated = finding.placement == review::Placement::Outdated;
            let state = run.state(&finding.id);
            let outcome = run.outcome(&finding.id);
            let reason = outcome
                .filter(|_| matches!(state, FindingState::Skipped | FindingState::NoChange))
                .and_then(|outcome| outcome.note.clone());
            let label = match outcome.and_then(|outcome| outcome.commit.as_deref()) {
                Some(sha) if state == FindingState::Fixed => {
                    format!("Fixed · {}", review::short_sha(sha))
                }
                _ => state.label().to_string(),
            };
            let (badge_text, badge_bg, badge_border) = badge_look(state, theme);
            let id = finding.id.clone();
            let meta = if outdated {
                format!("{} · {} · outdated", finding.id, finding.location())
            } else {
                format!("{} · {}", finding.id, finding.location())
            };
            div()
                .debug_selector({
                    let id = finding.id.clone();
                    move || format!("review-finding-{id}")
                })
                .flex()
                .flex_row()
                .items_start()
                .gap(px(8.0))
                .py(px(3.0))
                .when(
                    outdated || matches!(state, FindingState::Fixed | FindingState::NoChange),
                    |el| el.opacity(0.5),
                )
                .child(
                    div()
                        .flex_none()
                        .px(px(5.0))
                        .rounded(px(4.0))
                        .bg(wash)
                        .text_color(tint)
                        .text_size(crate::typography::ui_rems(10.0))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(finding.severity.word()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_color(theme.text.opacity(0.85))
                                .child(finding.title.clone()),
                        )
                        .children(reason.map(|reason| {
                            div()
                                .text_size(crate::typography::ui_rems(11.0))
                                .text_color(theme.text_muted)
                                .line_clamp(3)
                                .child(reason)
                        }))
                        .child(
                            div()
                                .font_family(theme.font_mono.clone())
                                .text_size(crate::typography::ui_rems(10.5))
                                .text_color(theme.text_faint)
                                .child(meta),
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .px(px(6.0))
                        .rounded(px(5.0))
                        .text_size(crate::typography::ui_rems(10.5))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(badge_text)
                        .when_some(badge_bg, |el, bg| el.bg(bg))
                        .when_some(badge_border, |el, border| {
                            el.border_1().border_color(border)
                        })
                        .when(state == FindingState::AgentOnIt, |el| el.border_dashed())
                        .child(label),
                )
                .when(state == FindingState::Open, |el| {
                    el.child(
                        div()
                            .id(SharedString::from(format!("review-dismiss-{id}")))
                            .flex_none()
                            .size(px(18.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(5.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme.glass_hover()))
                            .role(gpui::Role::Button)
                            .aria_label(format!("Dismiss {}", finding.title))
                            .tooltip(crate::settings::widgets::text_tooltip("Dismiss"))
                            .child(
                                crate::icons::icon(crate::icons::CLOSE)
                                    .size(px(10.0))
                                    .text_color(theme.text_muted),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.dismiss_finding(&id, cx);
                            })),
                    )
                })
        });
        let button = |id: &'static str, label: SharedString, primary: bool| {
            div()
                .id(id)
                .debug_selector(move || id.to_string())
                .px(px(9.0))
                .py(px(3.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .role(gpui::Role::Button)
                .aria_label(label.clone())
                .text_size(crate::typography::ui_rems(12.0))
                .when(primary, |el| el.bg(theme.solid).text_color(theme.on_solid))
                .when(!primary, |el| {
                    el.border_1()
                        .border_color(theme.border)
                        .text_color(theme.text)
                        .hover(|s| s.bg(theme.glass_hover()))
                })
                .child(label)
        };
        let fixable = run.fixable().count();
        Some(
            div()
                .debug_selector(|| "review-findings".into())
                .pl(px(36.0))
                .pr(px(12.0))
                .pb(px(8.0))
                .flex()
                .flex_col()
                .text_size(crate::typography::ui_rems(12.0))
                .children(note)
                .children(retry_error)
                .when(listed, |el| el.children(rows))
                .when(listed, |el| {
                    el.child(
                        div()
                            .pt(px(6.0))
                            .flex()
                            .flex_row()
                            .gap(px(6.0))
                            .when(fixable > 0, |el| {
                                el.child(
                                    button(
                                        "review-fix-all",
                                        format!("Fix {fixable} remaining").into(),
                                        true,
                                    )
                                    .on_click(cx.listener(
                                        |this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.stage_findings(cx);
                                        },
                                    )),
                                )
                            })
                            .child(
                                button("review-open-changes", "Open Changes".into(), false)
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        cx.stop_propagation();
                                        cx.emit(ComposerEvent::OpenChanges);
                                    })),
                            ),
                    )
                })
                .into_any_element(),
        )
    }

    fn stage_findings(&mut self, cx: &mut Context<Self>) {
        let key = self.current_key.clone();
        self.state.update(cx, |state, cx| {
            state.stage_review_findings(&key, None);
            cx.notify();
        });
        self.close_roster(cx);
        cx.notify();
    }

    fn dismiss_finding(&mut self, id: &str, cx: &mut Context<Self>) {
        let key = self.current_key.clone();
        self.state.update(cx, |state, cx| {
            state.dismiss_review_finding(&key, id);
            cx.notify();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::ChatReview;

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
            assert!(matches!(chip_look(&review, turn), Some(Look::Empty(_))));
        }
        let mut clean = picked(Some(Ok((0, 0))));
        let mut run = ReviewRun::new(turn);
        run.status = RunStatus::Done;
        clean.run = Some(run);
        assert!(matches!(chip_look(&clean, turn), Some(Look::Empty(_))));

        let untouched = ChatReview {
            stats: [(ReviewScope::Uncommitted, Ok((0, 0)))].into(),
            ..ChatReview::default()
        };
        assert!(chip_look(&untouched, ReviewScope::Uncommitted).is_none());
    }

    #[test]
    fn the_pill_counts_outcomes_and_leaves_out_the_empty_ones() {
        let reply = (1..=5)
            .map(|n| {
                format!("{{\"path\":\"a\",\"line\":{n},\"severity\":\"bug\",\"title\":\"t\"}}")
            })
            .collect::<Vec<_>>()
            .join(",");
        let review::ReadOutcome::Read(Some(findings)) =
            review::read_outcome("completed", &format!("```zeron-review\n[{reply}]\n```"))
        else {
            panic!("unread");
        };
        let mut delegation: zeron_proto::Delegation = serde_json::from_value(serde_json::json!({
            "delegatedByChatId": "owner",
            "depth": 1,
            "anchor": "task",
            "wake": "never",
            "status": "completed",
            "startedAt": "2026-10-17T10:00:00Z",
        }))
        .unwrap();
        for (id, kind) in [
            ("F1", zeron_proto::FindingOutcomeKind::Fixed),
            ("F2", zeron_proto::FindingOutcomeKind::Fixed),
            ("F3", zeron_proto::FindingOutcomeKind::Skipped),
            ("F4", zeron_proto::FindingOutcomeKind::NoChange),
        ] {
            delegation.finding_outcomes.insert(
                id.into(),
                zeron_proto::FindingOutcome {
                    outcome: kind,
                    note: None,
                    commit: None,
                },
            );
        }
        let mut run = ReviewRun::new(ReviewScope::Uncommitted);
        run.status = RunStatus::Done;
        run.findings = findings;
        run.delegation = Some(delegation.clone());
        assert_eq!(
            outcome_counts(&run),
            "2 fixed · 1 skipped · 1 no change · 1 open"
        );
        delegation.finding_outcomes.remove("F3");
        delegation.finding_outcomes.remove("F4");
        run.delegation = Some(delegation);
        assert_eq!(outcome_counts(&run), "2 fixed · 3 open");
    }
}
