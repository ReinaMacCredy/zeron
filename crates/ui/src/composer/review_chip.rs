use std::collections::HashMap;

use gpui::{
    AnyElement, App, Bounds, Context, Pixels, Point, SharedString, Task, div, prelude::*, px,
};
use zeron_proto::{CheckoutDiff, HarnessId, Model, PermissionMode};
use zeron_rpc::methods;

use super::{Composer, ComposerEvent};
use crate::changes::{default_base_ref, parse_patch};
use crate::popover::{self, Popup};
use crate::review::{
    self, PromptInput, ReviewRun, ReviewScope, ReviewerChoice, ReviewerOptions, RunStatus,
};
use crate::theme::Theme;

const CHIP_HEIGHT: f32 = 28.0;

#[derive(Default)]
pub(super) struct ReviewChipUi {
    pub(super) menu: Popup<()>,
    stats_task: Option<Task<()>>,
    spawn_task: Option<Task<()>>,
    read_task: Option<Task<()>>,
    options: Option<ReviewerOptions>,
    options_task: Option<Task<()>>,
    model_menu: Option<(usize, HarnessId)>,
    model_intent: popover::HoverIntent<usize>,
    model_bounds: Option<Bounds<Pixels>>,
    model_left: bool,
    models: HashMap<HarnessId, Vec<Model>>,
    models_task: Option<Task<()>>,
    busy_hover: bool,
}

#[derive(Clone)]
struct Target {
    chat_id: String,
    cwd: String,
    branch: Option<String>,
    worktree: bool,
    harness: Option<HarnessId>,
    permission: Option<PermissionMode>,
}

