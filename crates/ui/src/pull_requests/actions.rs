use gpui::{
    AnyElement, ClickEvent, Context, Entity, SharedString, Subscription, Task, div, prelude::*, px,
};
use zeron_proto::{
    ChangeRequestAction, ChangeRequestChecksState, ChangeRequestDetail, ChangeRequestFailure,
    ChangeRequestLabelCandidates, ChangeRequestListItem, ChangeRequestMergeMethod,
    ChangeRequestMergeability, ChangeRequestPermissions, ChangeRequestReviewerCandidates,
    ChangeRequestState, GetChangeRequestRequest, RunChangeRequestActionRequest,
    SetChangeRequestLabelRequest, SetChangeRequestReviewerRequest, UpdateChangeRequestRequest,
};
use zeron_rpc::methods;

use super::detail::is_workflow_approval;
use super::{PullRequestKey, PullRequestsPage, Tones, avatar, failure_message, label_color};
use crate::composer::{ComposerInput, ComposerInputEvent};
use crate::icons::{self, icon};
use crate::popover::{self, Loadable, Popup};
use crate::settings::widgets::text_tooltip;
use crate::theme::Theme;

pub(super) const MERGE_METHODS: [(ChangeRequestMergeMethod, &str, &str); 3] = [
    (ChangeRequestMergeMethod::Merge, "Merge", "merge"),
    (
        ChangeRequestMergeMethod::Squash,
        "Squash and merge",
        "squash",
    ),
    (
        ChangeRequestMergeMethod::Rebase,
        "Rebase and merge",
        "rebase",
    ),
];

fn method_label(method: ChangeRequestMergeMethod) -> &'static str {
    MERGE_METHODS
        .iter()
        .find(|(known, _, _)| *known == method)
        .map(|(_, label, _)| *label)
        .unwrap_or("Merge")
}

fn method_word(method: ChangeRequestMergeMethod) -> &'static str {
    MERGE_METHODS
        .iter()
        .find(|(known, _, _)| *known == method)
        .map(|(_, _, word)| *word)
        .unwrap_or("merge")
}

/// The current pick, else the last one used, else the first the repository allows, else merge
/// (t3code `resolvePullRequestMergeMethod`; there is no per-project default here).
pub(super) fn pick_method(
    current: Option<ChangeRequestMergeMethod>,
    last: Option<ChangeRequestMergeMethod>,
    allowed: &[ChangeRequestMergeMethod],
) -> ChangeRequestMergeMethod {
    current
        .filter(|method| allowed.contains(method))
        .or(last.filter(|method| allowed.contains(method)))
        .or(allowed.first().copied())
        .unwrap_or(ChangeRequestMergeMethod::Merge)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Primary {
    Nothing,
    Merged,
    Closed,
    Resolve,
    Ready,
    AutoMergeArmed,
    EnableAutoMerge(ChangeRequestMergeMethod),
    Merge(ChangeRequestMergeMethod),
}

/// t3code `resolvePullRequestPrimaryControl`: the first matching rule wins.
pub(super) fn primary_control(
    item: &ChangeRequestListItem,
    detail: Option<&ChangeRequestDetail>,
    permissions: Option<&ChangeRequestPermissions>,
    method: ChangeRequestMergeMethod,
) -> Primary {
    match item.state {
        ChangeRequestState::Merged => return Primary::Merged,
        ChangeRequestState::Closed => return Primary::Closed,
        ChangeRequestState::Open => {}
    }
    if item.mergeability == ChangeRequestMergeability::Conflicting {
        return Primary::Resolve;
    }
    let Some(permissions) = permissions else {
        return Primary::Nothing;
    };
    if item.is_draft {
        return if permissions.can_update {
            Primary::Ready
        } else {
            Primary::Nothing
        };
    }
    if detail.is_some_and(|detail| detail.auto_merge.is_some()) {
        return Primary::AutoMergeArmed;
    }
    if permissions.merge_methods.is_empty() || !permissions.can_write {
        return Primary::Nothing;
    }
    if detail.is_some() && item.checks_state != Some(ChangeRequestChecksState::Passing) {
        return Primary::EnableAutoMerge(method);
    }
    Primary::Merge(method)
}

pub(super) fn can_edit(permissions: Option<&ChangeRequestPermissions>) -> bool {
    permissions.is_some_and(|permissions| permissions.did_author || permissions.can_write)
}

pub(super) fn pending_workflows(detail: &ChangeRequestDetail) -> usize {
    detail
        .checks
        .iter()
        .filter(|check| is_workflow_approval(check))
        .count()
}

pub(super) struct Confirm {
    key: PullRequestKey,
    action: ChangeRequestAction,
    title: String,
    description: String,
    button: String,
    destructive: bool,
}

/// t3code's confirm dialog copy (`Panel:2445-2511`); actions without an entry run at once.
pub(super) fn confirm_copy(
    action: ChangeRequestAction,
    number: u64,
    workflows: usize,
) -> Option<(String, String, String, bool)> {
    Some(match action {
        ChangeRequestAction::Merge { method } => (
            "Merge pull request?".into(),
            format!("This merges #{number} using {}.", method_word(method)),
            method_label(method).into(),
            false,
        ),
        ChangeRequestAction::EnableAutoMerge { method } => (
            "Enable auto-merge?".into(),
            format!(
                "This merges #{number} using {} as soon as the host considers it ready, which may be immediately.",
                method_word(method)
            ),
            "Enable auto-merge".into(),
            false,
        ),
        ChangeRequestAction::Revert => (
            "Revert these changes?".into(),
            format!("This opens a new pull request that reverses the changes merged by #{number}."),
            "Create revert PR".into(),
            false,
        ),
        ChangeRequestAction::ApproveWorkflows => (
            "Approve workflows to run?".into(),
            format!(
                "This allows {workflows} {} from #{number} to run. Review the code and workflow changes first.",
                if workflows == 1 {
                    "workflow"
                } else {
                    "workflows"
                }
            ),
            "Approve and run".into(),
            false,
        ),
        ChangeRequestAction::Close => (
            "Close pull request?".into(),
            format!("This closes #{number} without merging it."),
            "Close".into(),
            true,
        ),
        _ => return None,
    })
}

/// Success title, failure title, and the hint shown when the host gave no words of its own.
pub(super) fn action_copy(
    action: ChangeRequestAction,
) -> (&'static str, &'static str, &'static str) {
    const REFUSED: &str = "The host refused it.";
    match action {
        ChangeRequestAction::Merge { .. } => (
            "Pull request merged",
            "Could not merge this pull request",
            "The host refused the merge. Check that you have write access, that the checks it requires have passed, and that the branch is not conflicting.",
        ),
        ChangeRequestAction::Ready => (
            "Marked ready for review",
            "Could not mark this ready for review",
            REFUSED,
        ),
        ChangeRequestAction::Draft => (
            "Converted to draft",
            "Could not convert this to a draft",
            REFUSED,
        ),
        ChangeRequestAction::Close => (
            "Pull request closed",
            "Could not close this pull request",
            REFUSED,
        ),
        ChangeRequestAction::Reopen => (
            "Pull request reopened",
            "Could not reopen this pull request",
            REFUSED,
        ),
        ChangeRequestAction::UpdateBranch { rebase } => (
            "Branch updated with the base branch",
            "Could not update this branch",
            if rebase {
                "The host refused it. A rebase stops at the first commit that does not apply cleanly; updating with a merge commit may still work."
            } else {
                REFUSED
            },
        ),
        ChangeRequestAction::EnableAutoMerge { .. } => (
            "Auto-merge turned on — merges as soon as this is ready, sooner if it already is",
            "Could not turn on auto-merge",
            REFUSED,
        ),
        ChangeRequestAction::DisableAutoMerge => (
            "Auto-merge turned off",
            "Could not turn off auto-merge",
            REFUSED,
        ),
        ChangeRequestAction::Revert => (
            "Revert pull request opened",
            "Could not open a revert pull request",
            REFUSED,
        ),
        ChangeRequestAction::ApproveWorkflows => {
            ("Workflows approved", "Could not approve workflows", REFUSED)
        }
    }
}

/// The host's words where readable, else the hint (t3code `readableFailure`).
pub(super) fn readable(message: &str, hint: &str) -> String {
    if let Ok(failure) = serde_json::from_str::<ChangeRequestFailure>(message) {
        return failure_message(failure).to_string();
    }
    let trimmed = message.trim();
    let generic = trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("command failed")
        || trimmed.to_ascii_lowercase().contains("exited with code")
        || trimmed.eq_ignore_ascii_case("unknown error");
    if generic {
        hint.to_string()
    } else if trimmed.chars().count() > 320 {
        format!("{}…", trimmed.chars().take(319).collect::<String>())
    } else {
        trimmed.to_string()
    }
}

struct Editor {
    input: Entity<ComposerInput>,
    _sub: Subscription,
}

pub(super) struct Picker<T> {
    popup: Popup<()>,
    candidates: Loadable<T>,
    search: Option<Editor>,
    busy: bool,
}

impl<T> Default for Picker<T> {
    fn default() -> Self {
        Self {
            popup: Popup::default(),
            candidates: Loadable::Idle,
            search: None,
            busy: false,
        }
    }
}

#[derive(Default)]
pub(super) struct ActionsState {
    pub(super) permissions: Loadable<ChangeRequestPermissions>,
    pending: Option<&'static str>,
    method: Option<ChangeRequestMergeMethod>,
    more: Popup<()>,
    title: Option<Editor>,
    saving_title: bool,
    body: Option<Editor>,
    saving_body: bool,
    labels: Picker<ChangeRequestLabelCandidates>,
    reviewers: Picker<ChangeRequestReviewerCandidates>,
    stale_open: bool,
    _task: Option<Task<()>>,
}

fn action_key(action: ChangeRequestAction) -> &'static str {
    match action {
        ChangeRequestAction::Merge { .. } => "merge",
        ChangeRequestAction::EnableAutoMerge { .. } => "enable-auto-merge",
        ChangeRequestAction::DisableAutoMerge => "disable-auto-merge",
        ChangeRequestAction::Ready => "ready",
        ChangeRequestAction::Draft => "draft",
        ChangeRequestAction::Close => "close",
        ChangeRequestAction::Reopen => "reopen",
        ChangeRequestAction::UpdateBranch { .. } => "update-branch",
        ChangeRequestAction::Revert => "revert",
        ChangeRequestAction::ApproveWorkflows => "approve-workflows",
    }
}

