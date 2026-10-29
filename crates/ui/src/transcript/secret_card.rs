use super::*;
use crate::composer::{ComposerInput, ComposerInputEvent};
use gpui::{Focusable, FontWeight, HighlightStyle, InteractiveText, StyledText, UnderlineStyle};
use zeron_proto::view::SecretStatus;

const KEPT_OUT: &str =
    "Kept out of chat, in a private file this agent's commands can read for 24 hours";

#[derive(Default)]
pub(super) struct SecretCards {
    inputs: HashMap<SharedString, (Entity<ComposerInput>, Subscription)>,
    sent: HashMap<SharedString, SecretStatus>,
    busy: HashSet<SharedString>,
    failures: HashMap<SharedString, SharedString>,
}

impl Transcript {
    fn secret_input(
        &mut self,
        request_id: &SharedString,
        placeholder: Option<SharedString>,
        cx: &mut Context<Self>,
    ) -> Entity<ComposerInput> {
        if let Some((input, _)) = self.secrets.inputs.get(request_id) {
            return input.clone();
        }
        let placeholder = placeholder.unwrap_or_else(|| "Paste the secret".into());
        let input = cx.new(|cx| {
            ComposerInput::new(placeholder, cx)
                .with_single_line()
                .with_masked()
                .with_tab_stop()
                .with_text_metrics(13.0, 18.0)
        });
        let id = request_id.clone();
        let events = cx.subscribe(&input, move |this: &mut Self, _, event, cx| match event {
            ComposerInputEvent::Submitted => this.answer_secret(id.clone(), true, cx),
            ComposerInputEvent::Edited => cx.notify(),
            _ => {}
        });
        self.secrets
            .inputs
            .insert(request_id.clone(), (input.clone(), events));
        input
    }

