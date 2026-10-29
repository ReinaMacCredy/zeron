use gpui::{
    AnyElement, App, ClickEvent, FontWeight, Hsla, SharedString, Window, div, prelude::*, px,
};
use zeron_proto::computer::ComputerPermission;
pub use zeron_proto::computer::cards::*;

use crate::appshots::{AppshotCapabilities, CapabilityState};
use crate::theme::Theme;

pub const COMPUTER_PERMISSION_ORDER: [ComputerPermission; 3] = [
    ComputerPermission::Accessibility,
    ComputerPermission::ScreenRecording,
    ComputerPermission::InputMonitoring,
];

pub fn missing_computer_permissions(capabilities: AppshotCapabilities) -> Vec<ComputerPermission> {
    let granted = |state: CapabilityState| {
        matches!(
            state,
            CapabilityState::Ready | CapabilityState::UserSelection
        )
    };
    COMPUTER_PERMISSION_ORDER
        .into_iter()
        .filter(|permission| {
            !granted(match permission {
                ComputerPermission::Accessibility => capabilities.application_text,
                ComputerPermission::ScreenRecording => capabilities.window_capture,
                ComputerPermission::InputMonitoring => capabilities.input_monitoring,
            })
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardTone {
    Info,
    Success,
    Warning,
}

pub struct CardAction {
    pub id: SharedString,
    pub label: SharedString,
    pub primary: bool,
    pub on_click: Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>,
}

fn tone_color(tone: CardTone, theme: &Theme) -> Hsla {
    match tone {
        CardTone::Info => theme.text_muted,
        CardTone::Success => theme.success,
        CardTone::Warning => theme.warning,
    }
}

pub fn computer_event_row(
    icon: &'static str,
    text: SharedString,
    tone: CardTone,
    action: Option<CardAction>,
    theme: &Theme,
) -> AnyElement {
    let color = tone_color(tone, theme);
    div()
        .h(px(26.0))
        .w_full()
        .flex()
        .items_center()
        .gap(px(8.0))
        .text_size(px(12.5))
        .text_color(color)
        .child(
            crate::icons::icon(icon)
                .flex_none()
                .size(px(13.0))
                .text_color(color),
        )
        .child(div().min_w_0().truncate().child(text))
        .when_some(action, |row, action| {
            let on_click = action.on_click;
            let selector = action.id.to_string();
            row.child(
                div()
                    .id(action.id)
                    .debug_selector(move || selector)
                    .flex_none()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text.opacity(0.9))
                    .cursor_pointer()
                    .hover(|s| s.opacity(0.8))
                    .on_click(move |event, window, cx| {
                        cx.stop_propagation();
                        on_click(event, window, cx)
                    })
                    .child(action.label),
            )
        })
        .into_any_element()
}

pub fn computer_card(
    tone: CardTone,
    icon: &'static str,
    title: SharedString,
    lines: Vec<SharedString>,
    actions: Vec<CardAction>,
    status: Option<SharedString>,
    theme: &Theme,
) -> AnyElement {
    let accent = tone_color(tone, theme);
    let tile_bg = match tone {
        CardTone::Info => crate::theme::ink(0.09),
        _ => accent.opacity(0.12),
    };
    let settled = !actions.is_empty() || status.is_some();
    let buttons = actions.into_iter().map(|action| {
        let on_click = action.on_click;
        let selector = action.id.to_string();
        div()
            .id(action.id)
            .debug_selector(move || selector)
            .h(px(26.0))
            .px(px(10.0))
            .flex()
            .items_center()
            .rounded(px(8.0))
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer()
            .map(|button| {
                if action.primary {
                    button
                        .bg(theme.text)
                        .text_color(theme.on_solid)
                        .hover(|s| s.opacity(0.9))
                } else {
                    button
                        .border_1()
                        .border_color(crate::theme::hairline(0.1))
                        .text_color(theme.text.opacity(0.9))
                        .hover(|s| s.bg(crate::theme::ink(0.06)))
                }
            })
            .on_click(move |event, window, cx| {
                cx.stop_propagation();
                on_click(event, window, cx)
            })
            .child(action.label)
    });
    let footer = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(6.0))
        .children(buttons)
        .when_some(status, |footer, status| {
            footer.child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(accent)
                    .child(status),
            )
        });
    div()
        .py(px(4.0))
        .w_full()
        .child(
            div()
                .w_full()
                .flex()
                .items_start()
                .gap(px(10.0))
                .overflow_hidden()
                .rounded(px(10.0))
                .border_1()
                .border_color(crate::theme::hairline(0.08))
                .bg(crate::theme::ink(0.045))
                .p(px(10.0))
                .text_size(px(12.0))
                .line_height(px(17.0))
                .child(
                    div()
                        .flex_none()
                        .size(px(28.0))
                        .rounded(px(8.0))
                        .bg(tile_bg)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(crate::icons::icon(icon).size(px(14.0)).text_color(accent)),
                )
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme.text.opacity(0.9))
                                .child(title),
                        )
                        .children(
                            lines
                                .into_iter()
                                .map(|line| div().text_color(theme.text_muted).child(line)),
                        )
                        .when(settled, |body| body.child(footer.mt(px(6.0)))),
                ),
        )
        .into_any_element()
}

