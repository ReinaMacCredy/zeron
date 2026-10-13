use gpui::{AnyElement, App, Context, SharedString, Task, div, prelude::*, px};
use zeron_proto::{CheckoutDiff, HarnessId};
use zeron_rpc::methods;

use super::{Composer, ComposerEvent};
use crate::changes::{default_base_ref, parse_patch};
use crate::popover::{self, Popup};
use crate::review::{self, PromptInput, ReviewRun, ReviewScope, RunStatus};
use crate::theme::Theme;

const CHIP_HEIGHT: f32 = 28.0;

#[derive(Default)]
pub(super) struct ReviewChipUi {
    pub(super) menu: Popup<()>,
    stats_task: Option<Task<()>>,
    spawn_task: Option<Task<()>>,
    read_task: Option<Task<()>>,
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
    Busy,
    Findings(usize),
    Again(u32, u32),
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

fn other_family(chat: Option<HarnessId>, offered: &[HarnessId]) -> Option<HarnessId> {
    let wanted: &[HarnessId] = match chat {
        Some(HarnessId::ClaudeCode) => &[HarnessId::Codex],
        Some(HarnessId::Codex) => &[HarnessId::ClaudeCode],
        _ => &[HarnessId::ClaudeCode, HarnessId::Codex],
    };
    wanted.iter().copied().find(|id| offered.contains(id))
}

async fn offered_harnesses(engine: &crate::state::EngineHandle) -> Vec<HarnessId> {
    let Ok(value) = engine
        .client()
        .call(methods::LIST_HARNESSES, serde_json::json!({}))
        .await
    else {
        return Vec::new();
    };
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| {
            row.get("installed").and_then(|v| v.as_bool()) != Some(false)
                && row.get("enabled").and_then(|v| v.as_bool()) != Some(false)
        })
        .filter_map(|row| serde_json::from_value(row.get("id")?.clone()).ok())
        .collect()
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
            let reviewed = match reviewed {
                Some(patch) => Some(patch),
                None => checkout_diff(&engine, &target, scope)
                    .await
                    .ok()
                    .map(|(diff, _)| diff.patch),
            };
            let outcome = match view {
                Err(err) => Err(err.to_string()),
                Ok(view) => {
                    let summary = view
                        .get("summary")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    if view.get("status").and_then(|v| v.as_str()) == Some("failed") {
                        Err(summary)
                    } else {
                        Ok(review::parse_findings(&summary))
                    }
                }
            };
            this.update(cx, |composer, cx| {
                composer.set_run(&chat_id, cx, |run| {
                    if run.task_id.as_deref() != Some(task_id.as_str()) {
                        return;
                    }
                    match outcome {
                        Err(message) => run.status = RunStatus::Failed(message),
                        Ok(None) => run.status = RunStatus::Unreadable,
                        Ok(Some(mut findings)) => {
                            let files = parse_patch(reviewed.as_deref().unwrap_or_default());
                            review::place_new(&mut findings, &files);
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
        self.state.update(cx, |state, cx| {
            state.reviews.entry(chat_id.clone()).or_default().run = Some(ReviewRun::new(scope));
            cx.notify();
        });
        self.review.spawn_task =
            Some(cx.spawn(async move |this, cx| {
                let started = async {
                let (diff, base) = checkout_diff(&engine, &target, scope).await?;
                let files: Vec<String> = diff.files.iter().map(|file| file.path.clone()).collect();
                if files.is_empty() {
                    return Err("Nothing to review".to_string());
                }
                let offered = offered_harnesses(&engine).await;
                let picked = other_family(target.harness, &offered);
                let harness = picked.or(target.harness);
                let note = (picked.is_none()).then(|| {
                    format!(
                        "No other agent is available, so {} reviews its own family",
                        target.harness.map_or("the chat's agent", super::harness_display_name)
                    )
                });
                let task = review::prompt(PromptInput {
                    scope,
                    base: base.as_deref(),
                    patch: &diff.patch,
                    truncated: diff.truncated,
                    files: &files,
                });
                let mut params = serde_json::json!({
                    "ownerChatId": target.chat_id,
                    "task": task,
                    "title": "Review changes",
                    "role": "review",
                    "wake": "never",
                    "clientRequestId": review::request_id(scope, &uuid::Uuid::new_v4().to_string()),
                });
                if let Some(harness) = harness {
                    params["harness"] = serde_json::to_value(harness).unwrap_or_default();
                }
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
                Ok((task_id, diff.patch, note))
            }
            .await;
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

    fn close_review_menu(&mut self, cx: &mut Context<Self>) {
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
                    theme.warning,
                    theme.warning.opacity(0.45),
                    None,
                    "The reviewer did not end with a findings block. Click to open its thread."
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
        let trailing_counts = match look {
            Look::Idle(add, del) | Look::Again(add, del) => Some(counts(add, del)),
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
            .when(!busy, |el| {
                el.cursor_pointer().hover(|s| s.bg(crate::theme::ink(0.06)))
            })
            .tooltip(crate::settings::widgets::text_tooltip(tip))
            .child(crate::icons::icon(icon).size(px(14.0)).text_color(color))
            .child(label)
            .children(trailing_counts)
            .when(!busy, |el| {
                el.on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.on_review_click(scope, cx);
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
            .when(matches!(look, Look::Again(..)), |el| el.border_dashed())
            .when(busy, |el| el.opacity(0.55))
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
            Some(Look::Unreadable) => {
                let task = self
                    .state
                    .read(cx)
                    .reviews
                    .get(&target.chat_id)
                    .and_then(|review| review.run.as_ref())
                    .and_then(|run| run.task_id.clone());
                if let Some(task) = task {
                    cx.emit(ComposerEvent::OpenChat(task));
                }
            }
            Some(Look::Busy) | None => {}
            Some(_) => self.start_review(scope, cx),
        }
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
            .child(review_now)
            .into_any_element();
        popover::anchored_menu_above(
            "composer-review-popup",
            menu,
            self.review.menu.closing_since(),
        )
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
            _ => return None,
        };
        Some(suffix)
    }

    pub(super) fn render_review_findings(
        &self,
        doc_id: &str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let run = self.review_run_for(doc_id, cx)?;
        if run.status != RunStatus::Done || run.findings.is_empty() {
            return None;
        }
        let rows = run.findings.iter().map(|finding| {
            let (tint, wash) = match finding.severity {
                review::Severity::Bug => (theme.danger, theme.danger.opacity(0.14)),
                review::Severity::Risk => (theme.warning, theme.warning.opacity(0.14)),
                review::Severity::Nit => (theme.text_muted, crate::theme::ink(0.08)),
            };
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
                .children(rows)
                .child(
                    div()
                        .pt(px(6.0))
                        .flex()
                        .flex_row()
                        .gap(px(6.0))
                        .when(fixable, |el| {
                            el.child(button("review-fix-all", "Fix all", true).on_click(
                                cx.listener(|this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.stage_findings(None, cx);
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
                .into_any_element(),
        )
    }

    pub(crate) fn stage_findings(&mut self, only: Option<String>, cx: &mut Context<Self>) {
        let key = self.current_key.clone();
        self.state.update(cx, |state, cx| {
            let Some(run) = state
                .reviews
                .get_mut(&key)
                .and_then(|review| review.run.as_mut())
            else {
                return;
            };
            let (staged, kept): (Vec<_>, Vec<_>) = run.findings.drain(..).partition(|finding| {
                finding.placement != review::Placement::Outdated
                    && only.as_ref().is_none_or(|id| &finding.id == id)
            });
            run.findings = kept;
            for finding in staged {
                state.add_review_comment(&key, finding.comment());
            }
            cx.notify();
        });
        self.close_roster();
        cx.notify();
    }

    pub(crate) fn dismiss_finding(&mut self, id: &str, cx: &mut Context<Self>) {
        let key = self.current_key.clone();
        self.set_run(&key, cx, |run| {
            run.findings.retain(|finding| finding.id != id)
        });
    }
}
