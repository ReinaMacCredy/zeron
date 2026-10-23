//! Hand-off buttons on the pull request page; the prompts live in
//! `zeron_proto::pull_request::handoff`.
use super::*;
pub(super) use zeron_proto::pull_request::handoff::VerificationPlatform;
use zeron_proto::pull_request::handoff::{Handoff, checkout_command, handoffs, prompt, repository};

fn handoff_id(kind: Handoff) -> &'static str {
    match kind {
        Handoff::Review => "pr-handoff-review",
        Handoff::AddressFeedback => "pr-handoff-feedback",
        Handoff::FixChecks => "pr-handoff-checks",
        Handoff::ResolveConflicts => "pr-handoff-conflicts",
    }
}

fn handoff_glyph(kind: Handoff) -> &'static str {
    match kind {
        Handoff::Review => crate::icons::EYE,
        Handoff::AddressFeedback => crate::icons::CHAT_ROUND_LINE,
        Handoff::FixChecks => crate::icons::DANGER_TRIANGLE,
        Handoff::ResolveConflicts => crate::icons::PULL_REQUEST,
    }
}

/// Resolve the actual remote on the selected device; registry repository IDs
/// group checkouts but are not necessarily GitHub owner/name identities.
pub(super) async fn matching_checkout(
    engine: &crate::state::EngineHandle,
    spaces: Vec<zeron_proto::Space>,
    device: &str,
    repository: &str,
) -> Result<Option<zeron_proto::Space>, String> {
    let mut failed = false;
    for space in spaces.into_iter().filter(|space| space.device_id == device) {
        let result = engine
            .client()
            .call(
                methods::GET_CHANGE_REQUEST_REPOSITORY,
                serde_json::json!({"cwd": space.path, "targetDeviceId": device, "repository": repository}),
            )
            .await;
        match result.and_then(|value| {
            serde_json::from_value::<Option<String>>(value)
                .map_err(|_| zeron_rpc::RpcError::Failed("invalid repository response".into()))
        }) {
            Ok(Some(repo)) if repo.eq_ignore_ascii_case(repository) => return Ok(Some(space)),
            Ok(_) => {}
            Err(_) => failed = true,
        }
    }
    if failed {
        Err("Couldn’t check this device’s projects. Check its connection and try again.".into())
    } else {
        Ok(None)
    }
}

