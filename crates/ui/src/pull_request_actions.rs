use super::*;
use crate::composer::{ComposerInput, ComposerInputEvent};
use crate::popover::{self, Loadable, Popup};
use zeron_proto::{
    ChangeRequestControls, ChangeRequestLabelCandidates, ChangeRequestMergeMethod,
    ChangeRequestReviewEvent, ChangeRequestReviewerCandidates,
};
use zeron_rpc::RpcError;

const MERGE_METHODS: [(ChangeRequestMergeMethod, &str, &str, &str); 3] = [
    (
        ChangeRequestMergeMethod::Merge,
        "Merge",
        "merge",
        "Merge commit",
    ),
    (
        ChangeRequestMergeMethod::Squash,
        "Squash and merge",
        "squash",
        "Squash",
    ),
    (
        ChangeRequestMergeMethod::Rebase,
        "Rebase and merge",
        "rebase",
        "Rebase",
    ),
];

const MERGE_HINT: &str = "The host refused the merge. Check that you have write access, that the checks it requires have passed, and that the branch is not conflicting.";
const REFUSED: &str = "The host refused it.";

fn method_entry(method: ChangeRequestMergeMethod) -> (&'static str, &'static str, &'static str) {
    MERGE_METHODS
        .iter()
        .find(|(known, ..)| *known == method)
        .map(|(_, label, word, choice)| (*label, *word, *choice))
        .unwrap_or(("Merge", "merge", "Merge commit"))
}

