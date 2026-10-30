use std::collections::HashMap;

use gpui::{
    AnyElement, App, Bounds, Context, Pixels, Point, SharedString, Task, div, prelude::*, px,
};
use zeron_proto::{CheckoutDiff, HarnessId, Model};
use zeron_rpc::methods;

use super::{Composer, ComposerEvent};
use crate::popover::{self, Popup};
use crate::review::{
    self, FindingState, Look, ReviewRun, ReviewScope, ReviewSettings, ReviewerChoice,
    ReviewerOptions, RunStatus,
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

async fn checkout_diff(
    engine: &crate::state::EngineHandle,
    target: &Target,
    scope: ReviewScope,
    commit: Option<&str>,
) -> Result<(CheckoutDiff, Option<String>), String> {
    let branches: Vec<String> = match scope {
        ReviewScope::Branch => serde_json::from_value(
            engine
                .client()
                .call(
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
    let value = engine
        .client()
        .call(
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

async fn launch_review(
    engine: &crate::state::EngineHandle,
    target: &Target,
    scope: ReviewScope,
    commit: Option<&str>,
    choice: Option<&ReviewerChoice>,
) -> Result<review::StartedReview, String> {
    let started = engine
        .client()
        .call(
            methods::START_REVIEW,
            review::start_params(&target.chat_id, scope, commit, choice),
        )
        .await
        .map_err(|err| err.to_string())?;
    serde_json::from_value::<review::StartedReview>(started)
        .ok()
        .filter(|started| !started.task_id.is_empty())
        .ok_or_else(|| "The reviewer did not start".to_string())
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
        review::reviewer_child(&self.state.read(cx).chats, chat_id, task_id).cloned()
    }

    pub(super) fn sync_review(&mut self, cx: &mut Context<Self>) {
        if !self.hosts_review() {
            return;
        }
        let Some(target) = self.review_target(cx) else {
            return;
        };
        self.state.update(cx, |state, _| {
            state
                .reviews
                .entry(target.chat_id.clone())
                .or_default()
                .adopt(&state.chats, &target.chat_id);
        });
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
                        review.apply_stats(scope, result);
                    }
                    cx.notify();
                });
            })
            .ok();
        })
        .detach();
    }

    fn read_finished_review(&mut self, target: &Target, cx: &mut Context<Self>) {
        let Some(review::FinishedRead {
            task_id,
            scope,
            reviewed,
            commit,
        }) = ({
            let state = self.state.read(cx);
            state
                .reviews
                .get(&target.chat_id)
                .and_then(|review| review.finished(&state.chats, &target.chat_id))
        })
        else {
            return;
        };
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
                Ok(view) => review::read_task_view(&view),
            };
            this.update(cx, |composer, cx| {
                composer.set_run(&chat_id, cx, |run| {
                    if run.task_id.as_deref() == Some(task_id.as_str()) {
                        run.apply_read(outcome, reviewed.as_deref(), restored);
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
            if run.sync_child(delegation, owner_working) {
                cx.notify();
            }
            run.next_hand()
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
                    review::hand_params(&owner, &task_id, &ids, withdraw),
                )
                .await;
            this.update(cx, |composer, cx| {
                composer.review.hand_task = None;
                if let Err(err) = handed {
                    tracing::warn!(error = %err, "review findings not handed");
                    composer.set_run(&owner, cx, |run| {
                        if run.task_id.as_deref() == Some(task_id.as_str()) {
                            run.hand_failed(&ids, withdraw);
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
        let (previous, chosen, commit) = self.state.update(cx, |state, cx| {
            let review = state.reviews.entry(chat_id.clone()).or_default();
            let previous = review.begin(scope, choice);
            cx.notify();
            (
                previous,
                review.reviewer.clone(),
                review.run.as_ref().and_then(|run| run.commit.clone()),
            )
        });
        cx.spawn(async move |this, cx| {
            let started =
                launch_review(&engine, &target, scope, commit.as_deref(), chosen.as_ref()).await;
            this.update(cx, |composer, cx| {
                composer.set_run(&chat_id, cx, |run| run.apply_start(started, previous));
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
            .cloned()
            .unwrap_or_default()
            .effective_reviewer(self.review.settings.as_ref())
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
        self.state
            .read(cx)
            .reviews
            .get(&target.chat_id)?
            .look(scope)
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
        let label: SharedString = look.label(scope, picked.as_deref()).into();
        let tip: SharedString = look.tip(scope).into();
        let (icon, color, border, fill) = match &look {
            Look::Idle(..) => (crate::icons::CHECKLIST, theme.text, theme.border, None),
            Look::Findings(_) => (
                crate::icons::CHECKLIST,
                theme.danger,
                theme.danger.opacity(0.45),
                Some(theme.danger.opacity(0.10)),
            ),
            Look::Again(..) | Look::Stopped(..) => {
                (crate::icons::REFRESH, theme.text_muted, theme.border, None)
            }
            Look::Clean => (
                crate::icons::LUCIDE_CIRCLE_CHECK,
                theme.success,
                theme.success.opacity(0.40),
                None,
            ),
            Look::Failed(_) => (
                crate::icons::CHECKLIST,
                theme.danger,
                theme.danger.opacity(0.45),
                None,
            ),
            Look::Empty(_) | Look::Busy | Look::Unreadable => (
                crate::icons::CHECKLIST,
                theme.text_muted,
                theme.border,
                None,
            ),
        };
        let busy = matches!(look, Look::Busy);
        let stoppable = busy
            && self
                .state
                .read(cx)
                .reviews
                .get(&target.chat_id)
                .is_some_and(|review| review.run.as_ref().is_some_and(ReviewRun::stoppable));
        let stopping = stoppable && self.review.busy_hover;
        let (icon, label, color) = if stopping {
            (
                crate::icons::STOP,
                SharedString::from(review::STOP_LABEL),
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
                SharedString::from(review::STOP_TIP)
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
            .aria_label(review::REVIEW_OPTIONS)
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
        match self
            .review_look(&target, scope, cx)
            .map(|look| look.action())
        {
            Some(review::ChipAction::OpenFindings) => self.open_review_findings(cx),
            Some(review::ChipAction::Start) => self.start_review(scope, cx),
            Some(review::ChipAction::Inert) | None => {}
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
        let next = self.reviewer_choice(&target.chat_id, cx);
        if !self
            .state
            .read(cx)
            .reviews
            .get(&target.chat_id)?
            .reviewer_changed(&next)
        {
            return None;
        }
        let name = review::choice_name(self.review.options.as_ref(), &next, target.harness);
        let options = self.review.options.clone().unwrap_or_default();
        let (mark, tint) = review::brand(&options, &next, target.harness);
        Some((name, mark, tint))
    }

    fn open_review_findings(&mut self, cx: &mut Context<Self>) {
        cx.emit(ComposerEvent::OpenChanges);
    }

    pub(crate) fn review_change_counts(&self, cx: &App) -> Option<(u32, u32)> {
        if !self.hosts_review() {
            return None;
        }
        let target = self.review_target(cx)?;
        let scope = self.review_scope(&target, cx);
        self.state
            .read(cx)
            .reviews
            .get(&target.chat_id)?
            .changes(scope)
            .filter(|(add, del)| add + del > 0)
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
        let rows = review
            .scope_rows(target.branch.as_deref())
            .into_iter()
            .map(|row| {
                let scope = row.scope;
                let id = SharedString::from(format!("composer-review-scope-{}", scope.key()));
                let selector = id.clone();
                popover::menu_row(&popup, false, id.clone())
                    .id(id)
                    .debug_selector(move || selector.to_string())
                    .role(gpui::Role::MenuItemRadio)
                    .aria_label(row.label.clone())
                    .gap(px(8.0))
                    .child(crate::settings::widgets::select_check(
                        &popup,
                        scope == selected,
                    ))
                    .child(div().flex_1().child(row.label.clone()))
                    .child(match row.counts {
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
                            .child(row.note.clone().unwrap_or_else(|| "…".to_string()))
                            .into_any_element(),
                    })
                    .when(row.empty, |el| el.opacity(0.5))
                    .when(row.note.is_none(), |el| {
                        el.on_click(
                            cx.listener(move |this, _, _, cx| this.set_review_scope(scope, cx)),
                        )
                    })
            });
        let mut rows: Vec<AnyElement> = rows.map(IntoElement::into_any_element).collect();
        rows.push(self.render_commit_row(&review, selected, &popup, window, cx));
        let effective = self.reviewer_choice(&target.chat_id, cx);
        let options = self.review.options.clone().unwrap_or_default();
        let open_models = self.review.submenu;
        let reviewer_rows: Vec<AnyElement> = review::reviewer_rows(
            self.review.options.as_ref(),
            &effective,
            target.harness,
            |harness, model| self.model_label(harness, model),
        )
        .into_iter()
        .enumerate()
        .map(
            |(
                ix,
                review::ReviewerRow {
                    choice,
                    harness,
                    label,
                    detail,
                    selected,
                },
            )| {
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
            },
        )
        .collect();
        let review_label = review.start_label(selected);
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
            .child(popover::menu_heading(&popup, review::SCOPE_HEADING))
            .children(rows)
            .child(popover::menu_separator())
            .child(popover::menu_heading(&popup, review::REVIEWER_HEADING))
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
        let levels =
            options.reasoning_stops(harness, model_entry.map(|m| m.reasoning_levels.as_slice()));
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
                                .child(popover::menu_heading(popup, review::THINKING_HEADING)),
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
            .aria_label(review::COMMIT_LABEL)
            .gap(px(8.0))
            .child(crate::settings::widgets::select_check(
                popup,
                selected == ReviewScope::Commit,
            ))
            .child(div().flex_1().child(review::COMMIT_LABEL))
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
            .map(|input| input.read(cx).text().to_string())
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
                .filter(|commit| review::commit_matches(&commit.subject, &commit.sha, &query))
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
                                .child(review::NO_MATCHING_COMMITS),
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

    pub(crate) fn render_review_findings(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.hosts_review() {
            return None;
        }
        let run = self
            .state
            .read(cx)
            .reviews
            .get(&self.current_key)?
            .run
            .clone()?;
        let listed = run.findings_listed();
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
                .child(review::retry_failed(&message))
        });
        let rows = run.findings.iter().map(|finding| {
            let (tint, wash) = review::severity_colors(finding.severity, theme);
            let outdated = finding.placement == review::Placement::Outdated;
            let state = run.state(&finding.id);
            let reason = run.reason(&finding.id);
            let label = run.state_label(&finding.id);
            let (badge_text, badge_bg, badge_border) = badge_look(state, theme);
            let id = finding.id.clone();
            let meta = finding.meta();
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
                .when(outdated || state.settled(), |el| el.opacity(0.5))
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
                            .tooltip(crate::settings::widgets::text_tooltip(review::DISMISS))
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
        let fix_label: SharedString = review::fix_remaining(run.fixable().count()).into();
        let fixable = run.fixable().count();
        Some(
            div()
                .debug_selector(|| "review-findings".into())
                .px(px(12.0))
                .py(px(8.0))
                .flex()
                .flex_col()
                .text_size(crate::typography::ui_rems(12.0))
                .children(note)
                .children(retry_error)
                .when(listed, |el| el.children(rows))
                .when(listed, |el| {
                    el.child(div().pt(px(6.0)).flex().flex_row().gap(px(6.0)).when(
                        fixable > 0,
                        |el| {
                            el.child(
                                div()
                                    .id("review-fix-all")
                                    .debug_selector(|| "review-fix-all".into())
                                    .px(px(9.0))
                                    .py(px(3.0))
                                    .rounded(px(6.0))
                                    .cursor_pointer()
                                    .role(gpui::Role::Button)
                                    .aria_label(fix_label.clone())
                                    .text_size(crate::typography::ui_rems(12.0))
                                    .bg(theme.solid)
                                    .text_color(theme.on_solid)
                                    .child(fix_label)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.stage_findings(cx);
                                    })),
                            )
                        },
                    ))
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