    fn answer_secret(&mut self, request_id: SharedString, save: bool, cx: &mut Context<Self>) {
        if self.secrets.busy.contains(&request_id) || self.secrets.sent.contains_key(&request_id) {
            return;
        }
        let Some(chat_id) = self.chat_id.clone() else {
            return;
        };
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let secret = match save {
            true => {
                let Some((input, _)) = self.secrets.inputs.get(&request_id) else {
                    return;
                };
                let text = input.read(cx).text().trim().to_owned();
                if text.is_empty() {
                    return;
                }
                Some(text)
            }
            false => None,
        };
        self.secrets.busy.insert(request_id.clone());
        self.secrets.failures.remove(&request_id);
        cx.notify();
        let params = serde_json::json!({
            "chatId": chat_id.to_string(),
            "requestId": request_id.to_string(),
            "secret": secret,
        });
        cx.spawn(async move |this, cx| {
            let sent = engine
                .client()
                .call(zeron_rpc::methods::ANSWER_SECRET, params)
                .await
                .map_err(|err| err.to_string());
            this.update(cx, |transcript, cx| {
                transcript.secrets.busy.remove(&request_id);
                match sent {
                    Ok(_) => {
                        let status = if save {
                            SecretStatus::Saved
                        } else {
                            SecretStatus::Declined
                        };
                        transcript.secrets.sent.insert(request_id.clone(), status);
                        transcript.secrets.inputs.remove(&request_id);
                    }
                    Err(err) => {
                        transcript
                            .secrets
                            .failures
                            .insert(request_id.clone(), engine_error(&err).into());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_secret_request(
        &mut self,
        request_id: SharedString,
        label: SharedString,
        reason: SharedString,
        placeholder: Option<SharedString>,
        status: SecretStatus,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let status = match status {
            SecretStatus::Pending => self
                .secrets
                .sent
                .get(&request_id)
                .copied()
                .unwrap_or(SecretStatus::Pending),
            status => status,
        };
        if status != SecretStatus::Pending {
            self.secrets.inputs.remove(&request_id);
            return crate::computer::cards::computer_event_row(
                crate::icons::LUCIDE_SHIELD,
                status.label().into(),
                match status {
                    SecretStatus::Saved => crate::computer::cards::CardTone::Success,
                    _ => crate::computer::cards::CardTone::Info,
                },
                None,
                theme,
            );
        }
        let input = self.secret_input(&request_id, placeholder, cx);
        let empty = input.read(cx).text().trim().is_empty();
        let focused = input.focus_handle(cx).is_focused(window);
        let busy = self.secrets.busy.contains(&request_id);
        let failure = self.secrets.failures.get(&request_id).cloned();
        let save_id = request_id.clone();
        let decline_id = request_id.clone();
        let disabled = empty || busy;
        let focus = input.focus_handle(cx);
        div()
            .py(px(4.0))
            .w_full()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(crate::computer::cards::computer_event_row(
                crate::icons::LUCIDE_SHIELD,
                SecretStatus::Pending.label().into(),
                crate::computer::cards::CardTone::Info,
                None,
                theme,
            ))
            .child(
                div()
                    .id(("secret-card", fnv1a(request_id.as_bytes())))
                    .debug_selector(|| "secret-card".into())
                    .role(gpui::Role::Group)
                    .aria_label(label.clone())
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .rounded(px(14.0))
                    .border_1()
                    .border_color(crate::theme::hairline(0.1))
                    .bg(crate::theme::ink(0.045))
                    .p(px(16.0))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .child(
                                div()
                                    .text_size(px(14.0))
                                    .line_height(px(20.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.text)
                                    .child(label),
                            )
                            .child(linked_text(&request_id, reason, theme)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .id("secret-field")
                                    .debug_selector(|| "secret-field".into())
                                    .flex_1()
                                    .min_w_0()
                                    .h(px(34.0))
                                    .px(px(10.0))
                                    .flex()
                                    .items_center()
                                    .rounded(px(8.0))
                                    .border_1()
                                    .border_color(if focused {
                                        theme.accent
                                    } else {
                                        crate::theme::hairline(0.14)
                                    })
                                    .bg(crate::theme::ink(0.03))
                                    .cursor_text()
                                    .on_click(move |_, window, cx| window.focus(&focus, cx))
                                    .child(div().flex_1().min_w_0().child(input)),
                            )
                            .child(
                                div()
                                    .id("secret-save")
                                    .debug_selector(|| "secret-save".into())
                                    .role(gpui::Role::Button)
                                    .aria_label("Save securely")
                                    .aria_disabled(disabled)
                                    .flex_none()
                                    .h(px(34.0))
                                    .px(px(14.0))
                                    .flex()
                                    .items_center()
                                    .rounded(px(8.0))
                                    .bg(theme.accent_strong)
                                    .text_color(theme.on_accent)
                                    .text_size(px(13.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .when(disabled, |el| el.opacity(0.5))
                                    .when(!disabled, |el| {
                                        el.cursor_pointer().hover(|s| s.opacity(0.9)).on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.answer_secret(save_id.clone(), true, cx)
                                            }),
                                        )
                                    })
                                    .child("Save securely"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(12.0))
                            .text_size(px(12.0))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(6.0))
                                    .text_color(theme.text_muted)
                                    .child(
                                        crate::icons::icon(crate::icons::LUCIDE_SHIELD)
                                            .flex_none()
                                            .size(px(13.0))
                                            .text_color(theme.text_muted),
                                    )
                                    .child(match failure {
                                        Some(failure) => div()
                                            .text_color(theme.warning)
                                            .child(failure)
                                            .into_any_element(),
                                        None => div().child(KEPT_OUT).into_any_element(),
                                    }),
                            )
                            .child(
                                div()
                                    .id("secret-decline")
                                    .debug_selector(|| "secret-decline".into())
                                    .role(gpui::Role::Button)
                                    .aria_label("Decline")
                                    .flex_none()
                                    .px(px(6.0))
                                    .h(px(24.0))
                                    .flex()
                                    .items_center()
                                    .rounded(px(6.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_size(px(12.5))
                                    .text_color(theme.text_muted)
                                    .when(!busy, |el| {
                                        el.cursor_pointer()
                                            .hover(|s| s.text_color(theme.text))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.answer_secret(decline_id.clone(), false, cx)
                                            }))
                                    })
                                    .child("Decline"),
                            ),
                    ),
            )
            .into_any_element()
    }
}

fn engine_error(err: &str) -> String {
    let err = err.trim();
    match err.find(": ") {
        Some(at) if err[..at].chars().all(|c| c.is_alphanumeric() || c == ' ') => {
            err[at + 2..].to_owned()
        }
        _ => err.to_owned(),
    }
}

fn link_ranges(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    let mut from = 0;
    while let Some(at) = ["https://", "http://"]
        .iter()
        .filter_map(|scheme| text[from..].find(scheme))
        .min()
    {
        let start = from + at;
        let end = text[start..]
            .find(char::is_whitespace)
            .map_or(text.len(), |len| start + len);
        let end = start
            + text[start..end]
                .trim_end_matches(['.', ',', ')', ';', ':', '"', '\''])
                .len();
        ranges.push(start..end);
        from = end.max(start + 1);
    }
    ranges
}

fn linked_text(id: &SharedString, text: SharedString, theme: &Theme) -> AnyElement {
    let ranges = link_ranges(&text);
    let style = HighlightStyle {
        color: Some(theme.accent),
        underline: Some(UnderlineStyle {
            thickness: px(1.0),
            color: Some(theme.accent.opacity(0.5)),
            wavy: false,
        }),
        ..Default::default()
    };
    let urls: Vec<String> = ranges.iter().map(|r| text[r.clone()].to_owned()).collect();
    let styled = StyledText::new(text.clone())
        .with_highlights(ranges.iter().map(|range| (range.clone(), style)));
    div()
        .text_size(px(13.0))
        .line_height(px(19.0))
        .text_color(theme.text_muted)
        .child(
            InteractiveText::new(("secret-reason", fnv1a(id.as_bytes())), styled).on_click(
                ranges,
                move |ix, _, cx| {
                    if let Some(url) = urls.get(ix) {
                        cx.open_url(url);
                    }
                },
            ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_ranges_cover_urls_without_trailing_punctuation() {
        let text =
            "Create it at https://login.tailscale.com/admin/settings/keys. Then http://x.dev, done";
        let found: Vec<&str> = link_ranges(text).into_iter().map(|r| &text[r]).collect();
        assert_eq!(
            found,
            [
                "https://login.tailscale.com/admin/settings/keys",
                "http://x.dev"
            ]
        );
        assert!(link_ranges("no links here").is_empty());
    }
}