impl ActionsState {
    pub(super) fn close_menus(&mut self) {
        self.more = Popup::default();
        self.close_picker(true);
        self.close_picker(false);
    }

    fn picker_is_open(&self, labels: bool) -> bool {
        if labels {
            self.labels.popup.is_open()
        } else {
            self.reviewers.popup.is_open()
        }
    }

    fn close_picker(&mut self, labels: bool) {
        if labels {
            self.labels.popup = Popup::default();
            self.labels.search = None;
        } else {
            self.reviewers.popup = Popup::default();
            self.reviewers.search = None;
        }
    }

    fn open_picker(&mut self, labels: bool) {
        if labels {
            self.labels.popup.open(());
        } else {
            self.reviewers.popup.open(());
        }
    }
}

impl PullRequestsPage {
    fn last_merge_method(cx: &gpui::App) -> Option<ChangeRequestMergeMethod> {
        crate::settings::current(cx).pull_requests_merge_method
    }

    pub(super) fn merge_method(
        &self,
        key: &PullRequestKey,
        cx: &gpui::App,
    ) -> ChangeRequestMergeMethod {
        let state = &self.details[key].actions;
        let allowed = state
            .permissions
            .ready()
            .map(|permissions| permissions.merge_methods.as_slice())
            .unwrap_or(&[]);
        pick_method(state.method, Self::last_merge_method(cx), allowed)
    }