enum Look {
    Idle(u32, u32),
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
) -> Result<(CheckoutDiff, Option<String>), String> {
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
    choice: &ReviewerChoice,
) -> Result<(String, String, Option<String>), String> {
    let (diff, base) = checkout_diff(engine, target, scope).await?;
    let files: Vec<String> = diff.files.iter().map(|file| file.path.clone()).collect();
    if files.is_empty() {
        return Err("Nothing to review".to_string());
    }
    let options = review::reviewer_options(engine).await;
    let reviewer = review::resolve_reviewer(choice, target.harness, &options);
    let profile = reviewer.profile.as_ref();
    let permission_mode = profile
        .and_then(|profile| profile.permission_mode)
        .filter(|mode| {
            target
                .permission
                .is_some_and(|owner| mode.rank() <= owner.rank())
        });
    let task = review::prompt(PromptInput {
        scope,
        base: base.as_deref(),
        patch: &diff.patch,
        truncated: diff.truncated,
        files: &files,
    });
    let params = serde_json::json!({
        "ownerChatId": target.chat_id,
        "task": task,
        "title": "Review changes",
        "role": "review",
        "wake": "never",
        "clientRequestId": review::request_id(scope, &uuid::Uuid::new_v4().to_string()),
        "harness": reviewer.harness,
        "model": reviewer.model,
        "reasoning": profile.and_then(|profile| profile.reasoning),
        "permissionMode": permission_mode,
        "modelOptions": profile
            .filter(|profile| !profile.model_options.is_empty())
            .map(|profile| profile.model_options.clone()),
        "instructions": profile.map(|profile| profile.instructions.clone()),
        "profileId": profile.map(|profile| profile.id.clone()),
        "disabledTools": profile.map(|profile| profile.disabled_tools.clone()).unwrap_or_default(),
    });
    let view = engine
        .client()
        .call(methods::DELEGATE_TASK, params)
        .await
        .map_err(|err| err.to_string())?;
    let task_id = view
        .get("taskId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "The reviewer did not start".to_string())?
        .to_string();
    Ok((task_id, diff.patch, reviewer.note))
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
            permission: chat.config.as_ref().map(|config| config.permission_mode),
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
                chat.delegation.as_ref().is_some_and(|d| {
                    d.delegated_by_chat_id == chat_id
                        && d.client_request_id
                            .as_deref()
                            .is_some_and(|id| id.starts_with(review::REQUEST_PREFIX))
                })
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
        if self
            .state
            .read(cx)
            .reviews
            .get(&target.chat_id)
            .is_none_or(|review| review.run.is_none())
            && let Some(child) = self.reviewer_child(&target.chat_id, None, cx)
            && let Some(scope) = child
                .delegation
                .as_ref()
                .and_then(|d| d.client_request_id.as_deref())
                .and_then(review::request_scope)
        {
            let mut run = ReviewRun::new(scope);
            run.task_id = Some(child.id.clone());
            run.status = RunStatus::Running;
            self.state.update(cx, |state, _| {
                state.reviews.entry(target.chat_id.clone()).or_default().run = Some(run);
            });
        }
        self.read_finished_review(&target, cx);

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
        let key = format!(
            "{}|{}|{scope:?}|{run_scope:?}|{marker}",
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
            self.fetch_review_stats(target, scopes, key, cx);
        }
    }

    fn fetch_review_stats(
        &mut self,
        target: Target,
        scopes: Vec<ReviewScope>,
        key: String,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.state.update(cx, |state, _| {
            state
                .reviews
                .entry(target.chat_id.clone())
                .or_default()
                .stats_key = Some(key.clone());
        });
        self.review.stats_task = Some(cx.spawn(async move |this, cx| {
            let mut fetched = Vec::new();
            for scope in scopes {
                fetched.push((scope, checkout_diff(&engine, &target, scope).await));
            }
            this.update(cx, |composer, cx| {
                composer.state.update(cx, |state, cx| {
                    let review = state.reviews.entry(target.chat_id.clone()).or_default();
                    if review.stats_key.as_deref() != Some(key.as_str()) {
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
        }));
    }

    fn read_finished_review(&mut self, target: &Target, cx: &mut Context<Self>) {
        let Some((task_id, scope, reviewed)) = self
            .state
            .read(cx)
            .reviews
            .get(&target.chat_id)
            .and_then(|review| review.run.as_ref())
            .filter(|run| run.status == RunStatus::Running)
            .and_then(|run| Some((run.task_id.clone()?, run.scope, run.reviewed.clone())))
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
        self.review.read_task = Some(cx.spawn(async move |this, cx| {
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
                None => checkout_diff(&engine, &target, scope)
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
        let mut run = ReviewRun::new(scope);
        run.choice = Some(choice.clone());
        self.state.update(cx, |state, cx| {
            state.reviews.entry(chat_id.clone()).or_default().run = Some(run);
            cx.notify();
        });
        self.review.spawn_task = Some(cx.spawn(async move |this, cx| {
            let started = launch_review(&engine, &target, scope, &choice).await;
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
                        Err(message) => run.status = RunStatus::Failed(message),
                    }
                });
            })
            .ok();
        }));
    }

    fn reviewer_choice(&self, chat_id: &str, cx: &App) -> ReviewerChoice {
        self.state
            .read(cx)
            .reviews
            .get(chat_id)
            .and_then(|review| review.reviewer.clone())
            .unwrap_or_else(|| crate::settings::review_reviewer(cx))
    }

    fn set_reviewer(&mut self, choice: ReviewerChoice, cx: &mut Context<Self>) {
        let key = self.current_key.clone();
        self.state.update(cx, |state, cx| {
            state.reviews.entry(key).or_default().reviewer = Some(choice);
            cx.notify();
        });
        self.close_review_menu(cx);
    }

    fn close_review_menu(&mut self, cx: &mut Context<Self>) {
        self.review.model_intent.reset();
        self.review.model_menu = None;
        self.review.model_bounds = None;
        if self.review.menu.begin_close() {
            popover::reap_popup(cx, |composer: &mut Self| &mut composer.review.menu);
            cx.notify();
        }
    }

    fn open_review_menu(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.review_target(cx) else {
            return;
        };
        self.review.menu.open(());
        let key = format!("menu|{}", uuid::Uuid::new_v4());
        self.fetch_review_stats(target, ReviewScope::ALL.to_vec(), key, cx);
        if let Some(engine) = self.state.read(cx).engine().cloned() {
            self.review.options_task = Some(cx.spawn(async move |this, cx| {
                let options = review::reviewer_options(&engine).await;
                this.update(cx, |composer, cx| {
                    composer.review.options = Some(options);
                    cx.notify();
                })
                .ok();
            }));
        }
        cx.notify();
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
        let review = self.state.read(cx).reviews.get(&target.chat_id)?;
        let counts = review.changes(scope).filter(|(add, del)| add + del > 0);
        let Some(run) = &review.run else {
            return counts.map(|(add, del)| Look::Idle(add, del));
        };
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
            _ if !run.findings.is_empty() => Look::Findings(run.findings.len()),
            _ if run.reported > 0 => return counts.map(|(add, del)| Look::Idle(add, del)),
            _ if counts.is_none() => return None,
            _ => Look::Clean,
        };
        Some(look)
    }

    pub(super) fn render_review_chip(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.hosts_review() {
            return None;
        }
        let target = self.review_target(cx)?;
        let scope = self.review_scope(&target, cx);
        let look = self.review_look(&target, scope, cx)?;
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
        let (icon, label, color, border, fill, tip): (&str, SharedString, _, _, _, SharedString) =
            match &look {
                Look::Idle(..) => (
                    crate::icons::CHECKLIST,
                    "Review".into(),
                    theme.text,
                    theme.border,
                    None,
                    format!(
                        "Review {} with a fresh agent",
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
        let trailing_counts = match look {
            Look::Idle(add, del) | Look::Again(add, del) | Look::Stopped(add, del) => {
                Some(counts(add, del))
            }
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
            .when(!busy || stoppable, |el| {
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
            .when(!busy, |el| {
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
            .then(|| self.render_review_menu(&target, scope, theme, cx));
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
            Some(Look::Busy) | None => {}
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
        let (mark, tint) = options.brand(&next, target.harness);
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
        let rows: Vec<_> = rows.collect();
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
        let open_models = self.review.model_menu;
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
                    }),
                    None => options.label(&choice),
                };
                let detail = match (&choice, &effective) {
                    (ReviewerChoice::Auto, _) => auto_name.clone(),
                    (
                        _,
                        ReviewerChoice::Harness {
                            harness: h,
                            model: Some(model),
                        },
                    ) if selected && harness == Some(*h) => Some(self.model_label(*h, model)),
                    _ => None,
                };
                let (mark, tint) = options.brand(&choice, target.harness);
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
                    row = row.child(self.model_menu_probe(open, cx));
                    if open {
                        row = row.child(popover::nested_menu(
                            format!("composer-reviewer-models-{ix}"),
                            self.render_model_menu(harness, &effective, theme, cx),
                            self.review.model_left,
                        ));
                    }
                }
                div()
                    .id(("composer-reviewer-hover", ix))
                    .on_hover(cx.listener(move |this, hovered: &bool, window, cx| {
                        if *hovered {
                            this.hover_reviewer_row(
                                ix,
                                harness,
                                window.mouse_position(),
                                false,
                                cx,
                            );
                        } else {
                            this.review.model_intent.leave(&ix);
                        }
                    }))
                    .on_mouse_move(
                        cx.listener(move |this, event: &gpui::MouseMoveEvent, _, cx| {
                            this.hover_reviewer_row(ix, harness, event.position, true, cx);
                        }),
                    )
                    .child(row)
                    .into_any_element()
            })
            .collect();
        let review_label = format!(
            "Review {}",
            selected.label(review.base.as_deref()).to_lowercase()
        );
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
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_review_menu(cx)))
            .child(popover::menu_heading(&popup, "Scope"))
            .children(rows)
            .child(popover::menu_separator())
            .child(popover::menu_heading(&popup, "Reviewer"))
            .children(reviewer_rows)
            .child(popover::menu_separator())
            .child(review_now)
            .into_any_element();
        popover::anchored_menu_above(
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

    fn set_model_menu(&mut self, open: Option<(usize, HarnessId)>, cx: &mut Context<Self>) {
        if self.review.model_menu != open {
            self.review.model_menu = open;
            self.review.model_bounds = None;
            cx.notify();
        }
        let Some((_, harness)) = open else {
            return;
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

    fn hover_reviewer_row(
        &mut self,
        key: usize,
        harness: Option<HarnessId>,
        pointer: Point<Pixels>,
        moved: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.review.menu.is_open() {
            return;
        }
        let current = self.review.model_menu.map(|(open, _)| open);
        let (bounds, left) = (self.review.model_bounds, self.review.model_left);
        let action = if moved {
            self.review
                .model_intent
                .moved(current.as_ref(), &key, pointer, bounds, left)
        } else {
            self.review
                .model_intent
                .enter(current.as_ref(), &key, pointer, bounds, left)
        };
        let target = harness.map(|harness| (key, harness));
        match action {
            popover::HoverAction::None => {}
            popover::HoverAction::Open => {
                self.set_model_menu(target, cx);
                self.review.model_intent.record_origin(pointer);
            }
            popover::HoverAction::Defer => {
                self.review.model_intent.defer(cx, move |this, cx| {
                    if this.review.menu.is_open()
                        && this.review.model_intent.pending() == Some(&key)
                    {
                        this.review.model_intent.cancel();
                        this.set_model_menu(target, cx);
                        this.review.model_intent.record_origin(pointer);
                    }
                });
            }
        }
    }

    fn model_menu_probe(&self, open: bool, cx: &mut Context<Self>) -> AnyElement {
        let entity = cx.entity().downgrade();
        let exit = entity.clone();
        gpui::canvas(
            move |bounds, window, cx| {
                let left = bounds.right() + px(220.0) > window.viewport_size().width;
                let _ = entity.update(cx, |this, cx| {
                    if this.review.model_left != left {
                        this.review.model_left = left;
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
                        let (bounds, left) = (this.review.model_bounds, this.review.model_left);
                        if this.review.model_intent.contains_pointer(
                            trigger,
                            bounds,
                            event.position,
                            left,
                        ) {
                            return;
                        }
                        this.review.model_intent.reset();
                        this.set_model_menu(None, cx);
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
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let popup = theme.for_popup();
        let current = match effective {
            ReviewerChoice::Harness { harness: h, model } if *h == harness => Some(model.clone()),
            _ => None,
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
        let entity = cx.entity().downgrade();
        popover::popover_card(&popup)
            .id("composer-reviewer-models-card")
            .w(px(210.0))
            .max_h(px(360.0))
            .overflow_y_scroll()
            .relative()
            .p(px(4.0))
            .flex()
            .flex_col()
            .gap(px(popover::MENU_GAP))
            .children(rows)
            .child(
                gpui::canvas(
                    move |bounds, _, cx| {
                        let _ =
                            entity.update(cx, |this, _| this.review.model_bounds = Some(bounds));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .into_any_element()
    }

    pub(super) fn review_menu_open(&self) -> bool {
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
                let n = run.findings.len();
                let mut text = format!("{n} finding{}", if n == 1 { "" } else { "s" });
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
        let rows = run.findings.iter().map(|finding| {
            let (tint, wash) = finding.severity.colors(theme);
            let outdated = finding.placement == review::Placement::Outdated;
            let id = finding.id.clone();
            div()
                .flex()
                .flex_row()
                .items_start()
                .gap(px(8.0))
                .py(px(3.0))
                .when(outdated, |el| el.opacity(0.5))
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
                        .child(
                            div()
                                .font_family(theme.font_mono.clone())
                                .text_size(crate::typography::ui_rems(10.5))
                                .text_color(theme.text_faint)
                                .child(if outdated {
                                    format!("{} · outdated", finding.location())
                                } else {
                                    finding.location()
                                }),
                        ),
                )
                .child(
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
        });
        let button = |id: &'static str, label: &'static str, primary: bool| {
            div()
                .id(id)
                .debug_selector(move || id.to_string())
                .px(px(9.0))
                .py(px(3.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .role(gpui::Role::Button)
                .aria_label(label)
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
        let fixable = run
            .findings
            .iter()
            .any(|finding| finding.placement != review::Placement::Outdated);
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
                .when(listed, |el| el.children(rows))
                .when(listed, |el| {
                    el.child(
                        div()
                            .pt(px(6.0))
                            .flex()
                            .flex_row()
                            .gap(px(6.0))
                            .when(fixable, |el| {
                                el.child(button("review-fix-all", "Fix all", true).on_click(
                                    cx.listener(|this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.stage_findings(cx);
                                    }),
                                ))
                            })
                            .child(
                                button("review-open-changes", "Open Changes", false).on_click(
                                    cx.listener(|_, _, _, cx| {
                                        cx.stop_propagation();
                                        cx.emit(ComposerEvent::OpenChanges);
                                    }),
                                ),
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
        self.close_roster();
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