pub struct ApprovalCard {
    pub icon: Option<std::sync::Arc<gpui::Image>>,
    pub title: SharedString,
    pub detail: Option<SharedString>,
    pub risk: Option<&'static str>,
    pub disclosure: bool,
    pub actions: Vec<CardAction>,
    pub status: Option<SharedString>,
}

pub fn approval_card(card: ApprovalCard, theme: &Theme) -> AnyElement {
    let buttons = card.actions.into_iter().map(|action| {
        let on_click = action.on_click;
        let selector = action.id.to_string();
        div()
            .id(action.id)
            .debug_selector(move || selector)
            .h(px(28.0))
            .px(px(12.0))
            .flex()
            .items_center()
            .rounded(px(8.0))
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer()
            .map(|button| {
                if action.primary {
                    button
                        .bg(theme.text)
                        .text_color(theme.on_solid)
                        .hover(|s| s.opacity(0.9))
                } else {
                    button
                        .border_1()
                        .border_color(crate::theme::hairline(0.12))
                        .text_color(theme.text.opacity(0.9))
                        .hover(|s| s.bg(crate::theme::ink(0.06)))
                }
            })
            .on_click(move |event, window, cx| {
                cx.stop_propagation();
                on_click(event, window, cx)
            })
            .child(action.label)
    });
    let icon = match card.icon {
        Some(icon) => gpui::img(icon).size(px(32.0)).into_any_element(),
        None => div()
            .size(px(32.0))
            .rounded(px(8.0))
            .bg(crate::theme::ink(0.09))
            .flex()
            .items_center()
            .justify_center()
            .child(
                crate::icons::icon(crate::icons::COMPUTER_CURSOR)
                    .size(px(16.0))
                    .text_color(theme.text_muted),
            )
            .into_any_element(),
    };
    div()
        .py(px(4.0))
        .w_full()
        .child(
            div()
                .id("computer-approval-card")
                .debug_selector(|| "computer-approval-card".into())
                .w_full()
                .flex()
                .flex_col()
                .overflow_hidden()
                .rounded(px(12.0))
                .border_1()
                .border_color(match card.risk {
                    Some(_) => theme.warning.opacity(0.4),
                    None => crate::theme::hairline(0.1),
                })
                .bg(crate::theme::ink(0.045))
                .text_size(px(12.5))
                .line_height(px(18.0))
                .when_some(card.risk, |body, risk| {
                    body.child(
                        div()
                            .debug_selector(|| "computer-approval-risk".into())
                            .px(px(12.0))
                            .py(px(8.0))
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .bg(theme.warning.opacity(0.12))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.warning)
                                    .child(
                                        crate::icons::icon(crate::icons::DANGER_TRIANGLE)
                                            .size(px(13.0))
                                            .text_color(theme.warning),
                                    )
                                    .child("Elevated Risk"),
                            )
                            .child(div().text_color(theme.text_muted).child(risk)),
                    )
                })
                .child(
                    div()
                        .p(px(12.0))
                        .flex()
                        .items_center()
                        .gap(px(12.0))
                        .child(div().flex_none().child(icon))
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap(px(2.0))
                                .child(
                                    div()
                                        .text_size(px(13.5))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(theme.text)
                                        .child(card.title),
                                )
                                .when_some(card.detail, |column, detail| {
                                    column.child(div().text_color(theme.text_muted).child(detail))
                                }),
                        ),
                )
                .when(card.disclosure, |body| {
                    body.child(
                        div()
                            .debug_selector(|| "computer-approval-disclosure".into())
                            .px(px(12.0))
                            .pb(px(10.0))
                            .text_size(px(11.5))
                            .text_color(theme.text_muted)
                            .child(COMPUTER_FIRST_USE_DISCLOSURE),
                    )
                })
                .child(
                    div()
                        .px(px(12.0))
                        .pb(px(12.0))
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .justify_end()
                        .gap(px(6.0))
                        .when_some(card.status, |footer, status| {
                            footer.child(
                                div()
                                    .flex_1()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme.warning)
                                    .child(status),
                            )
                        })
                        .children(buttons),
                ),
        )
        .into_any_element()
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::appshots::{AppshotPlatform, CaptureTarget};
    use zeron_proto::UserInputAnswer;
    use zeron_proto::computer::{ComputerConsentApp, ComputerConsentScope};

    fn answer(labels: &[&str]) -> Vec<UserInputAnswer> {
        vec![UserInputAnswer {
            question_id: "consent".into(),
            labels: labels.iter().map(|l| l.to_string()).collect(),
        }]
    }

    #[test]
    fn a_left_open_note_names_its_app_in_the_singular_and_the_plural() {
        use zeron_engine::computer::manager::{ComputerLeftOpen, left_open_note};
        for (count, app) in [(1, "Ghostty"), (3, "Google Chrome")] {
            let note = left_open_note(&ComputerLeftOpen {
                app: app.into(),
                count,
            });
            assert_eq!(computer_notice(&note), Some(ComputerNotice::LeftOpen));
            assert_eq!(left_open_app(&note), Some(app));
        }
        assert_eq!(left_open_app("Left 1 Ghostty window open"), Some("Ghostty"));
        assert_eq!(
            left_open_app("Left 2 Ghostty windows open"),
            Some("Ghostty")
        );
        for other in [
            "Left the chat",
            "Left 0 Ghostty windows open",
            "Left two Ghostty windows open",
            "Left 1  window open",
            "Left 1 Ghostty window open now",
        ] {
            assert_eq!(computer_notice(other), None, "{other}");
        }
    }

    #[test]
    fn consent_answers_use_the_engine_labels() {
        use zeron_engine::computer::approval_gate::ApprovalDecision;
        let always = ConsentChoice::AlwaysAllow("Always allow Safari".into());
        assert_eq!(
            consent_answers("consent", &ConsentChoice::Allow),
            answer(&["Allow"])
        );
        assert_eq!(
            consent_answers("consent", &ConsentChoice::Decline),
            answer(&["Deny"])
        );
        assert_eq!(
            consent_answers("consent", &always),
            answer(&["Always allow Safari"])
        );
        let decision = |choice: &ConsentChoice| {
            zeron_engine::computer::engine_seams::consent_decision(&consent_answers(
                "consent", choice,
            ))
        };
        assert_eq!(decision(&ConsentChoice::Allow), ApprovalDecision::Accept);
        assert_eq!(decision(&ConsentChoice::Decline), ApprovalDecision::Decline);
        assert_eq!(decision(&always), ApprovalDecision::AcceptForSession);
        assert_eq!(
            consent_outcome(true, Some(&answer(&["Always allow Safari"]))),
            Some(ConsentOutcome::AlwaysAllowed)
        );
    }

    #[test]
    fn the_codex_consent_buttons_map_to_the_engine_answers() {
        use zeron_engine::computer::approval_gate::ApprovalDecision;
        use zeron_engine::computer::engine_seams::consent_decision;
        let verified = safari(Some("com.apple.Safari"));
        let task = consent_choices(ComputerConsentScope::Task, Some(&verified));
        assert_eq!(
            task.iter()
                .map(|(choice, label)| (consent_choice_key(choice), label.as_str()))
                .collect::<Vec<_>>(),
            [
                ("allow", "Allow this conversation"),
                ("always", "Always allow"),
                ("deny", "Deny")
            ]
        );
        let decisions: Vec<_> = task
            .iter()
            .map(|(choice, _)| consent_decision(&consent_answers("consent", choice)))
            .collect();
        assert_eq!(
            decisions,
            [
                ApprovalDecision::Accept,
                ApprovalDecision::AcceptForSession,
                ApprovalDecision::Decline
            ]
        );
        assert_eq!(
            consent_answers("consent", &task[1].0),
            answer(&["Always allow Safari"]),
            "the button reads Always allow; the engine still gets its own label"
        );
        assert_eq!(
            consent_choices(ComputerConsentScope::Task, Some(&safari(None)))
                .into_iter()
                .map(|(_, label)| label)
                .collect::<Vec<_>>(),
            ["Allow this conversation", "Deny"],
            "Always only for a verified bundle"
        );
        assert_eq!(
            consent_choices(ComputerConsentScope::Call, None)
                .into_iter()
                .map(|(_, label)| label)
                .collect::<Vec<_>>(),
            ["Allow once", "Deny"]
        );
    }

    fn safari(bundle_id: Option<&str>) -> ComputerConsentApp {
        ComputerConsentApp {
            name: "Safari".into(),
            bundle_id: bundle_id.map(str::to_owned),
            pid: Some(42),
        }
    }

    #[test]
    fn missing_permissions_follow_synaras_order() {
        let capabilities = |ax, screen, input| AppshotCapabilities {
            platform: AppshotPlatform::MacOs,
            global_shortcut: CapabilityState::Ready,
            window_capture: screen,
            application_text: ax,
            input_monitoring: input,
            target: CaptureTarget::ActiveWindow,
        };
        use CapabilityState::*;
        assert!(missing_computer_permissions(capabilities(Ready, Ready, Ready)).is_empty());
        assert_eq!(
            missing_computer_permissions(capabilities(
                PermissionRequired,
                Ready,
                PermissionRequired
            )),
            vec![
                ComputerPermission::Accessibility,
                ComputerPermission::InputMonitoring
            ]
        );
        assert_eq!(
            missing_computer_permissions(capabilities(Ready, UserSelection, Unavailable)),
            vec![ComputerPermission::InputMonitoring]
        );
    }
}