    pub(super) fn load_permissions(&mut self, key: &PullRequestKey, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let Some(state) = self.details.get_mut(key) else {
            return;
        };
        if state.actions.permissions.ready().is_none() {
            state.actions.permissions = Loadable::Loading;
        }
        let params = serde_json::to_value(GetChangeRequestRequest {
            host: key.host.clone(),
            repository: key.repository.clone(),
            number: key.number,
        })
        .unwrap_or(serde_json::Value::Null);
        let key = key.clone();
        state.actions._task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::GET_CHANGE_REQUEST_PERMISSIONS, params)
                .await
                .map_err(|error| error.to_string())
                .and_then(|value| {
                    serde_json::from_value::<ChangeRequestPermissions>(value)
                        .map_err(|error| error.to_string())
                });
            this.update(cx, |page, cx| {
                if let Some(state) = page.details.get_mut(&key) {
                    match result {
                        Ok(permissions) => state.actions.permissions = Loadable::Ready(permissions),
                        Err(error) if state.actions.permissions.ready().is_none() => {
                            state.actions.permissions = Loadable::Error(error)
                        }
                        Err(_) => {}
                    }
                    cx.notify();
                }
                page.sync_code_review(&key, cx);
            })
            .ok();
        }));
    }

    fn refresh_pull_request(&mut self, key: &PullRequestKey, cx: &mut Context<Self>) {
        self.load_detail(key, cx);
        self.load_permissions(key, cx);
        self.load_activity(key, cx);
        self.refresh(cx);
    }

    pub(super) fn request_action(
        &mut self,
        key: &PullRequestKey,
        action: ChangeRequestAction,
        cx: &mut Context<Self>,
    ) {
        self.close_more_menu(key, cx);
        let workflows = self
            .details
            .get(key)
            .and_then(|state| state.detail.ready().map(pending_workflows))
            .unwrap_or(0);
        match confirm_copy(action, key.number, workflows) {
            Some((title, description, button, destructive)) => {
                self.confirm = Some(Confirm {
                    key: key.clone(),
                    action,
                    title,
                    description,
                    button,
                    destructive,
                });
                cx.notify();
            }
            None => self.run_action(key, action, None, cx),
        }
    }

    pub(super) fn run_action(
        &mut self,
        key: &PullRequestKey,
        action: ChangeRequestAction,
        comment: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let Some(state) = self.details.get_mut(key) else {
            return;
        };
        if state.actions.pending.is_some() {
            return;
        }
        state.actions.pending = Some(action_key(action));
        let with_comment = comment.is_some();
        let params = serde_json::to_value(RunChangeRequestActionRequest {
            host: key.host.clone(),
            repository: key.repository.clone(),
            number: key.number,
            action,
            comment,
        })
        .unwrap_or(serde_json::Value::Null);
        let key = key.clone();
        cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::RUN_CHANGE_REQUEST_ACTION, params)
                .await;
            this.update(cx, |page, cx| {
                if let Some(state) = page.details.get_mut(&key) {
                    state.actions.pending = None;
                }
                let (success, failure, hint) = action_copy(action);
                match result {
                    Ok(_) => {
                        if with_comment && let Some(state) = page.details.get(&key) {
                            let composer = state.activity.composer();
                            composer.update(cx, |input, cx| input.set_text("", cx));
                        }
                        page.toast(success, cx);
                    }
                    Err(error) => page.toast_with(failure, readable(&error.to_string(), hint), cx),
                }
                page.refresh_pull_request(&key, cx);
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn close_more_menu(&mut self, key: &PullRequestKey, cx: &mut Context<Self>) {
        if let Some(state) = self.details.get_mut(key)
            && state.actions.more.begin_close()
        {
            let key = key.clone();
            popover::reap_popup(cx, move |page: &mut Self| {
                &mut page
                    .details
                    .get_mut(&key)
                    .expect("detail kept while its menu closes")
                    .actions
                    .more
            });
            cx.notify();
        }
    }

    /// Escape: the dialog first, then whichever menu, picker or editor is open on `key`.
    pub(super) fn dismiss_action_overlay(&mut self, cx: &mut Context<Self>) -> bool {
        if self.confirm.take().is_some() {
            cx.notify();
            return true;
        }
        let Some(key) = self.active.clone() else {
            return false;
        };
        let Some(state) = self.details.get_mut(&key) else {
            return false;
        };
        let actions = &mut state.actions;
        let open = actions.more.is_open() && !actions.more.is_closing();
        let labels = actions.labels.popup.is_open() && !actions.labels.popup.is_closing();
        let reviewers = actions.reviewers.popup.is_open() && !actions.reviewers.popup.is_closing();
        if open {
            self.close_more_menu(&key, cx);
            return true;
        }
        if labels || reviewers {
            actions.close_picker(true);
            actions.close_picker(false);
            cx.notify();
            return true;
        }
        if actions.title.take().is_some() || actions.body.take().is_some() {
            cx.notify();
            return true;
        }
        false
    }

    pub(super) fn render_confirm(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let confirm = self.confirm.as_ref()?;
        let popup = theme.for_popup();
        let key = confirm.key.clone();
        let action = confirm.action;
        let dialog = div()
            .id("pull-request-confirm-scrim")
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::hsla(0.0, 0.0, 0.0, 0.4))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                popover::popover_card(&popup)
                    .bg(popup.surface_dialog)
                    .id("pull-request-confirm")
                    .role(gpui::Role::Dialog)
                    .aria_label(SharedString::from(confirm.title.clone()))
                    .w(px(400.0))
                    .p(px(20.0))
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(16.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(popup.text)
                            .child(confirm.title.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(popup.text_muted)
                            .child(confirm.description.clone()),
                    )
                    .child(
                        div()
                            .mt(px(12.0))
                            .flex()
                            .justify_end()
                            .gap(px(8.0))
                            .child(
                                button(
                                    "pull-request-confirm-cancel",
                                    "Cancel",
                                    Look::Outline,
                                    &popup,
                                )
                                .on_click(cx.listener(
                                    |page, _, _, cx| {
                                        page.confirm = None;
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(
                                button(
                                    "pull-request-confirm-ok",
                                    confirm.button.clone(),
                                    if confirm.destructive {
                                        Look::Destructive
                                    } else {
                                        Look::Primary
                                    },
                                    &popup,
                                )
                                .on_click(cx.listener(
                                    move |page, _, _, cx| {
                                        page.confirm = None;
                                        page.run_action(&key, action, None, cx);
                                    },
                                )),
                            ),
                    ),
            );
        Some(gpui::deferred(dialog).with_priority(2).into_any_element())
    }

    /// Auto-merge badge, the primary control and the More menu, right of row A.
    pub(super) fn render_action_cluster(
        &self,
        key: &PullRequestKey,
        item: &ChangeRequestListItem,
        detail: Option<&ChangeRequestDetail>,
        theme: &Theme,
        tones: Tones,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = &self.details[key].actions;
        let permissions = state.permissions.ready();
        let method = self.merge_method(key, cx);
        let primary = primary_control(item, detail, permissions, method);
        let pending = state.pending;
        let auto_merge = detail.and_then(|detail| detail.auto_merge.as_ref());
        let auto_badge = auto_merge.map(|auto| {
            let label = match auto.method {
                Some(method) => format!("Auto-merge ({})", method_word(method)),
                None => "Auto-merge".to_string(),
            };
            div()
                .id("pull-request-auto-merge")
                .h(px(24.0))
                .flex()
                .items_center()
                .gap(px(4.0))
                .px(px(7.0))
                .rounded(px(6.0))
                .bg(theme.accent.opacity(0.1))
                .text_size(px(12.0))
                .text_color(theme.accent)
                .tooltip(text_tooltip(format!(
                    "{label}: the host will merge this on its own once its requirements are met"
                )))
                .child(
                    icon(icons::LUCIDE_GIT_MERGE)
                        .size(px(14.0))
                        .text_color(theme.accent),
                )
                .child(label)
                .into_any_element()
        });
        let badge = |glyph: &'static str, label: &'static str, tone: gpui::Hsla| {
            div()
                .h(px(24.0))
                .flex()
                .items_center()
                .gap(px(4.0))
                .px(px(7.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(theme.hairline(0.09))
                .text_size(px(12.0))
                .text_color(tone)
                .child(icon(glyph).size(px(14.0)).text_color(tone))
                .child(label)
                .into_any_element()
        };
        let primary_key = key.clone();
        let primary_element: Option<AnyElement> = match primary {
            Primary::Nothing => None,
            Primary::Merged => Some(badge(icons::LUCIDE_GIT_MERGE, "Merged", tones.merged)),
            Primary::Closed => Some(badge(
                icons::LUCIDE_GIT_PULL_REQUEST_CLOSED,
                "Closed",
                tones.closed,
            )),
            Primary::AutoMergeArmed => None,
            Primary::Resolve => {
                let busy = self.handoff.as_ref().is_some_and(|(at, kind)| {
                    at == key && *kind == super::handoff::Handoff::ResolveConflicts
                });
                Some(
                    icon_button(
                        "pull-request-resolve",
                        if busy {
                            "Preparing..."
                        } else {
                            "Resolve conflicts"
                        },
                        Look::DestructiveOutline,
                        theme,
                        icons::LUCIDE_TRIANGLE_ALERT,
                        theme.danger,
                        14.0,
                    )
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.start_handoff(
                            &primary_key,
                            super::handoff::Handoff::ResolveConflicts,
                            cx,
                        )
                    }))
                    .into_any_element(),
                )
            }
            Primary::Ready => Some(
                button(
                    "pull-request-ready",
                    if pending == Some("ready") {
                        "Ready for review"
                    } else {
                        "Ready for review"
                    },
                    Look::Primary,
                    theme,
                )
                .when(pending.is_some(), |el| el.opacity(0.64))
                .on_click(cx.listener(move |page, _, _, cx| {
                    page.request_action(&primary_key, ChangeRequestAction::Ready, cx)
                }))
                .into_any_element(),
            ),
            Primary::EnableAutoMerge(method) => Some(
                icon_button(
                    "pull-request-enable-auto-merge",
                    if pending == Some("enable-auto-merge") {
                        "Enabling...".to_string()
                    } else {
                        format!("Auto-merge ({})", method_word(method))
                    },
                    Look::Primary,
                    theme,
                    icons::LUCIDE_GIT_MERGE,
                    theme.bg,
                    14.0,
                )
                .when(pending.is_some(), |el| el.opacity(0.64))
                .on_click(cx.listener(move |page, _, _, cx| {
                    page.request_action(
                        &primary_key,
                        ChangeRequestAction::EnableAutoMerge { method },
                        cx,
                    )
                }))
                .into_any_element(),
            ),
            Primary::Merge(method) => Some(
                button(
                    "pull-request-merge",
                    if pending == Some("merge") {
                        "Merging..."
                    } else {
                        method_label(method)
                    },
                    Look::Primary,
                    theme,
                )
                .when(pending.is_some(), |el| el.opacity(0.64))
                .on_click(cx.listener(move |page, _, _, cx| {
                    page.request_action(&primary_key, ChangeRequestAction::Merge { method }, cx)
                }))
                .into_any_element(),
            ),
        };
        let more = self.render_more_menu(key, item, detail, primary, theme, cx);
        let checkout = (item.state == ChangeRequestState::Open)
            .then(|| self.render_checkout_menu(key, theme, cx));
        div()
            .flex()
            .items_center()
            .gap(px(4.0))
            .flex_none()
            .children(checkout)
            .children(auto_badge)
            .children(primary_element)
            .child(more)
            .into_any_element()
    }

    fn render_more_menu(
        &self,
        key: &PullRequestKey,
        item: &ChangeRequestListItem,
        detail: Option<&ChangeRequestDetail>,
        primary: Primary,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = &self.details[key].actions;
        let press_key = key.clone();
        let click_key = key.clone();
        let trigger = div()
            .id("pull-request-more")
            .role(gpui::Role::Button)
            .aria_label("More pull request actions")
            .relative()
            .size(px(24.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .hover(|style| style.bg(theme.ink(0.06)))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |page, _, _, _| {
                    if let Some(state) = page.details.get_mut(&press_key) {
                        state.actions.more.note_trigger_press();
                    }
                }),
            )
            .on_click(cx.listener(move |page, _, _, cx| {
                let Some(state) = page.details.get_mut(&click_key) else {
                    return;
                };
                if state.actions.more.take_press_was_open() {
                    page.close_more_menu(&click_key, cx);
                } else {
                    state.actions.more.open(());
                    cx.notify();
                }
            }))
            .child(
                icon(icons::LUCIDE_ELLIPSIS)
                    .size(px(16.0))
                    .text_color(theme.text_muted),
            );
        if !state.more.is_open() {
            return trigger.into_any_element();
        }
        let popup = theme.for_popup();
        let permissions = state.permissions.ready();
        let method = self.merge_method(key, cx);
        let mut rows: Vec<AnyElement> = Vec::new();
        let row = |id: &'static str, glyph: &'static str, label: String, sub: Option<&str>| {
            popover::menu_row(&popup, false, id)
                .id(id)
                .role(gpui::Role::MenuItem)
                .aria_label(SharedString::from(label.clone()))
                .child(icon(glyph).size(px(14.0)).text_color(popup.text_muted))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(label)
                        .children(sub.map(|sub| {
                            div()
                                .text_size(px(11.0))
                                .text_color(popup.text_muted)
                                .child(sub.to_owned())
                        })),
                )
        };
        let separator = || popover::menu_separator();
        let refresh_key = key.clone();
        rows.push(
            row(
                "pull-request-more-refresh",
                icons::LUCIDE_REFRESH_CW,
                "Refresh".into(),
                None,
            )
            .on_click(cx.listener(move |page, _, _, cx| {
                page.close_more_menu(&refresh_key, cx);
                page.refresh_pull_request(&refresh_key, cx);
            }))
            .into_any_element(),
        );
        rows.extend(self.handoff_rows(key, theme, cx));
        let open = item.state == ChangeRequestState::Open;
        let allowed = |check: fn(&ChangeRequestPermissions) -> bool| permissions.is_some_and(check);
        if open {
            let mut group: Vec<AnyElement> = Vec::new();
            if primary != Primary::Ready && allowed(|p| p.can_update) {
                let (id, label, action) = if item.is_draft {
                    (
                        "pull-request-more-ready",
                        "Ready for review",
                        ChangeRequestAction::Ready,
                    )
                } else {
                    (
                        "pull-request-more-draft",
                        "Convert to draft",
                        ChangeRequestAction::Draft,
                    )
                };
                let action_key = key.clone();
                group.push(
                    row(id, icons::LUCIDE_GIT_PULL_REQUEST_DRAFT, label.into(), None)
                        .on_click(cx.listener(move |page, _, _, cx| {
                            page.request_action(&action_key, action, cx)
                        }))
                        .into_any_element(),
                );
            }
            let can_merge = allowed(|p| p.can_write && !p.merge_methods.is_empty());
            if matches!(primary, Primary::EnableAutoMerge(_)) && can_merge {
                let action_key = key.clone();
                group.push(
                    row(
                        "pull-request-more-merge",
                        icons::LUCIDE_GIT_MERGE,
                        "Merge now".into(),
                        None,
                    )
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.request_action(&action_key, ChangeRequestAction::Merge { method }, cx)
                    }))
                    .into_any_element(),
                );
            }
            if can_merge && !item.is_draft {
                let armed = detail.is_some_and(|detail| detail.auto_merge.is_some());
                let (id, label, action) = if armed {
                    (
                        "pull-request-more-auto-off",
                        "Disable auto-merge",
                        ChangeRequestAction::DisableAutoMerge,
                    )
                } else {
                    (
                        "pull-request-more-auto-on",
                        "Enable auto-merge",
                        ChangeRequestAction::EnableAutoMerge { method },
                    )
                };
                let action_key = key.clone();
                group.push(
                    row(id, icons::LUCIDE_GIT_MERGE, label.into(), None)
                        .on_click(cx.listener(move |page, _, _, cx| {
                            page.request_action(&action_key, action, cx)
                        }))
                        .into_any_element(),
                );
            }
            let methods = permissions
                .map(|p| p.merge_methods.clone())
                .unwrap_or_default();
            let conflicting = item.mergeability == ChangeRequestMergeability::Conflicting;
            if !group.is_empty() {
                rows.push(separator().into_any_element());
                rows.extend(group);
            }
            if can_merge && methods.len() >= 2 && !item.is_draft && !conflicting {
                rows.push(separator().into_any_element());
                for (known, label, _) in MERGE_METHODS {
                    if !methods.contains(&known) {
                        continue;
                    }
                    let pick_key = key.clone();
                    rows.push(
                        popover::menu_row(
                            &popup,
                            false,
                            SharedString::from(format!("pull-request-method-{label}")),
                        )
                        .id(SharedString::from(format!("pull-request-method-{label}")))
                        .role(gpui::Role::MenuItemRadio)
                        .aria_label(label)
                        .child(label)
                        .child(
                            div()
                                .ml_auto()
                                .size(px(14.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .when(known == method, |el| {
                                    el.child(div().size(px(6.0)).rounded_full().bg(popup.text))
                                }),
                        )
                        .on_click(cx.listener(move |page, _, _, cx| {
                            if let Some(state) = page.details.get_mut(&pick_key) {
                                state.actions.method = Some(known);
                            }
                            crate::settings::update(
                                crate::settings::SavePolicy::Debounced,
                                cx,
                                |settings| settings.pull_requests_merge_method = Some(known),
                            );
                            cx.notify();
                        }))
                        .into_any_element(),
                    );
                }
            }
        }
        rows.push(separator().into_any_element());
        let url = item.url.clone();
        let open_key = key.clone();
        rows.push(
            row(
                "pull-request-more-open",
                icons::LUCIDE_ARROW_UP_RIGHT,
                "Open on GitHub".into(),
                None,
            )
            .on_click(cx.listener(move |page, _, _, cx| {
                page.close_more_menu(&open_key, cx);
                cx.open_url(&url);
            }))
            .into_any_element(),
        );
        let link = item.url.clone();
        let copy_key = key.clone();
        rows.push(
            row(
                "pull-request-more-copy",
                icons::LUCIDE_LINK,
                "Copy link".into(),
                None,
            )
            .on_click(cx.listener(move |page, _, _, cx| {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(link.clone()));
                page.close_more_menu(&copy_key, cx);
            }))
            .into_any_element(),
        );
        let destructive: Option<(
            &'static str,
            &'static str,
            &'static str,
            ChangeRequestAction,
        )> = match item.state {
            ChangeRequestState::Open if allowed(|p| p.can_update) => Some((
                "pull-request-more-close",
                icons::LUCIDE_GIT_PULL_REQUEST_CLOSED,
                "Close pull request",
                ChangeRequestAction::Close,
            )),
            ChangeRequestState::Closed if allowed(|p| p.can_update) => Some((
                "pull-request-more-reopen",
                icons::LUCIDE_GIT_PULL_REQUEST,
                "Reopen pull request",
                ChangeRequestAction::Reopen,
            )),
            ChangeRequestState::Merged if allowed(|p| p.can_write) => Some((
                "pull-request-more-revert",
                icons::LUCIDE_ROTATE_CCW,
                "Revert changes",
                ChangeRequestAction::Revert,
            )),
            _ => None,
        };
        if let Some((id, glyph, label, action)) = destructive {
            rows.push(separator().into_any_element());
            let action_key = key.clone();
            rows.push(
                popover::menu_row(&popup, false, id)
                    .id(id)
                    .role(gpui::Role::MenuItem)
                    .aria_label(label)
                    .text_color(if action == ChangeRequestAction::Close {
                        popup.danger
                    } else {
                        popup.text
                    })
                    .child(icon(glyph).size(px(14.0)).text_color(
                        if action == ChangeRequestAction::Close {
                            popup.danger
                        } else {
                            popup.text_muted
                        },
                    ))
                    .child(label)
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.request_action(&action_key, action, cx)
                    }))
                    .into_any_element(),
            );
        }
        let closing = state.more.closing_since();
        let out_key = key.clone();
        let menu = popover::popover_card(&popup)
            .min_w(px(288.0))
            .flex()
            .flex_col()
            .gap(px(popover::MENU_GAP))
            .on_mouse_down_out(
                cx.listener(move |page, _, _, cx| page.close_more_menu(&out_key, cx)),
            )
            .children(rows)
            .into_any_element();
        trigger
            .child(popover::anchored_menu_below_end(
                "pull-request-more-menu",
                menu,
                closing,
            ))
            .into_any_element()
    }

    pub(super) fn render_title(
        &self,
        key: &PullRequestKey,
        item: &ChangeRequestListItem,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = &self.details[key].actions;
        if let Some(editor) = &state.title {
            let save_key = key.clone();
            let cancel_key = key.clone();
            let saving = state.saving_title;
            return div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(
                    div()
                        .id("pull-request-title-input")
                        .role(gpui::Role::TextInput)
                        .aria_label("Pull request title")
                        .flex_1()
                        .min_w_0()
                        .h(px(28.0))
                        .px(px(8.0))
                        .flex()
                        .items_center()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(theme.hairline(0.14))
                        .text_size(px(14.0))
                        .child(editor.input.clone()),
                )
                .child(
                    button("pull-request-title-cancel", "Cancel", Look::Ghost, theme).on_click(
                        cx.listener(move |page, _, _, cx| {
                            if let Some(state) = page.details.get_mut(&cancel_key) {
                                state.actions.title = None;
                                cx.notify();
                            }
                        }),
                    ),
                )
                .child(
                    button(
                        "pull-request-title-save",
                        if saving { "Saving..." } else { "Save" },
                        Look::Outline,
                        theme,
                    )
                    .on_click(cx.listener(move |page, _, _, cx| page.save_title(&save_key, cx))),
                )
                .into_any_element();
        }
        let editable = can_edit(state.permissions.ready());
        let edit_key = key.clone();
        let title = item.title.clone();
        div()
            .id("pull-request-title")
            .group("pull-request-title")
            .flex()
            .items_center()
            .gap(px(4.0))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .text_size(px(16.0))
                    .line_height(px(22.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.text)
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(item.title.clone()),
            )
            .when(editable, |el| {
                el.child(
                    div()
                        .id("pull-request-title-edit")
                        .role(gpui::Role::Button)
                        .aria_label("Edit title")
                        .size(px(20.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.0))
                        .invisible()
                        .group_hover("pull-request-title", |style| style.visible())
                        .hover(|style| style.bg(theme.ink(0.06)))
                        .cursor_pointer()
                        .on_click(cx.listener(move |page, _, window, cx| {
                            page.start_title_edit(&edit_key, title.clone(), window, cx)
                        }))
                        .child(
                            icon(icons::LUCIDE_PENCIL)
                                .size(px(12.0))
                                .text_color(theme.text_muted),
                        ),
                )
            })
            .into_any_element()
    }

    fn start_title_edit(
        &mut self,
        key: &PullRequestKey,
        title: String,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| {
            let mut input = ComposerInput::with_context("Pull request title", "PaletteSearch", cx);
            input.set_text(title, cx);
            input
        });
        let sub_key = key.clone();
        let sub = cx.subscribe(&input, move |page, _, event, cx| {
            if matches!(
                event,
                ComposerInputEvent::Submitted | ComposerInputEvent::ModifiedSubmitted
            ) {
                page.save_title(&sub_key, cx);
            }
        });
        let handle = gpui::Focusable::focus_handle(input.read(cx), cx);
        window.focus(&handle, cx);
        if let Some(state) = self.details.get_mut(key) {
            state.actions.title = Some(Editor { input, _sub: sub });
        }
        cx.notify();
    }

    fn save_title(&mut self, key: &PullRequestKey, cx: &mut Context<Self>) {
        let Some(state) = self.details.get(key) else {
            return;
        };
        let Some(editor) = &state.actions.title else {
            return;
        };
        let text = editor.input.read(cx).text().trim().to_string();
        let unchanged = state
            .detail
            .ready()
            .map(|detail| detail.item.title == text)
            .unwrap_or(state.seed_title() == text);
        if text.is_empty() || unchanged {
            if let Some(state) = self.details.get_mut(key) {
                state.actions.title = None;
            }
            cx.notify();
            return;
        }
        self.update_fields(key, Some(text), None, cx);
    }

    pub(super) fn description_editor(
        &self,
        key: &PullRequestKey,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let state = &self.details[key].actions;
        let editor = state.body.as_ref()?;
        let save_key = key.clone();
        let cancel_key = key.clone();
        let saving = state.saving_body;
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    div()
                        .id("pull-request-body-input")
                        .role(gpui::Role::TextInput)
                        .aria_label("Pull request description")
                        .min_h(px(120.0))
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(theme.hairline(0.14))
                        .px(px(10.0))
                        .py(px(8.0))
                        .text_size(px(13.0))
                        .child(editor.input.clone()),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.0))
                        .child(
                            button("pull-request-body-cancel", "Cancel", Look::Ghost, theme)
                                .on_click(cx.listener(move |page, _, _, cx| {
                                    if let Some(state) = page.details.get_mut(&cancel_key) {
                                        state.actions.body = None;
                                        cx.notify();
                                    }
                                })),
                        )
                        .child(
                            button(
                                "pull-request-body-save",
                                if saving { "Saving..." } else { "Save" },
                                Look::Outline,
                                theme,
                            )
                            .on_click(
                                cx.listener(move |page, _, _, cx| page.save_body(&save_key, cx)),
                            ),
                        ),
                )
                .into_any_element(),
        )
    }

    pub(super) fn description_pencil(
        &self,
        key: &PullRequestKey,
        body: &str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let state = &self.details[key].actions;
        (can_edit(state.permissions.ready()) && state.body.is_none()).then(|| {
            let edit_key = key.clone();
            let body = body.to_string();
            div()
                .id("pull-request-body-edit")
                .role(gpui::Role::Button)
                .aria_label("Edit description")
                .size(px(20.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.0))
                .cursor_pointer()
                .hover(|style| style.bg(theme.ink(0.06)))
                .on_click(cx.listener(move |page, _, window, cx| {
                    cx.stop_propagation();
                    page.start_body_edit(&edit_key, body.clone(), window, cx)
                }))
                .child(
                    icon(icons::LUCIDE_PENCIL)
                        .size(px(12.0))
                        .text_color(theme.text_muted),
                )
                .into_any_element()
        })
    }

    fn start_body_edit(
        &mut self,
        key: &PullRequestKey,
        body: String,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| {
            let mut input = ComposerInput::new("Describe this pull request", cx);
            input.set_text(body, cx);
            input
        });
        let sub_key = key.clone();
        let sub = cx.subscribe(&input, move |page, _, event, cx| {
            if matches!(event, ComposerInputEvent::ModifiedSubmitted) {
                page.save_body(&sub_key, cx);
            }
        });
        let handle = gpui::Focusable::focus_handle(input.read(cx), cx);
        window.focus(&handle, cx);
        if let Some(state) = self.details.get_mut(key) {
            state.actions.body = Some(Editor { input, _sub: sub });
        }
        cx.notify();
    }

    fn save_body(&mut self, key: &PullRequestKey, cx: &mut Context<Self>) {
        let Some(editor) = self
            .details
            .get(key)
            .and_then(|state| state.actions.body.as_ref())
        else {
            return;
        };
        let text = editor.input.read(cx).text().to_string();
        self.update_fields(key, None, Some(text), cx);
    }

    fn update_fields(
        &mut self,
        key: &PullRequestKey,
        title: Option<String>,
        body: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let Some(state) = self.details.get_mut(key) else {
            return;
        };
        let is_title = title.is_some();
        if is_title {
            state.actions.saving_title = true;
        } else {
            state.actions.saving_body = true;
        }
        let params = serde_json::to_value(UpdateChangeRequestRequest {
            host: key.host.clone(),
            repository: key.repository.clone(),
            number: key.number,
            title,
            body,
        })
        .unwrap_or(serde_json::Value::Null);
        let key = key.clone();
        cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::UPDATE_CHANGE_REQUEST, params)
                .await;
            this.update(cx, |page, cx| {
                if let Some(state) = page.details.get_mut(&key) {
                    if is_title {
                        state.actions.saving_title = false;
                    } else {
                        state.actions.saving_body = false;
                    }
                    if result.is_ok() {
                        if is_title {
                            state.actions.title = None;
                        } else {
                            state.actions.body = None;
                        }
                    }
                }
                match result {
                    Ok(_) => page.refresh_pull_request(&key, cx),
                    Err(error) if is_title => page.toast_with(
                        "The title could not be saved",
                        readable(&error.to_string(), "The host refused the new title."),
                        cx,
                    ),
                    Err(error) => page.toast_with(
                        "Could not save the description",
                        readable(&error.to_string(), "The host refused it."),
                        cx,
                    ),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Amber warning beside the base branch, with the update buttons the reader may use.
    pub(super) fn render_out_of_date(
        &self,
        key: &PullRequestKey,
        item: &ChangeRequestListItem,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let state = &self.details[key].actions;
        let permissions = state.permissions.ready()?;
        let behind = permissions.behind_by.filter(|behind| *behind > 0)?;
        (item.state == ChangeRequestState::Open
            && item.mergeability == ChangeRequestMergeability::Mergeable)
            .then_some(())?;
        let hover_key = key.clone();
        let open = state.stale_open;
        let mut warning = div()
            .id("pull-request-out-of-date")
            .relative()
            .flex_none()
            .cursor_default()
            .on_hover(cx.listener(move |page, hovered: &bool, _, cx| {
                if let Some(state) = page.details.get_mut(&hover_key)
                    && state.actions.stale_open != *hovered
                {
                    state.actions.stale_open = *hovered;
                    cx.notify();
                }
            }))
            .child(
                icon(icons::LUCIDE_TRIANGLE_ALERT)
                    .size(px(14.0))
                    .text_color(theme.warning),
            );
        if open {
            let popup = theme.for_popup();
            let methods: Vec<(bool, &'static str)> = if permissions.can_update_branch {
                vec![(false, "Update branch"), (true, "Update with rebase")]
            } else {
                Vec::new()
            };
            let buttons = methods.into_iter().map(|(rebase, label)| {
                let action_key = key.clone();
                icon_button(
                    if rebase {
                        "pull-request-update-rebase"
                    } else {
                        "pull-request-update-merge"
                    },
                    label,
                    Look::Outline,
                    &popup,
                    icons::LUCIDE_GIT_MERGE,
                    popup.text_muted,
                    12.0,
                )
                .on_click(cx.listener(move |page, _: &ClickEvent, _, cx| {
                    page.request_action(
                        &action_key,
                        ChangeRequestAction::UpdateBranch { rebase },
                        cx,
                    )
                }))
            });
            warning = warning.child(
                gpui::deferred(
                    gpui::anchored().snap_to_window_with_margin(px(8.0)).child(
                        popover::popover_card(&popup)
                            .mt(px(18.0))
                            .max_w(px(320.0))
                            .p(px(12.0))
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .text_size(px(12.0))
                            .font_family(theme.font_sans.clone())
                            .text_color(popup.text)
                            .child(format!(
                                "This branch is out-of-date with {} by {behind} {}.",
                                item.base_ref,
                                if behind == 1 { "commit" } else { "commits" }
                            ))
                            .child(
                                div()
                                    .text_color(popup.text_muted)
                                    .child("Changes can be cleanly merged."),
                            )
                            .child(div().mt(px(4.0)).flex().gap(px(6.0)).children(buttons)),
                    ),
                )
                .with_priority(1),
            );
        }
        Some(warning.into_any_element())
    }

    /// Summary's trailing button while fork workflows wait for approval and the reader may give it.
    pub(super) fn render_approve_workflows(
        &self,
        key: &PullRequestKey,
        detail: &ChangeRequestDetail,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let state = &self.details[key].actions;
        let allowed = state.permissions.ready().is_some_and(|p| p.can_write);
        (allowed && detail.is_cross_repository && pending_workflows(detail) > 0).then(|| {
            let action_key = key.clone();
            icon_button(
                "pull-request-approve-workflows",
                if state.pending == Some("approve-workflows") {
                    "Approving..."
                } else {
                    "Approve workflows to run"
                },
                Look::WarningOutline,
                theme,
                icons::LUCIDE_PLAY,
                theme.warning,
                12.0,
            )
            .on_click(cx.listener(move |page, _, _, cx| {
                page.request_action(&action_key, ChangeRequestAction::ApproveWorkflows, cx)
            }))
            .into_any_element()
        })
    }

    /// `Close with comment` / `Reopen with comment` beside the comment box (merged PRs get neither).
    pub(super) fn comment_box_actions(
        &self,
        key: &PullRequestKey,
        state: ChangeRequestState,
        empty: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let actions = &self.details[key].actions;
        let can_update = actions.permissions.ready().is_some_and(|p| p.can_update);
        let (id, label, busy_label, action, glyph, look) = match state {
            ChangeRequestState::Open if can_update => (
                "pull-request-close-with-comment",
                "Close with comment",
                "Closing...",
                ChangeRequestAction::Close,
                icons::LUCIDE_GIT_PULL_REQUEST_CLOSED,
                Look::DestructiveOutline,
            ),
            ChangeRequestState::Closed if can_update => (
                "pull-request-reopen-with-comment",
                "Reopen with comment",
                "Reopening...",
                ChangeRequestAction::Reopen,
                icons::LUCIDE_ROTATE_CCW,
                Look::Outline,
            ),
            _ => return None,
        };
        let busy = actions.pending == Some(action_key(action));
        let action_key = key.clone();
        Some(
            icon_button(
                id,
                if busy { busy_label } else { label },
                look,
                theme,
                glyph,
                match look {
                    Look::DestructiveOutline => theme.danger,
                    _ => theme.text_muted,
                },
                12.0,
            )
            .when(empty || actions.pending.is_some(), |el| el.opacity(0.64))
            .when(!empty && actions.pending.is_none(), |el| {
                el.on_click(cx.listener(move |page, _, _, cx| {
                    let Some(text) = page
                        .details
                        .get(&action_key)
                        .map(|state| state.activity.composer().read(cx).text().to_string())
                    else {
                        return;
                    };
                    page.run_action(&action_key, action, Some(text), cx);
                }))
            })
            .into_any_element(),
        )
    }

    fn picker_search(
        &mut self,
        key: &PullRequestKey,
        labels: bool,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let placeholder = if labels {
            "Search labels"
        } else {
            "Search people with access"
        };
        let input = cx.new(|cx| ComposerInput::with_context(placeholder, "PaletteSearch", cx));
        let sub = cx.subscribe(&input, |_, _, event, cx| {
            if matches!(event, ComposerInputEvent::Edited) {
                cx.notify();
            }
        });
        let handle = gpui::Focusable::focus_handle(input.read(cx), cx);
        window.focus(&handle, cx);
        if let Some(state) = self.details.get_mut(key) {
            let editor = Some(Editor { input, _sub: sub });
            if labels {
                state.actions.labels.search = editor;
            } else {
                state.actions.reviewers.search = editor;
            }
        }
    }

    fn load_candidates(&mut self, key: &PullRequestKey, labels: bool, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let Some(state) = self.details.get_mut(key) else {
            return;
        };
        if labels && state.actions.labels.candidates.ready().is_none() {
            state.actions.labels.candidates = Loadable::Loading;
        }
        if !labels && state.actions.reviewers.candidates.ready().is_none() {
            state.actions.reviewers.candidates = Loadable::Loading;
        }
        let params = serde_json::to_value(GetChangeRequestRequest {
            host: key.host.clone(),
            repository: key.repository.clone(),
            number: key.number,
        })
        .unwrap_or(serde_json::Value::Null);
        let key = key.clone();
        cx.spawn(async move |this, cx| {
            let method = if labels {
                methods::LIST_CHANGE_REQUEST_LABEL_CANDIDATES
            } else {
                methods::LIST_CHANGE_REQUEST_REVIEWER_CANDIDATES
            };
            let result = engine
                .client()
                .call(method, params)
                .await
                .map_err(|error| readable(&error.to_string(), ""));
            this.update(cx, |page, cx| {
                let Some(state) = page.details.get_mut(&key) else {
                    return;
                };
                if labels {
                    state.actions.labels.candidates = match result.and_then(|value| {
                        serde_json::from_value(value).map_err(|error| error.to_string())
                    }) {
                        Ok(candidates) => Loadable::Ready(candidates),
                        Err(error) => Loadable::Error(error),
                    };
                } else {
                    state.actions.reviewers.candidates = match result.and_then(|value| {
                        serde_json::from_value(value).map_err(|error| error.to_string())
                    }) {
                        Ok(candidates) => Loadable::Ready(candidates),
                        Err(error) => Loadable::Error(error),
                    };
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn set_label(&mut self, key: &PullRequestKey, name: String, add: bool, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let Some(state) = self.details.get_mut(key) else {
            return;
        };
        if state.actions.labels.busy {
            return;
        }
        state.actions.labels.busy = true;
        let params = serde_json::to_value(SetChangeRequestLabelRequest {
            host: key.host.clone(),
            repository: key.repository.clone(),
            number: key.number,
            name: name.clone(),
            add,
        })
        .unwrap_or(serde_json::Value::Null);
        let key = key.clone();
        cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::SET_CHANGE_REQUEST_LABEL, params)
                .await;
            this.update(cx, |page, cx| {
                if let Some(state) = page.details.get_mut(&key) {
                    state.actions.labels.busy = false;
                }
                if let Err(error) = result {
                    page.toast_with(
                        if add {
                            format!("Could not put {name} on")
                        } else {
                            format!("Could not take {name} off")
                        },
                        readable(
                            &error.to_string(),
                            "The host refused it. Check that you have triage access on this repository.",
                        ),
                        cx,
                    );
                }
                page.load_candidates(&key, true, cx);
                page.load_detail(&key, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn set_reviewer(
        &mut self,
        key: &PullRequestKey,
        login: String,
        team: bool,
        add: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let Some(state) = self.details.get_mut(key) else {
            return;
        };
        if state.actions.reviewers.busy {
            return;
        }
        state.actions.reviewers.busy = true;
        let params = serde_json::to_value(SetChangeRequestReviewerRequest {
            host: key.host.clone(),
            repository: key.repository.clone(),
            number: key.number,
            login: login.clone(),
            team,
            add,
        })
        .unwrap_or(serde_json::Value::Null);
        let key = key.clone();
        cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::SET_CHANGE_REQUEST_REVIEWER, params)
                .await;
            this.update(cx, |page, cx| {
                if let Some(state) = page.details.get_mut(&key) {
                    state.actions.reviewers.busy = false;
                }
                match result {
                    Ok(_) => page.toast(
                        if add {
                            format!("Review requested from {login}")
                        } else {
                            format!("Review request to {login} taken back")
                        },
                        cx,
                    ),
                    Err(error) => page.toast_with(
                        if add {
                            format!("Could not ask {login} for a review")
                        } else {
                            format!("Could not take back the review request to {login}")
                        },
                        readable(
                            &error.to_string(),
                            "The host refused it. Check that you have write access on this repository, and that they still have access to it.",
                        ),
                        cx,
                    ),
                }
                page.load_candidates(&key, false, cx);
                page.load_detail(&key, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// The tag / user-plus button at the end of the Labels or Reviewers row and its picker.
    pub(super) fn render_picker(
        &self,
        key: &PullRequestKey,
        labels: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = &self.details[key].actions;
        let permissions = state.permissions.ready();
        let allowed = permissions.is_some_and(|p| if labels { p.can_triage } else { p.can_write });
        let (id, glyph, aria, reason) = if labels {
            (
                "pull-request-labels-picker",
                icons::LUCIDE_TAG,
                "Change labels",
                "Changing labels needs triage access on this repository",
            )
        } else {
            (
                "pull-request-reviewers-picker",
                icons::LUCIDE_USER_PLUS,
                "Request a review",
                "Asking someone to review needs write access on this repository",
            )
        };
        let trigger = div()
            .id(id)
            .role(gpui::Role::Button)
            .aria_label(aria)
            .relative()
            .size(px(22.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .child(icon(glyph).size(px(14.0)).text_color(theme.text_muted));
        if !allowed {
            return trigger
                .opacity(0.64)
                .tooltip(text_tooltip(reason))
                .into_any_element();
        }
        let picker_open = state.picker_is_open(labels);
        let toggle_key = key.clone();
        let trigger = trigger
            .cursor_pointer()
            .hover(|style| style.bg(theme.ink(0.06)))
            .on_click(cx.listener(move |page, _, window, cx| {
                let Some(state) = page.details.get_mut(&toggle_key) else {
                    return;
                };
                if state.actions.picker_is_open(labels) {
                    state.actions.close_picker(labels);
                    cx.notify();
                    return;
                }
                state.actions.open_picker(labels);
                page.picker_search(&toggle_key, labels, window, cx);
                page.load_candidates(&toggle_key, labels, cx);
                cx.notify();
            }));
        if !picker_open {
            return trigger.into_any_element();
        }
        let popup = theme.for_popup();
        let query = if labels {
            state.labels.search.as_ref()
        } else {
            state.reviewers.search.as_ref()
        }
        .map(|editor| editor.input.read(cx).text().trim().to_lowercase())
        .unwrap_or_default();
        let search = if labels {
            state.labels.search.as_ref()
        } else {
            state.reviewers.search.as_ref()
        }
        .map(|editor| editor.input.clone());
        let busy = if labels {
            state.labels.busy
        } else {
            state.reviewers.busy
        };
        let message = |text: String| {
            div()
                .p(px(8.0))
                .text_size(px(12.0))
                .text_color(popup.text_muted)
                .child(text)
                .into_any_element()
        };
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut footer: Option<&'static str> = None;
        if labels {
            match &state.labels.candidates {
                Loadable::Idle | Loadable::Loading => rows.push(picker_skeleton(&popup)),
                Loadable::Error(error) => {
                    rows.push(message(format!("The labels could not be read. {error}")))
                }
                Loadable::Ready(candidates) => {
                    let shown: Vec<_> = candidates
                        .labels
                        .iter()
                        .filter(|label| {
                            query.is_empty()
                                || label.name.to_lowercase().contains(&query)
                                || label
                                    .description
                                    .as_deref()
                                    .is_some_and(|text| text.to_lowercase().contains(&query))
                        })
                        .collect();
                    if shown.is_empty() {
                        rows.push(message(
                            if query.is_empty() {
                                "This repository has no labels."
                            } else {
                                "No label matches that."
                            }
                            .into(),
                        ));
                    }
                    for (index, label) in shown.into_iter().enumerate() {
                        let set_key = key.clone();
                        let name = label.name.clone();
                        let add = !label.applied;
                        rows.push(
                            popover::menu_row(
                                &popup,
                                false,
                                SharedString::from(format!("pr-label-row-{index}")),
                            )
                            .id(("pr-label-row", index))
                            .role(gpui::Role::MenuItemCheckBox)
                            .aria_label(SharedString::from(label.name.clone()))
                            .when(busy, |el| el.opacity(0.64))
                            .text_size(px(12.0))
                            .child(
                                div()
                                    .size(px(8.0))
                                    .flex_none()
                                    .rounded_full()
                                    .bg(label_color(label.color.as_deref(), &popup)),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .flex()
                                    .items_center()
                                    .child(div().flex_none().child(label.name.clone()))
                                    .children(label.description.clone().map(|description| {
                                        div()
                                            .min_w_0()
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .whitespace_nowrap()
                                            .text_color(popup.text_muted)
                                            .child(format!(" · {description}"))
                                    })),
                            )
                            .when(label.applied, |el| {
                                el.child(
                                    icon(icons::LUCIDE_CHECK)
                                        .size(px(14.0))
                                        .text_color(popup.text),
                                )
                            })
                            .when(!busy, |el| {
                                el.on_click(cx.listener(move |page, _, _, cx| {
                                    page.set_label(&set_key, name.clone(), add, cx)
                                }))
                            })
                            .into_any_element(),
                        );
                    }
                    if candidates.truncated {
                        footer = Some(
                            "This repository has more labels than are listed here. Apply the rest on the host.",
                        );
                    }
                }
            }
        } else {
            match &state.reviewers.candidates {
                Loadable::Idle | Loadable::Loading => rows.push(picker_skeleton(&popup)),
                Loadable::Error(error) => rows.push(message(format!(
                    "The people with access could not be read. {error}"
                ))),
                Loadable::Ready(candidates) => {
                    let shown: Vec<_> = candidates
                        .reviewers
                        .iter()
                        .filter(|reviewer| {
                            query.is_empty()
                                || reviewer.login.to_lowercase().contains(&query)
                                || reviewer
                                    .name
                                    .as_deref()
                                    .is_some_and(|name| name.to_lowercase().contains(&query))
                        })
                        .collect();
                    if shown.is_empty() {
                        rows.push(message(
                            if query.is_empty() {
                                "Nobody else has access to this repository."
                            } else {
                                "Nobody with access matches that."
                            }
                            .into(),
                        ));
                    }
                    for (index, reviewer) in shown.into_iter().enumerate() {
                        let set_key = key.clone();
                        let login = reviewer.login.clone();
                        let team = reviewer.team;
                        let add = !reviewer.requested;
                        rows.push(
                            popover::menu_row(
                                &popup,
                                false,
                                SharedString::from(format!("pr-reviewer-row-{index}")),
                            )
                            .id(("pr-reviewer-row", index))
                            .role(gpui::Role::MenuItemCheckBox)
                            .aria_label(SharedString::from(reviewer.login.clone()))
                            .when(busy, |el| el.opacity(0.64))
                            .text_size(px(12.0))
                            .child(avatar(&reviewer.login, &popup))
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .child(reviewer.login.clone()),
                            )
                            .when(reviewer.team, |el| {
                                el.child(div().text_color(popup.text_muted).child("team"))
                            })
                            .when(reviewer.requested, |el| {
                                el.child(
                                    icon(icons::LUCIDE_CHECK)
                                        .size(px(14.0))
                                        .text_color(popup.text),
                                )
                            })
                            .when(!busy, |el| {
                                el.on_click(cx.listener(move |page, _, _, cx| {
                                    page.set_reviewer(&set_key, login.clone(), team, add, cx)
                                }))
                            })
                            .into_any_element(),
                        );
                    }
                    if candidates.truncated {
                        footer = Some(
                            "This repository has more people with access than are listed here. Ask for the rest on the host.",
                        );
                    }
                }
            }
        }
        let close_key = key.clone();
        let card = popover::popover_card(&popup)
            .w(px(288.0))
            .flex()
            .flex_col()
            .on_mouse_down_out(cx.listener(move |page, _, _, cx| {
                if let Some(state) = page.details.get_mut(&close_key) {
                    state.actions.close_picker(labels);
                    cx.notify();
                }
            }))
            .child(
                div().px(px(12.0)).pt(px(10.0)).child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .pb(px(6.0))
                        .border_b_1()
                        .border_color(popup.hairline(0.1))
                        .child(
                            icon(icons::LUCIDE_SEARCH)
                                .size(px(14.0))
                                .text_color(popup.text_muted.opacity(0.55)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(13.0))
                                .children(search),
                        ),
                ),
            )
            .child(
                div()
                    .id(if labels {
                        "pr-labels-picker-list"
                    } else {
                        "pr-reviewers-picker-list"
                    })
                    .max_h(px(288.0))
                    .overflow_y_scroll()
                    .p(px(4.0))
                    .flex()
                    .flex_col()
                    .gap(px(popover::MENU_GAP))
                    .children(rows),
            )
            .children(footer.map(|text| {
                div()
                    .px(px(8.0))
                    .py(px(6.0))
                    .text_size(px(12.0))
                    .text_color(popup.text_muted)
                    .child(text)
            }))
            .into_any_element();
        trigger
            .child(popover::anchored_menu_below(
                if labels {
                    "pr-labels-picker"
                } else {
                    "pr-reviewers-picker"
                },
                card,
                None,
            ))
            .into_any_element()
    }
}

fn picker_skeleton(theme: &Theme) -> AnyElement {
    let bar = theme.text_muted.opacity(0.15);
    div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .p(px(4.0))
        .children([0.6, 0.45, 0.7, 0.5].map(|width| {
            div().h(px(28.0)).flex().items_center().child(
                div()
                    .h(px(10.0))
                    .w(gpui::relative(width))
                    .rounded(px(4.0))
                    .bg(bar),
            )
        }))
        .into_any_element()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Look {
    Primary,
    Outline,
    Ghost,
    Destructive,
    DestructiveOutline,
    WarningOutline,
}

pub(super) fn icon_button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    look: Look,
    theme: &Theme,
    glyph: &'static str,
    tone: gpui::Hsla,
    size: f32,
) -> gpui::Stateful<gpui::Div> {
    styled_button(id, label.into(), look, theme, Some((glyph, tone, size)))
}

pub(super) fn button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    look: Look,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    styled_button(id, label.into(), look, theme, None)
}

fn styled_button(
    id: impl Into<gpui::ElementId>,
    label: SharedString,
    look: Look,
    theme: &Theme,
    leading: Option<(&'static str, gpui::Hsla, f32)>,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .role(gpui::Role::Button)
        .aria_label(label.clone())
        .h(px(24.0))
        .px(px(8.0))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.0))
        .rounded(px(6.0))
        .text_size(px(12.0))
        .cursor_pointer()
        .map(|el| match look {
            Look::Primary => el
                .bg(theme.text)
                .text_color(theme.bg)
                .hover(|style| style.opacity(0.9)),
            Look::Outline => el
                .border_1()
                .border_color(theme.hairline(0.14))
                .text_color(theme.text)
                .hover(|style| style.bg(theme.ink(0.05))),
            Look::Ghost => el
                .text_color(theme.text_muted)
                .hover(|style| style.bg(theme.ink(0.05)).text_color(theme.text)),
            Look::Destructive => el
                .bg(theme.danger)
                .text_color(gpui::hsla(0.0, 0.0, 1.0, 1.0))
                .hover(|style| style.opacity(0.9)),
            Look::DestructiveOutline => el
                .border_1()
                .border_color(theme.danger.opacity(0.32))
                .text_color(theme.danger)
                .hover(|style| style.bg(theme.danger.opacity(0.04))),
            Look::WarningOutline => el
                .border_1()
                .border_color(theme.warning.opacity(0.32))
                .text_color(theme.warning)
                .hover(|style| style.bg(theme.warning.opacity(0.04))),
        })
        .children(leading.map(|(glyph, tone, size)| icon(glyph).size(px(size)).text_color(tone)))
        .child(label)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use zeron_proto::ChangeRequestAutoMerge;

    use super::*;

    fn item(state: ChangeRequestState) -> ChangeRequestListItem {
        let at = chrono::Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap();
        ChangeRequestListItem {
            provider: "github".into(),
            host: "github.com".into(),
            repository: "acme/zeron".into(),
            number: 7,
            title: "Change".into(),
            url: "https://github.com/acme/zeron/pull/7".into(),
            author: None,
            head_ref: "feature".into(),
            base_ref: "main".into(),
            state,
            is_draft: false,
            mergeability: ChangeRequestMergeability::Mergeable,
            additions: 0,
            deletions: 0,
            created_at: at,
            updated_at: at,
            viewer_review_requested: false,
            labels: Vec::new(),
            review_decision: None,
            checks_state: Some(ChangeRequestChecksState::Passing),
        }
    }

    fn detail(item: &ChangeRequestListItem) -> ChangeRequestDetail {
        ChangeRequestDetail {
            item: item.clone(),
            body: String::new(),
            changed_files: 0,
            merged_at: None,
            closed_at: None,
            is_cross_repository: false,
            reviewers: Vec::new(),
            checks: Vec::new(),
            comments: Vec::new(),
            comment_count: 0,
            commit_count: 0,
            commits: Vec::new(),
            auto_merge: None,
            head_owner: None,
        }
    }

    fn writer() -> ChangeRequestPermissions {
        ChangeRequestPermissions {
            merge_methods: vec![ChangeRequestMergeMethod::Squash],
            can_write: true,
            can_triage: true,
            can_update: true,
            ..Default::default()
        }
    }

    #[test]
    fn primary_control_takes_the_first_matching_rule() {
        let squash = ChangeRequestMergeMethod::Squash;
        let open = item(ChangeRequestState::Open);
        let loaded = detail(&open);
        let p = writer();
        assert_eq!(
            primary_control(&item(ChangeRequestState::Merged), None, None, squash),
            Primary::Merged
        );
        assert_eq!(
            primary_control(&item(ChangeRequestState::Closed), None, None, squash),
            Primary::Closed
        );
        let conflicting = ChangeRequestListItem {
            mergeability: ChangeRequestMergeability::Conflicting,
            ..open.clone()
        };
        assert_eq!(
            primary_control(&conflicting, None, Some(&p), squash),
            Primary::Resolve
        );
        let draft = ChangeRequestListItem {
            is_draft: true,
            ..open.clone()
        };
        assert_eq!(
            primary_control(&draft, Some(&loaded), Some(&p), squash),
            Primary::Ready
        );
        let armed = ChangeRequestDetail {
            auto_merge: Some(ChangeRequestAutoMerge::default()),
            ..loaded.clone()
        };
        assert_eq!(
            primary_control(&open, Some(&armed), Some(&p), squash),
            Primary::AutoMergeArmed
        );
        assert_eq!(
            primary_control(&open, Some(&loaded), Some(&p), squash),
            Primary::Merge(squash)
        );
        let pending = ChangeRequestListItem {
            checks_state: Some(ChangeRequestChecksState::Pending),
            ..open.clone()
        };
        assert_eq!(
            primary_control(&pending, Some(&detail(&pending)), Some(&p), squash),
            Primary::EnableAutoMerge(squash)
        );
        let reader = ChangeRequestPermissions {
            can_write: false,
            ..writer()
        };
        assert_eq!(
            primary_control(&open, Some(&loaded), Some(&reader), squash),
            Primary::Nothing
        );
    }

    #[test]
    fn merge_method_prefers_the_pick_then_the_last_then_the_first_allowed() {
        use ChangeRequestMergeMethod::*;
        assert_eq!(
            pick_method(Some(Rebase), Some(Squash), &[Merge, Rebase]),
            Rebase
        );
        assert_eq!(
            pick_method(Some(Squash), Some(Rebase), &[Merge, Rebase]),
            Rebase
        );
        assert_eq!(pick_method(None, Some(Squash), &[Merge, Rebase]), Merge);
        assert_eq!(pick_method(None, None, &[]), Merge);
    }

    #[test]
    fn confirm_copy_and_failures_read_like_t3code() {
        let (title, description, button, destructive) = confirm_copy(
            ChangeRequestAction::Merge {
                method: ChangeRequestMergeMethod::Squash,
            },
            7,
            0,
        )
        .unwrap();
        assert_eq!(
            (
                title.as_str(),
                description.as_str(),
                button.as_str(),
                destructive
            ),
            (
                "Merge pull request?",
                "This merges #7 using squash.",
                "Squash and merge",
                false
            )
        );
        let (_, approve, _, _) = confirm_copy(ChangeRequestAction::ApproveWorkflows, 7, 1).unwrap();
        assert_eq!(
            approve,
            "This allows 1 workflow from #7 to run. Review the code and workflow changes first."
        );
        assert!(confirm_copy(ChangeRequestAction::Ready, 7, 0).is_none());
        assert_eq!(
            confirm_copy(ChangeRequestAction::Close, 7, 0).unwrap().3,
            true
        );
        assert_eq!(readable("  ", "hint"), "hint");
        assert_eq!(readable("command failed", "hint"), "hint");
        assert_eq!(
            readable("Base branch was modified", "hint"),
            "Base branch was modified"
        );
        assert_eq!(readable(&"x".repeat(400), "hint").chars().count(), 320);
    }
}