fn pick_method(
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

fn merge_confirm(
    method: ChangeRequestMergeMethod,
    number: u64,
    base: &str,
) -> (String, String, &'static str) {
    let (label, word, _) = method_entry(method);
    let into = if base.is_empty() {
        String::new()
    } else {
        format!(" into {base}")
    };
    (
        "Merge pull request?".into(),
        format!("This merges #{number}{into} using {word}."),
        label,
    )
}

fn readable(message: &str, hint: &str) -> String {
    let trimmed = message.trim();
    let generic = trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("command failed")
        || trimmed.to_ascii_lowercase().contains("exited with code")
        || trimmed.eq_ignore_ascii_case("unknown error");
    if generic {
        hint.to_owned()
    } else if trimmed.chars().count() > 320 {
        format!("{}…", trimmed.chars().take(319).collect::<String>())
    } else {
        trimmed.to_owned()
    }
}

fn failure_words(error: &RpcError, hint: &str) -> String {
    match error {
        RpcError::Failed(message) => readable(message, hint),
        other => super::failure_reason(other).to_owned(),
    }
}

fn label_color(color: Option<&str>, theme: &Theme) -> gpui::Hsla {
    color
        .map(|hex| hex.trim_start_matches('#'))
        .filter(|hex| hex.len() == 6)
        .and_then(|hex| u32::from_str_radix(hex, 16).ok())
        .map(|rgb| gpui::rgb(rgb).into())
        .unwrap_or(theme.text_muted)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PickerKind {
    Labels,
    Reviewers,
}

struct Reply {
    thread: String,
    input: Entity<ComposerInput>,
    _subscription: Subscription,
}

#[derive(Default)]
pub(super) struct ActionsState {
    pub(super) controls: Loadable<ChangeRequestControls>,
    controls_task: Option<Task<()>>,
    method: Option<ChangeRequestMergeMethod>,
    confirm: Option<(ChangeRequestMergeMethod, gpui::FocusHandle)>,
    merging: bool,
    picker: Popup<PickerKind>,
    search: Option<(Entity<ComposerInput>, Subscription)>,
    labels: Loadable<ChangeRequestLabelCandidates>,
    reviewers: Loadable<ChangeRequestReviewerCandidates>,
    picker_busy: bool,
    review_menu: Popup<()>,
    reviewing: bool,
    reply: Option<Reply>,
    thread_busy: bool,
}

impl ActionsState {
    pub(super) fn overlay_open(&self) -> bool {
        self.confirm.is_some() || self.picker.is_open() || self.review_menu.is_open()
    }
}

impl PullRequestDetailPage {
    pub(super) fn restore_controls(&mut self) {
        let key = (self.target.clone(), self.url.clone());
        if let Some(controls) = self.cache.borrow().controls.get(&key) {
            self.actions.controls = Loadable::Ready(controls.clone());
        }
    }

    pub(super) fn load_controls(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        if self.actions.controls.ready().is_none() {
            self.actions.controls = Loadable::Loading;
        }
        let params = self.params(false);
        let key = (self.target.clone(), self.url.clone());
        self.actions.controls_task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::GET_CHANGE_REQUEST_CONTROLS, params)
                .await
                .map_err(|error| super::failure_reason(&error).to_owned())
                .and_then(|value| {
                    serde_json::from_value::<ChangeRequestControls>(value)
                        .map_err(|_| "Update Zeron on the selected device.".to_owned())
                });
            let _ = this.update(cx, |page, cx| {
                match result {
                    Ok(controls) => {
                        page.cache
                            .borrow_mut()
                            .controls
                            .insert(key, controls.clone());
                        page.actions.controls = Loadable::Ready(controls);
                    }
                    Err(error) if page.actions.controls.ready().is_none() => {
                        page.actions.controls = Loadable::Error(error)
                    }
                    Err(_) => {}
                }
                cx.notify();
            });
        }));
    }

    #[allow(clippy::too_many_arguments)]
    fn write(
        &mut self,
        method: &'static str,
        fields: serde_json::Value,
        success: String,
        failure: String,
        hint: &'static str,
        cx: &mut Context<Self>,
        done: impl FnOnce(&mut Self, bool, &mut Context<Self>) + 'static,
    ) -> bool {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            crate::toast::error(cx, "Connect to the pull request’s device first.");
            return false;
        };
        let mut params = self.params(false);
        if let (Some(params), Some(fields)) = (params.as_object_mut(), fields.as_object()) {
            params.extend(fields.clone());
        }
        let (cache, key) = (self.cache.clone(), (self.target.clone(), self.url.clone()));
        cx.spawn(async move |this, cx| {
            let result = engine.client().call(method, params).await;
            if result.is_ok() {
                let mut cache = cache.borrow_mut();
                cache.controls.remove(&key);
                cache.evict(&key.0, &key.1);
            }
            let ok = result.is_ok();
            let _ = cx.update(|cx| match result {
                Ok(_) => crate::toast::success(cx, success),
                Err(error) => {
                    crate::toast::error(cx, format!("{failure}. {}", failure_words(&error, hint)))
                }
            });
            let _ = this.update(cx, |page, cx| {
                done(page, ok, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
        true
    }

    fn request_merge(
        &mut self,
        method: ChangeRequestMergeMethod,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        self.actions.confirm = Some((method, focus));
        cx.notify();
    }

    fn merge(&mut self, method: ChangeRequestMergeMethod, cx: &mut Context<Self>) {
        self.actions.confirm = None;
        if self.actions.merging {
            return;
        }
        self.cache.borrow_mut().merge_method = Some(method);
        self.actions.merging = self.write(
            methods::MERGE_CHANGE_REQUEST,
            serde_json::json!({ "method": method }),
            "Pull request merged".into(),
            "Couldn’t merge this pull request".into(),
            MERGE_HINT,
            cx,
            |page, ok, cx| {
                page.actions.merging = false;
                if ok {
                    page.load(true, cx);
                }
            },
        );
    }

    pub(super) fn merge_card(
        &self,
        detail: &ChangeRequestDetail,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let controls = self.actions.controls.ready()?;
        if !detail.state.eq_ignore_ascii_case("OPEN")
            || detail.is_draft
            || !controls.can_write
            || controls.merge_methods.is_empty()
        {
            return None;
        }
        let method = pick_method(
            self.actions.method,
            self.cache.borrow().merge_method,
            &controls.merge_methods,
        );
        let conflicting = detail.mergeable.eq_ignore_ascii_case("CONFLICTING");
        let blocked = conflicting || self.actions.merging;
        let (label, ..) = method_entry(method);
        let choices = (controls.merge_methods.len() > 1).then(|| {
            div()
                .flex()
                .flex_wrap()
                .gap(px(4.0))
                .children(controls.merge_methods.iter().map(|&choice| {
                    let selected = choice == method;
                    let (_, word, text) = method_entry(choice);
                    let id = format!("pr-merge-method-{word}");
                    let selector = id.clone();
                    crate::surface_chrome::tab(SharedString::from(id), selected, theme)
                        .debug_selector(move || selector.clone())
                        .role(gpui::Role::Button)
                        .aria_label(text)
                        .aria_selected(selected)
                        .text_size(crate::typography::ui_rems(12.0))
                        .line_height(crate::typography::ui_rems(16.0))
                        .px(px(10.0))
                        .child(text)
                        .on_click(cx.listener(move |page, _, _, cx| {
                            page.actions.method = Some(choice);
                            cx.notify();
                        }))
                }))
        });
        let subtitle = if conflicting {
            "Resolve the conflicts on GitHub before merging.".to_owned()
        } else if detail.base_ref_name.is_empty() {
            "Merges once you confirm.".to_owned()
        } else {
            format!("Into {} once you confirm.", detail.base_ref_name)
        };
        Some(
            widgets::section_card(theme)
                .mt(px(CARD_GAP))
                .id("pr-merge")
                .debug_selector(|| "pr-merge".into())
                .child(
                    widgets::card_row(theme, true)
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(180.0))
                                .flex()
                                .flex_col()
                                .gap(px(2.0))
                                .child(widgets::row_title(theme, "Merge pull request"))
                                .child(
                                    div()
                                        .text_size(crate::typography::ui_rems(12.0))
                                        .text_color(theme.text_muted)
                                        .child(subtitle),
                                ),
                        )
                        .children(choices)
                        .child(
                            widgets::action_button(theme, widgets::ActionTone::Solid)
                                .id("pr-merge-button")
                                .debug_selector(|| "pr-merge-button".into())
                                .role(gpui::Role::Button)
                                .aria_label(label)
                                .tab_index(0)
                                .flex_none()
                                .focus_visible(|style| style.border_2().border_color(theme.accent))
                                .when(blocked, |el| el.opacity(0.4).cursor_default())
                                .child(
                                    crate::icons::icon(crate::icons::LUCIDE_GIT_MERGE)
                                        .size(px(14.0))
                                        .flex_none()
                                        .text_color(theme.on_solid),
                                )
                                .child(if self.actions.merging {
                                    "Merging…"
                                } else {
                                    label
                                })
                                .when(!blocked, |el| {
                                    el.on_click(cx.listener(move |page, _, window, cx| {
                                        page.request_merge(method, window, cx)
                                    }))
                                }),
                        ),
                )
                .into_any_element(),
        )
    }

    pub(super) fn merge_dialog(
        &self,
        window: &mut Window,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (method, focus) = self.actions.confirm.clone()?;
        let detail = self.detail.as_ref()?;
        let (title, body, button) = merge_confirm(method, detail.number, &detail.base_ref_name);
        let card = popover::dialog_card(theme)
            .id("pr-merge-dialog")
            .debug_selector(|| "pr-merge-dialog".into())
            .role(gpui::Role::Dialog)
            .aria_label(title.clone())
            .track_focus(&focus)
            .on_key_down(cx.listener(|page, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    page.actions.confirm = None;
                    cx.notify();
                }
            }))
            .child(popover::dialog_title(theme, &title))
            .child(div().mt(px(6.0)).child(popover::dialog_body(theme, body)))
            .child(
                div()
                    .mt(px(16.0))
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        popover::btn_ghost(theme, "Cancel", "pr-merge-cancel")
                            .id("pr-merge-cancel")
                            .debug_selector(|| "pr-merge-cancel".into())
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.actions.confirm = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        popover::btn_primary(theme, button)
                            .id("pr-merge-confirm")
                            .debug_selector(|| "pr-merge-confirm".into())
                            .on_click(cx.listener(move |page, _, _, cx| page.merge(method, cx))),
                    ),
            )
            .into_any_element();
        Some(popover::modal(
            "pr-merge-modal",
            window.viewport_size(),
            card,
        ))
    }

    fn close_picker(&mut self, cx: &mut Context<Self>) {
        if self.actions.picker.begin_close() {
            self.actions.search = None;
            popover::reap_popup(cx, |page: &mut Self| &mut page.actions.picker);
            cx.notify();
        }
    }

    fn open_picker(&mut self, kind: PickerKind, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| {
            ComposerInput::with_context(
                match kind {
                    PickerKind::Labels => "Search labels",
                    PickerKind::Reviewers => "Search people with access",
                },
                "PaletteSearch",
                cx,
            )
            .with_single_line()
            .with_text_metrics(12.0, 16.0)
        });
        let subscription = cx.subscribe(&input, |_, _, event, cx| {
            if matches!(event, ComposerInputEvent::Edited) {
                cx.notify();
            }
        });
        window.focus(&input.read(cx).focus_handle(cx), cx);
        self.actions.search = Some((input, subscription));
        self.actions.picker.open(kind);
        self.load_candidates(kind, cx);
        cx.notify();
    }

    fn load_candidates(&mut self, kind: PickerKind, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let method = match kind {
            PickerKind::Labels => {
                if self.actions.labels.ready().is_none() {
                    self.actions.labels = Loadable::Loading;
                }
                methods::LIST_CHANGE_REQUEST_LABEL_CANDIDATES
            }
            PickerKind::Reviewers => {
                if self.actions.reviewers.ready().is_none() {
                    self.actions.reviewers = Loadable::Loading;
                }
                methods::LIST_CHANGE_REQUEST_REVIEWER_CANDIDATES
            }
        };
        let params = self.params(false);
        cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(method, params)
                .await
                .map_err(|error| failure_words(&error, REFUSED));
            let _ = this.update(cx, |page, cx| {
                let unreadable = |_| "Update Zeron on the selected device.".to_owned();
                match kind {
                    PickerKind::Labels => {
                        page.actions.labels = match result
                            .and_then(|value| serde_json::from_value(value).map_err(unreadable))
                        {
                            Ok(candidates) => Loadable::Ready(candidates),
                            Err(error) => Loadable::Error(error),
                        }
                    }
                    PickerKind::Reviewers => {
                        page.actions.reviewers = match result
                            .and_then(|value| serde_json::from_value(value).map_err(unreadable))
                        {
                            Ok(candidates) => Loadable::Ready(candidates),
                            Err(error) => Loadable::Error(error),
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_label(&mut self, name: String, add: bool, cx: &mut Context<Self>) {
        if self.actions.picker_busy {
            return;
        }
        self.actions.picker_busy = self.write(
            methods::SET_CHANGE_REQUEST_LABEL,
            serde_json::json!({ "name": name, "add": add }),
            if add {
                format!("Added {name}")
            } else {
                format!("Removed {name}")
            },
            if add {
                format!("Couldn’t add {name}")
            } else {
                format!("Couldn’t remove {name}")
            },
            "The host refused it. Check that you have triage access on this repository.",
            cx,
            |page, _, cx| {
                page.actions.picker_busy = false;
                page.load_controls(cx);
                if page.actions.picker.is_open() {
                    page.load_candidates(PickerKind::Labels, cx);
                }
            },
        );
    }

    fn set_reviewer(&mut self, login: String, team: bool, add: bool, cx: &mut Context<Self>) {
        if self.actions.picker_busy {
            return;
        }
        self.actions.picker_busy = self.write(
            methods::SET_CHANGE_REQUEST_REVIEWER,
            serde_json::json!({ "login": login, "team": team, "add": add }),
            if add {
                format!("Review requested from {login}")
            } else {
                format!("Review request to {login} withdrawn")
            },
            if add {
                format!("Couldn’t ask {login} for a review")
            } else {
                format!("Couldn’t withdraw the review request to {login}")
            },
            "The host refused it. Check that you have write access on this repository, and that they still have access to it.",
            cx,
            |page, _, cx| {
                page.actions.picker_busy = false;
                page.load_controls(cx);
                if page.actions.picker.is_open() {
                    page.load_candidates(PickerKind::Reviewers, cx);
                }
            },
        );
    }

    pub(super) fn overview_rows(&self, theme: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(controls) = self.actions.controls.ready() else {
            return Vec::new();
        };
        let none = || {
            div()
                .text_color(theme.text_muted)
                .child("None")
                .into_any_element()
        };
        let labels = if controls.labels.is_empty() {
            none()
        } else {
            div()
                .flex()
                .flex_wrap()
                .gap(px(4.0))
                .children(controls.labels.iter().map(|label| {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .max_w(px(192.0))
                        .rounded_full()
                        .border_1()
                        .border_color(theme.hairline(0.08))
                        .bg(theme.ink(0.025))
                        .pl(px(6.0))
                        .pr(px(8.0))
                        .py(px(2.0))
                        .text_size(crate::typography::ui_rems(12.0))
                        .child(
                            div()
                                .size(px(8.0))
                                .flex_none()
                                .rounded_full()
                                .bg(label_color(label.color.as_deref(), theme)),
                        )
                        .child(div().min_w_0().truncate().child(label.name.clone()))
                }))
                .into_any_element()
        };
        let reviewers = if controls.reviewers.is_empty() {
            none()
        } else {
            div()
                .flex()
                .flex_wrap()
                .gap(px(8.0))
                .children(
                    controls
                        .reviewers
                        .iter()
                        .enumerate()
                        .map(|(index, reviewer)| {
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .text_size(crate::typography::ui_rems(12.0))
                                .child(crate::pull_request_media::avatar(
                                    if reviewer.team { "" } else { &reviewer.login },
                                    format!("pr-reviewer-{index}").into(),
                                    18.0,
                                    theme,
                                ))
                                .child(reviewer.login.clone())
                        }),
                )
                .into_any_element()
        };
        let mut row = |content: AnyElement, kind: PickerKind| {
            div()
                .w_full()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(div().flex_1().min_w_0().child(content))
                .child(self.picker(kind, controls, theme, cx))
                .into_any_element()
        };
        vec![
            field_row("Labels", row(labels, PickerKind::Labels), false, theme),
            field_row(
                "Reviewers",
                row(reviewers, PickerKind::Reviewers),
                false,
                theme,
            ),
        ]
    }

    fn picker(
        &self,
        kind: PickerKind,
        controls: &ChangeRequestControls,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (id, glyph, aria, allowed, reason) = match kind {
            PickerKind::Labels => (
                "pr-labels-picker",
                crate::icons::LUCIDE_TAG,
                "Change labels",
                controls.can_triage,
                "Changing labels needs triage access on this repository",
            ),
            PickerKind::Reviewers => (
                "pr-reviewers-picker",
                crate::icons::LUCIDE_USER_PLUS,
                "Request a review",
                controls.can_write,
                "Asking someone to review needs write access on this repository",
            ),
        };
        let trigger = widgets::ghost_action(theme)
            .id(id)
            .debug_selector(move || id.to_owned())
            .role(gpui::Role::Button)
            .aria_label(aria)
            .size(px(24.0))
            .flex_none()
            .px_0()
            .py_0()
            .rounded(px(6.0))
            .justify_center()
            .child(
                crate::icons::icon(glyph)
                    .size(px(14.0))
                    .text_color(theme.text_muted),
            );
        if !allowed {
            return trigger
                .opacity(0.5)
                .cursor_default()
                .tooltip(widgets::text_tooltip(reason))
                .into_any_element();
        }
        let trigger = trigger
            .tab_index(0)
            .tooltip(widgets::text_tooltip(aria))
            .focus_visible(|style| style.border_2().border_color(theme.accent))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |page, _, _, _| {
                    page.actions
                        .picker
                        .note_trigger_press_matching(|open| *open == kind)
                }),
            )
            .on_key_down(cx.listener(|page, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" && page.actions.picker.is_open() {
                    cx.stop_propagation();
                    page.close_picker(cx);
                }
            }))
            .on_click(cx.listener(move |page, event, window, cx| {
                cx.stop_propagation();
                let open = if matches!(event, gpui::ClickEvent::Keyboard(_)) {
                    page.actions.picker.as_open() == Some(&kind)
                } else {
                    page.actions.picker.take_press_was_open()
                };
                if open {
                    page.close_picker(cx);
                } else {
                    page.open_picker(kind, window, cx);
                }
            }));
        if self.actions.picker.get() != Some(&kind) {
            return trigger.into_any_element();
        }
        trigger
            .child(popover::anchored_menu_below_end(
                format!("{id}-menu"),
                self.picker_card(kind, theme, cx),
                self.actions.picker.closing_since(),
            ))
            .into_any_element()
    }

    fn picker_card(&self, kind: PickerKind, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let popup = theme.for_popup();
        let query = self
            .actions
            .search
            .as_ref()
            .map(|(input, _)| input.read(cx).text().trim().to_lowercase())
            .unwrap_or_default();
        let busy = self.actions.picker_busy;
        let message = |text: String| {
            div()
                .p(px(8.0))
                .text_size(crate::typography::ui_rems(12.0))
                .text_color(popup.text_muted)
                .child(text)
                .into_any_element()
        };
        let row = |index: usize, name: String, checked: bool| {
            let id = match kind {
                PickerKind::Labels => format!("pr-label-row-{index}"),
                PickerKind::Reviewers => format!("pr-reviewer-row-{index}"),
            };
            let selector = id.clone();
            popover::menu_row(&popup, false, SharedString::from(id.clone()))
                .id(SharedString::from(id))
                .debug_selector(move || selector.clone())
                .role(gpui::Role::MenuItemCheckBox)
                .aria_label(SharedString::from(name))
                .aria_toggled(if checked {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                })
                .text_size(crate::typography::ui_rems(12.0))
                .when(busy, |el| el.opacity(0.6).cursor_default())
        };
        let check = |checked: bool| {
            checked.then(|| {
                crate::icons::icon(crate::icons::LUCIDE_CHECK)
                    .size(px(14.0))
                    .text_color(popup.text)
            })
        };
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut footer = None;
        match kind {
            PickerKind::Labels => match &self.actions.labels {
                Loadable::Idle | Loadable::Loading => rows.push(popover::skeleton_menu_rows(
                    "pr-labels-loading",
                    &popup,
                    4,
                    cx.entity_id(),
                    cx,
                )),
                Loadable::Error(error) => {
                    rows.push(message(format!("The labels couldn’t be read. {error}")))
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
                        let name = label.name.clone();
                        let add = !label.applied;
                        rows.push(
                            row(index, label.name.clone(), label.applied)
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
                                        .truncate()
                                        .child(label.name.clone())
                                        .children(label.description.clone().map(|text| {
                                            div()
                                                .text_color(popup.text_muted)
                                                .child(format!(" · {text}"))
                                        })),
                                )
                                .children(check(label.applied))
                                .when(!busy, |el| {
                                    el.on_click(cx.listener(move |page, _, _, cx| {
                                        page.set_label(name.clone(), add, cx)
                                    }))
                                })
                                .into_any_element(),
                        );
                    }
                    if candidates.truncated {
                        footer = Some(
                            "This repository has more labels than are listed here. Apply the rest on GitHub.",
                        );
                    }
                }
            },
            PickerKind::Reviewers => match &self.actions.reviewers {
                Loadable::Idle | Loadable::Loading => rows.push(popover::skeleton_menu_rows(
                    "pr-reviewers-loading",
                    &popup,
                    4,
                    cx.entity_id(),
                    cx,
                )),
                Loadable::Error(error) => rows.push(message(format!(
                    "The people with access couldn’t be read. {error}"
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
                        let login = reviewer.login.clone();
                        let (team, add) = (reviewer.team, !reviewer.requested);
                        rows.push(
                            row(index, reviewer.login.clone(), reviewer.requested)
                                .child(crate::pull_request_media::avatar(
                                    if reviewer.team { "" } else { &reviewer.login },
                                    format!("pr-reviewer-avatar-{index}").into(),
                                    18.0,
                                    &popup,
                                ))
                                .child(
                                    div()
                                        .min_w_0()
                                        .flex_1()
                                        .truncate()
                                        .child(reviewer.login.clone()),
                                )
                                .when(reviewer.team, |el| {
                                    el.child(div().text_color(popup.text_muted).child("team"))
                                })
                                .children(check(reviewer.requested))
                                .when(!busy, |el| {
                                    el.on_click(cx.listener(move |page, _, _, cx| {
                                        page.set_reviewer(login.clone(), team, add, cx)
                                    }))
                                })
                                .into_any_element(),
                        );
                    }
                    if candidates.truncated {
                        footer = Some(
                            "This repository has more people with access than are listed here. Ask the rest on GitHub.",
                        );
                    }
                }
            },
        }
        popover::popover_card(&popup)
            .w(px(288.0))
            .flex()
            .flex_col()
            .on_mouse_down_out(cx.listener(|page, _, _, cx| page.close_picker(cx)))
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
                            crate::icons::icon(crate::icons::LUCIDE_SEARCH)
                                .size(px(14.0))
                                .text_color(popup.text_muted),
                        )
                        .child(div().flex_1().min_w_0().children(
                            self.actions.search.as_ref().map(|(input, _)| input.clone()),
                        )),
                ),
            )
            .child(
                div()
                    .id(match kind {
                        PickerKind::Labels => "pr-labels-picker-list",
                        PickerKind::Reviewers => "pr-reviewers-picker-list",
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
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(popup.text_muted)
                    .child(text)
            }))
            .into_any_element()
    }

    fn reviewer_is_author(&self) -> bool {
        self.actions
            .controls
            .ready()
            .map(|controls| controls.did_author)
            .or_else(|| {
                self.detail
                    .as_ref()
                    .and_then(|detail| detail.viewer_did_author)
            })
            .unwrap_or(false)
    }

    fn close_review_menu(&mut self, cx: &mut Context<Self>) {
        if self.actions.review_menu.begin_close() {
            popover::reap_popup(cx, |page: &mut Self| &mut page.actions.review_menu);
            cx.notify();
        }
    }

    pub(super) fn submit_review(
        &mut self,
        event: ChangeRequestReviewEvent,
        cx: &mut Context<Self>,
    ) {
        self.close_review_menu(cx);
        let body = self.comment_input.read(cx).text().to_owned();
        if self.actions.reviewing || self.submission.is_some() {
            return;
        }
        if event != ChangeRequestReviewEvent::Approve && body.trim().is_empty() {
            crate::toast::error(cx, "Write a summary first. Only an approval can be empty.");
            return;
        }
        if body.len() > 60_000 {
            crate::toast::error(cx, "The review is too long (maximum 60,000 bytes).");
            return;
        }
        let success = match event {
            ChangeRequestReviewEvent::Comment => "Review submitted",
            ChangeRequestReviewEvent::Approve => "Pull request approved",
            ChangeRequestReviewEvent::RequestChanges => "Changes requested",
        };
        let submitted = body.clone();
        self.actions.reviewing = self.write(
            methods::SUBMIT_CHANGE_REQUEST_REVIEW,
            serde_json::json!({ "event": event, "body": body }),
            success.into(),
            "Couldn’t submit your review".into(),
            REFUSED,
            cx,
            move |page, ok, cx| {
                page.actions.reviewing = false;
                if ok {
                    if page.comment_input.read(cx).text() == submitted {
                        page.comment_input
                            .update(cx, |input, cx| input.set_text("", cx));
                        page.comment_error = None;
                        page.save_draft(cx);
                    }
                    page.load(true, cx);
                }
            },
        );
    }

    pub(super) fn review_trigger(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let detail = self.detail.as_ref()?;
        if !detail.state.eq_ignore_ascii_case("OPEN") {
            return None;
        }
        let empty = self.comment_input.read(cx).text().trim().is_empty();
        let author = self.reviewer_is_author();
        let busy = self.actions.reviewing || self.submission.is_some();
        let trigger = widgets::ghost_action(theme)
            .id("pr-review")
            .debug_selector(|| "pr-review".into())
            .role(gpui::Role::Button)
            .aria_label("Submit a review")
            .aria_expanded(self.actions.review_menu.is_open())
            .tab_index(0)
            .flex_none()
            .min_h(px(28.0))
            .px(px(8.0))
            .py(px(4.0))
            .focus_visible(|style| style.border_2().border_color(theme.accent))
            .when(busy, |el| el.opacity(0.4).cursor_default())
            .child(if self.actions.reviewing {
                "Reviewing…"
            } else {
                "Review"
            })
            .child(
                crate::icons::icon(crate::icons::ALT_ARROW_DOWN)
                    .size(px(12.0))
                    .text_color(theme.text_muted),
            )
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|page, _, _, _| page.actions.review_menu.note_trigger_press()),
            )
            .on_key_down(cx.listener(|page, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" && page.actions.review_menu.is_open() {
                    cx.stop_propagation();
                    page.close_review_menu(cx);
                }
            }));
        let trigger = if busy {
            trigger
        } else {
            trigger.on_click(cx.listener(|page, event, _, cx| {
                cx.stop_propagation();
                let open = if matches!(event, gpui::ClickEvent::Keyboard(_)) {
                    page.actions.review_menu.is_open()
                } else {
                    page.actions.review_menu.take_press_was_open()
                };
                if open {
                    page.close_review_menu(cx);
                } else {
                    page.actions.review_menu.open(());
                    cx.notify();
                }
            }))
        };
        if self.actions.review_menu.get().is_none() {
            return Some(trigger.into_any_element());
        }
        let popup = theme.for_popup();
        let choices = [
            (ChangeRequestReviewEvent::Approve, "Approve", !author),
            (
                ChangeRequestReviewEvent::RequestChanges,
                "Request changes",
                !author,
            ),
            (ChangeRequestReviewEvent::Comment, "Comment as review", true),
        ];
        let menu = popover::popover_card(&popup)
            .id("pr-review-menu")
            .debug_selector(|| "pr-review-menu".into())
            .w(px(240.0))
            .p(px(4.0))
            .flex()
            .flex_col()
            .gap(px(popover::MENU_GAP))
            .on_mouse_down_out(cx.listener(|page, _, _, cx| page.close_review_menu(cx)))
            .children(choices.into_iter().filter(|(.., shown)| *shown).map(
                |(event, label, _)| {
                    let id = format!("pr-review-{}", label.to_lowercase().replace(' ', "-"));
                    let selector = id.clone();
                    let needs_text = event != ChangeRequestReviewEvent::Approve && empty;
                    popover::menu_row(&popup, false, SharedString::from(id.clone()))
                        .id(SharedString::from(id))
                        .debug_selector(move || selector.clone())
                        .role(gpui::Role::MenuItem)
                        .aria_label(label)
                        .child(label)
                        .when(needs_text, |el| {
                            el.opacity(0.5)
                                .cursor_default()
                                .tooltip(widgets::text_tooltip("Write a summary in the box first"))
                        })
                        .when(!needs_text, |el| {
                            el.on_click(cx.listener(move |page, _, _, cx| {
                                page.submit_review(event, cx)
                            }))
                        })
                },
            ))
            .child(
                div()
                    .px(px(8.0))
                    .py(px(6.0))
                    .text_size(crate::typography::ui_rems(11.0))
                    .text_color(popup.text_muted)
                    .child(if author {
                        "Your own pull request can only get a comment review. The text in the box becomes its summary."
                    } else {
                        "The text in the box becomes the review summary."
                    }),
            )
            .into_any_element();
        Some(
            trigger
                .child(popover::anchored_menu_above_end(
                    "pr-review-popup",
                    menu,
                    self.actions.review_menu.closing_since(),
                ))
                .into_any_element(),
        )
    }

    fn start_reply(&mut self, thread: String, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| {
            ComposerInput::with_context("Reply…", crate::composer::MESSAGE_COMPOSER_CONTEXT, cx)
                .with_viewport_height(80.0)
                .with_tab_stop()
        });
        let subscription = cx.subscribe(&input, |page, _, event, cx| match event {
            ComposerInputEvent::Submitted | ComposerInputEvent::ModifiedSubmitted => {
                page.send_reply(cx)
            }
            ComposerInputEvent::Edited => cx.notify(),
            _ => {}
        });
        window.focus(&input.read(cx).focus_handle(cx), cx);
        self.actions.reply = Some(Reply {
            thread,
            input,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn send_reply(&mut self, cx: &mut Context<Self>) {
        let Some(reply) = &self.actions.reply else {
            return;
        };
        let body = reply.input.read(cx).text().to_owned();
        if body.trim().is_empty() || self.actions.thread_busy {
            return;
        }
        let thread = reply.thread.clone();
        self.actions.thread_busy = self.write(
            methods::REPLY_TO_CHANGE_REQUEST_THREAD,
            serde_json::json!({ "threadId": thread, "body": body }),
            "Reply posted".into(),
            "Couldn’t post your reply".into(),
            REFUSED,
            cx,
            move |page, ok, cx| {
                page.actions.thread_busy = false;
                if ok {
                    if page
                        .actions
                        .reply
                        .as_ref()
                        .is_some_and(|reply| reply.thread == thread)
                    {
                        page.actions.reply = None;
                    }
                    page.load(true, cx);
                }
            },
        );
    }

    fn set_resolved(&mut self, thread: String, resolved: bool, cx: &mut Context<Self>) {
        if self.actions.thread_busy {
            return;
        }
        self.actions.thread_busy = self.write(
            methods::SET_CHANGE_REQUEST_THREAD_RESOLVED,
            serde_json::json!({ "threadId": thread, "resolved": resolved }),
            if resolved {
                "Conversation resolved"
            } else {
                "Conversation reopened"
            }
            .into(),
            "Couldn’t update this conversation".into(),
            REFUSED,
            cx,
            |page, ok, cx| {
                page.actions.thread_busy = false;
                if ok {
                    page.load(true, cx);
                }
            },
        );
    }

    pub(super) fn thread_actions(
        &self,
        detail: &ChangeRequestDetail,
        index: usize,
        comment: &zeron_proto::ChangeRequestComment,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let thread = inline_thread(detail, index)?;
        if thread.id.is_empty()
            || !thread
                .comments
                .last()
                .is_some_and(|last| std::ptr::eq(last, comment))
        {
            return None;
        }
        let can_resolve = self
            .actions
            .controls
            .ready()
            .is_some_and(|controls| controls.can_write || controls.did_author);
        let busy = self.actions.thread_busy;
        let editor = self
            .actions
            .reply
            .as_ref()
            .filter(|reply| reply.thread == thread.id);
        let link = |id: String, label: &'static str| {
            let selector = id.clone();
            widgets::ghost_action(theme)
                .id(SharedString::from(id))
                .debug_selector(move || selector.clone())
                .role(gpui::Role::Button)
                .aria_label(label)
                .tab_index(0)
                .min_h(px(24.0))
                .px(px(6.0))
                .py(px(2.0))
                .text_size(crate::typography::ui_rems(12.0))
                .focus_visible(|style| style.border_2().border_color(theme.accent))
                .when(busy, |el| el.opacity(0.4).cursor_default())
                .child(label)
        };
        let mut actions = div().flex().items_center().gap(px(4.0));
        if editor.is_none() {
            let reply_thread = thread.id.clone();
            actions = actions.child(link(format!("pr-thread-reply-{index}"), "Reply").on_click(
                cx.listener(move |page, _, window, cx| {
                    page.start_reply(reply_thread.clone(), window, cx)
                }),
            ));
        }
        if can_resolve {
            let (resolve_thread, resolved) = (thread.id.clone(), thread.is_resolved);
            actions = actions.child(
                link(
                    format!("pr-thread-resolve-{index}"),
                    if resolved { "Unresolve" } else { "Resolve" },
                )
                .when(!busy, |el| {
                    el.on_click(cx.listener(move |page, _, _, cx| {
                        page.set_resolved(resolve_thread.clone(), !resolved, cx)
                    }))
                }),
            );
        }
        let editor = editor.map(|reply| {
            let can_send = !busy && !reply.input.read(cx).text().trim().is_empty();
            div()
                .id("pr-thread-reply-editor")
                .debug_selector(|| "pr-thread-reply-editor".into())
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(popover::dialog_field(
                    reply.input.clone().into_any_element(),
                ))
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.0))
                        .child(
                            popover::btn_ghost(theme, "Cancel", "pr-thread-reply-cancel")
                                .id("pr-thread-reply-cancel")
                                .on_click(cx.listener(|page, _, _, cx| {
                                    page.actions.reply = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            popover::btn_primary(theme, if busy { "Replying…" } else { "Reply" })
                                .id("pr-thread-reply-send")
                                .debug_selector(|| "pr-thread-reply-send".into())
                                .when(!can_send, |el| el.opacity(0.4).cursor_default())
                                .on_click(cx.listener(|page, _, _, cx| page.send_reply(cx))),
                        ),
                )
        });
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(actions)
                .children(editor)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pull_request_test_support::{self as fixture, ScriptedRpc};
    use std::sync::Mutex;

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
    fn merge_confirmation_and_failures_read_in_words() {
        assert_eq!(
            merge_confirm(ChangeRequestMergeMethod::Squash, 7, "main"),
            (
                "Merge pull request?".to_owned(),
                "This merges #7 into main using squash.".to_owned(),
                "Squash and merge"
            )
        );
        assert_eq!(readable("  ", "hint"), "hint");
        assert_eq!(readable("command failed", "hint"), "hint");
        assert_eq!(
            readable("Base branch was modified", "hint"),
            "Base branch was modified"
        );
        assert_eq!(readable(&"x".repeat(400), "hint").chars().count(), 320);
        assert_eq!(
            failure_words(&RpcError::Failed("Not mergeable".into()), "hint"),
            "Not mergeable"
        );
    }

    #[gpui::test]
    fn merging_needs_a_confirmation_and_refreshes_the_pull_request(cx: &mut gpui::TestAppContext) {
        let runtime = fixture::runtime();
        let _guard = runtime.enter();
        fixture::init(cx);
        let calls = std::sync::Arc::new(Mutex::new(Vec::<(String, serde_json::Value)>::new()));
        let rpc = ScriptedRpc::new({
            let calls = calls.clone();
            move |method, params| {
                calls.lock().unwrap().push((method.clone(), params));
                async move {
                    match method.as_str() {
                        methods::GET_CHANGE_REQUEST => {
                            zeron_rpc::RpcReply::value(&ChangeRequestDetail {
                                number: 7,
                                state: "OPEN".into(),
                                base_ref_name: "main".into(),
                                ..Default::default()
                            })
                        }
                        methods::GET_CHANGE_REQUEST_CONTROLS => {
                            zeron_rpc::RpcReply::value(&ChangeRequestControls {
                                merge_methods: vec![
                                    ChangeRequestMergeMethod::Merge,
                                    ChangeRequestMergeMethod::Squash,
                                ],
                                can_write: true,
                                ..Default::default()
                            })
                        }
                        methods::MERGE_CHANGE_REQUEST => {
                            zeron_rpc::RpcReply::value(&serde_json::json!({}))
                        }
                        other => Err(zeron_rpc::RpcError::UnknownMethod(other.into())),
                    }
                }
            }
        });
        let state = cx.update(|cx| fixture::state(cx, Some(rpc.client())));
        let (page, cx) = cx.add_window_view(|window, cx| {
            PullRequestDetailPage::new(
                state,
                "https://github.com/a/b/pull/7".into(),
                Some("device".into()),
                Default::default(),
                None,
                window,
                cx,
            )
        });
        rpc.settle(cx, &runtime, |cx| {
            page.read_with(cx, |page, _| page.actions.controls.ready().is_some())
        });
        assert!(cx.debug_bounds("pr-merge").is_some());
        assert!(cx.debug_bounds("pr-labels-picker").is_some());
        let bounds = cx.debug_bounds("pr-merge-method-squash").unwrap();
        cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        let bounds = cx.debug_bounds("pr-merge-button").unwrap();
        cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(
            calls
                .lock()
                .unwrap()
                .iter()
                .all(|(method, _)| method != methods::MERGE_CHANGE_REQUEST),
            "merging waits for the confirmation"
        );
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert!(cx.debug_bounds("pr-merge-dialog").is_none());
        let bounds = cx.debug_bounds("pr-merge-button").unwrap();
        cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        let bounds = cx.debug_bounds("pr-merge-confirm").unwrap();
        cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        let loads = |calls: &Mutex<Vec<(String, serde_json::Value)>>| {
            calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(method, _)| method == methods::GET_CHANGE_REQUEST)
                .count()
        };
        rpc.settle(cx, &runtime, |cx| {
            loads(&calls) == 2 && page.read_with(cx, |page, _| !page.loading)
        });
        let calls = calls.lock().unwrap();
        let (_, merge) = calls
            .iter()
            .find(|(method, _)| method == methods::MERGE_CHANGE_REQUEST)
            .unwrap();
        assert_eq!(merge["method"], "squash");
        assert_eq!(merge["url"], "https://github.com/a/b/pull/7");
        assert_eq!(merge["targetDeviceId"], "device");
        cx.update(|_, cx| {
            assert!(
                crate::toast::messages(cx)
                    .iter()
                    .any(|message| message == "Pull request merged")
            )
        });
    }
}