impl PullRequestDetailPage {
    fn prepare_handoff(&mut self, prompt: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.handoff_task.is_some() {
            return;
        }
        let state = self.state.read(cx);
        let Some(engine) = state.engine().cloned() else {
            self.handoff_error =
                Some("Connect to the pull request’s device to start a session.".into());
            cx.notify();
            return;
        };
        let device = self
            .target
            .clone()
            .or_else(|| state.local_device_id.clone())
            .unwrap_or_else(|| engine.engine_info().device_id.clone());
        let spaces = state.spaces_sorted().into_iter().cloned().collect();
        // The engine's URL for the pull request it actually read, not the
        // link it was opened from.
        let repo = repository(self.detail.as_ref().map_or(&self.url, |detail| &detail.url));
        self.handoff_device = Some(device.clone());
        self.handoff_error = None;
        let executor = cx.background_executor().clone();
        self.handoff_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result =
                find_checkout(engine, spaces, device.clone(), repo.clone(), executor).await;
            let _ = this.update_in(cx, |page, window, cx| {
                page.handoff_task = None;
                match result {
                    Ok(Some(space)) => window.dispatch_action(Box::new(StartPullRequestSession {
                        prompt, project_id: space.id, device: device.clone(), repository: repo.clone(),
                    }), cx),
                    Ok(None) => page.handoff_error = Some(format!(
                        "No checkout of {repo} is available on this device. Add its project, then try again.")),
                    Err(error) => page.handoff_error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Settings-style rows: purpose on the left with the actions trailing
    /// (wrapping below on narrow widths), then the checkout command.
    pub(super) fn handoff_card(
        &self,
        detail: &ChangeRequestDetail,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let command = checkout_command(detail, &self.url);
        let copy = command.clone();
        let buttons = handoffs(detail).into_iter().map(|kind| {
            let prompt = prompt(kind, detail, &self.url, self.verification_platform);
            let id = handoff_id(kind);
            let summary = kind.summary();
            widgets::action_button(theme, widgets::ActionTone::Filled)
                .id(id)
                .debug_selector(move || id.to_owned())
                .role(gpui::Role::Button)
                .aria_label(format!("{} in a new session", kind.label()))
                .aria_description(summary)
                .tab_index(0)
                .flex_none()
                .border_1()
                .border_color(gpui::transparent_black())
                .focus_visible(|style| style.border_2().border_color(theme.accent))
                .child(
                    crate::icons::icon(handoff_glyph(kind))
                        .size(px(14.0))
                        .flex_none()
                        .text_color(theme.text_muted),
                )
                .child(kind.label())
                .tooltip(widgets::text_tooltip(summary))
                .on_click(cx.listener(move |page, _, window, cx| {
                    page.prepare_handoff(prompt.clone(), window, cx)
                }))
        });
        widgets::section_card(theme)
            .mt(px(CARD_GAP))
            .id("pr-handoff")
            .debug_selector(|| "pr-handoff".into())
            .when(self.handoff_task.is_some(), |card| {
                card.child(
                    div()
                        .px(px(16.0))
                        .py(px(12.0))
                        .text_color(theme.text_muted)
                        .child("Finding a checkout…"),
                )
            })
            .children(self.handoff_error.as_ref().map(|error| {
                let device = self.handoff_device.clone();
                div()
                    .px(px(16.0))
                    .py(px(12.0))
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(8.0))
                    .child(error.clone())
                    .children(device.map(|device| {
                        widgets::action_button(theme, widgets::ActionTone::Filled)
                            .id("pr-add-checkout")
                            .role(gpui::Role::Button)
                            .aria_label("Add project on this device")
                            .tab_index(0)
                            .child("Add project")
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(
                                    Box::new(AddPullRequestProject(device.clone())),
                                    cx,
                                )
                            })
                    }))
            }))
            .child(
                widgets::card_row(theme, true)
                    .id("pr-handoff-row")
                    .debug_selector(|| "pr-handoff-row".into())
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(220.0))
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(widgets::row_title(theme, "Hand off to an agent"))
                            .child(
                                div()
                                    .text_size(crate::typography::ui_rems(12.0))
                                    .text_color(theme.text_muted)
                                    .child("Opens a new session with the prompt ready to send."),
                            ),
                    )
                    .child(
                        div()
                            .id("pr-handoff-actions")
                            .debug_selector(|| "pr-handoff-actions".into())
                            // Keep the buttons on one line when the row wraps.
                            .flex_none()
                            .max_w_full()
                            .flex()
                            .flex_wrap()
                            .gap(px(8.0))
                            .children(buttons),
                    ),
            )
            .child(
                widgets::card_row(theme, false)
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(120.0))
                            .child(widgets::row_title(theme, "Verification")),
                    )
                    .child(div().max_w_full().flex().flex_wrap().gap(px(4.0)).children(
                        VerificationPlatform::ALL.into_iter().map(|platform| {
                            let selected = self.verification_platform == platform;
                            let id = format!("pr-verification-{}", platform.label());
                            let selector = id.clone();
                            crate::surface_chrome::tab(SharedString::from(id), selected, theme)
                                .debug_selector(move || selector.clone())
                                .role(gpui::Role::Button)
                                .aria_label(platform.label())
                                .aria_selected(selected)
                                .text_size(crate::typography::ui_rems(12.0))
                                .line_height(crate::typography::ui_rems(16.0))
                                .px(px(10.0))
                                .child(
                                    div()
                                        .debug_selector(move || {
                                            format!("pr-verification-label-{}", platform.label())
                                        })
                                        .child(platform.label()),
                                )
                                .on_click(cx.listener(move |page, _, _, cx| {
                                    page.verification_platform = platform;
                                    cx.notify();
                                }))
                        }),
                    )),
            )
            .child(
                // Same label column and gap as the overview rows.
                widgets::card_row(theme, false)
                    .min_h(px(44.0))
                    .py(px(8.0))
                    .flex_nowrap()
                    .child(
                        div()
                            .w(px(80.0))
                            .flex_none()
                            .child(widgets::row_title(theme, "Checkout")),
                    )
                    .child(
                        div()
                            .id("pr-checkout-command")
                            .debug_selector(|| "pr-checkout-command".into())
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(theme.font_mono.clone())
                            .text_size(crate::typography::ui_rems(12.0))
                            .child(command),
                    )
                    .child(
                        action("pr-copy-checkout", "Copy checkout command", theme).on_click(
                            move |_, _, cx| {
                                cx.stop_propagation();
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                    copy.clone(),
                                ));
                            },
                        ),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn handoff_resolves_only_the_requested_repository_on_the_requested_device() {
        use crate::pull_request_test_support::ScriptedRpc;
        let rpc = ScriptedRpc::new(|method, params| async move {
            assert_eq!(method, methods::GET_CHANGE_REQUEST_REPOSITORY);
            assert_eq!(params["targetDeviceId"], "remote");
            assert!(matches!(
                params["repository"].as_str(),
                Some("a/b" | "missing/repo")
            ));
            zeron_rpc::RpcReply::value(&Some(if params["cwd"] == "/right" {
                "A/B"
            } else {
                "other/repo"
            }))
        });
        let engine = crate::state::EngineHandle::from_test_client(rpc.client());
        let space = |id: &str, device: &str| zeron_proto::Space {
            id: id.into(),
            device_id: device.into(),
            path: format!("/{id}"),
            name: None,
            git_detected: true,
            git_checked_at: None,
            checkout_id: None,
            repository_id: None,
            created_at: chrono::Utc::now(),
        };
        let spaces = vec![
            space("right", "local"),
            space("unrelated", "remote"),
            space("right", "remote"),
        ];
        let found = matching_checkout(&engine, spaces.clone(), "remote", "a/b")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.device_id, "remote");
        assert_eq!(found.path, "/right");
        assert!(
            matching_checkout(&engine, spaces, "remote", "missing/repo")
                .await
                .unwrap()
                .is_none()
        );
    }
}
