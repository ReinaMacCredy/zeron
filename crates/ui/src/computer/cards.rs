use gpui::{
    AnyElement, App, ClickEvent, FontWeight, Hsla, SharedString, Window, div, prelude::*, px,
};
use zeron_engine::computer::engine_seams::{COMPUTER_CONSENT_ALLOW, COMPUTER_CONSENT_DECLINE};
use zeron_engine::computer::manager::COMPUTER_CONTROL_ON_DISCLOSURE;
use zeron_engine::computer::runtime::COMPUTER_ESCAPE_STOP_NOTICE;
use zeron_proto::computer::{ComputerConsentScope, ComputerPermission, ThreadComputerState};
use zeron_proto::{UserInputAnswer, UserInputQuestion};

use crate::appshots::{AppshotCapabilities, CapabilityState};
use crate::theme::Theme;

pub const COMPUTER_SETUP_NOTE: &str = "Computer control needs setup";
pub const COMPUTER_UNAVAILABLE_NOTE: &str = "Computer is unavailable for this turn";
const CAPABILITY_DENIED_CODE: &str = "capability_denied";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputerNotice {
    Disclosure,
    EscapeStop,
    SetupRequired,
    Unavailable,
    ControlDenied,
}

pub fn computer_notice(note: &str) -> Option<ComputerNotice> {
    match note {
        COMPUTER_CONTROL_ON_DISCLOSURE => Some(ComputerNotice::Disclosure),
        COMPUTER_ESCAPE_STOP_NOTICE => Some(ComputerNotice::EscapeStop),
        COMPUTER_SETUP_NOTE => Some(ComputerNotice::SetupRequired),
        COMPUTER_UNAVAILABLE_NOTE => Some(ComputerNotice::Unavailable),
        _ => None,
    }
}

pub fn is_control_denied(is_error: bool, output: Option<&str>) -> bool {
    is_error && output.is_some_and(|output| output.contains(CAPABILITY_DENIED_CODE))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComputerRequest {
    EnableControl,
    OpenSetup(Vec<ComputerPermission>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentOutcome {
    Allowed,
    Declined,
    Cancelled,
}

pub fn consent_question(questions: &[UserInputQuestion]) -> Option<&UserInputQuestion> {
    questions.iter().find(|question| question.is_consent())
}

pub fn consent_scope(question: &UserInputQuestion) -> ComputerConsentScope {
    match question.kind {
        Some(zeron_proto::UserInputKind::Consent { scope }) => scope,
        None => ComputerConsentScope::Task,
    }
}

pub fn consent_outcome(
    resolved: bool,
    answers: Option<&[UserInputAnswer]>,
) -> Option<ConsentOutcome> {
    let labels: Vec<&str> = answers
        .unwrap_or_default()
        .iter()
        .flat_map(|answer| answer.labels.iter().map(String::as_str))
        .collect();
    if labels.contains(&COMPUTER_CONSENT_ALLOW) {
        Some(ConsentOutcome::Allowed)
    } else if !labels.is_empty() {
        Some(ConsentOutcome::Declined)
    } else if resolved {
        Some(ConsentOutcome::Cancelled)
    } else {
        None
    }
}

pub fn consent_answers(question_id: &str, allow: bool) -> Vec<UserInputAnswer> {
    let label = if allow {
        COMPUTER_CONSENT_ALLOW
    } else {
        COMPUTER_CONSENT_DECLINE
    };
    vec![UserInputAnswer {
        question_id: question_id.to_owned(),
        labels: vec![label.to_owned()],
    }]
}

/// The engine's task question opens with the card's own title; drop it.
pub fn consent_detail(scope: ComputerConsentScope, question: &str) -> String {
    let question = question.trim();
    match scope {
        ComputerConsentScope::Task => question
            .strip_prefix(consent_title(scope))
            .map_or(question, str::trim_start)
            .to_owned(),
        ComputerConsentScope::Call => question.to_owned(),
    }
}

pub fn consent_title(scope: ComputerConsentScope) -> &'static str {
    match scope {
        ComputerConsentScope::Task => "Allow Computer for this task?",
        ComputerConsentScope::Call => "Approve this tool call?",
    }
}

pub fn consent_allow_label(scope: ComputerConsentScope) -> &'static str {
    match scope {
        ComputerConsentScope::Task => "Allow Computer for this task",
        ComputerConsentScope::Call => "Approve once",
    }
}

pub fn consent_allow_description(scope: ComputerConsentScope) -> &'static str {
    match scope {
        ComputerConsentScope::Task => {
            "Continue routine desktop actions until this response ends. Stop cancels access. Clipboard reads still ask separately."
        }
        ComputerConsentScope::Call => "Allow just this request",
    }
}

pub fn consent_outcome_label(outcome: ConsentOutcome) -> &'static str {
    match outcome {
        ConsentOutcome::Allowed => "Allowed",
        ConsentOutcome::Declined => "Declined",
        ConsentOutcome::Cancelled => "Cancelled",
    }
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputPauseNotice {
    pub title: String,
    pub body: String,
}

/// No "Check again": Zeron has no UI-facing readiness read; the next state clears it.
pub fn input_pause_notice(state: &ThreadComputerState) -> Option<InputPauseNotice> {
    let pause = state.input_pause.as_ref()?;
    let window = pause
        .window_id
        .as_deref()
        .and_then(|id| state.windows.iter().find(|window| window.id == id));
    let target = window
        .and_then(|window| window.app_name.clone())
        .or_else(|| window.map(|window| window.title.clone()))
        .filter(|target| !target.trim().is_empty())
        .unwrap_or_else(|| "the target window".into());
    Some(InputPauseNotice {
        title: format!("Input paused — return to {target}"),
        body: Some(pause.message.trim())
            .filter(|message| !message.is_empty())
            .unwrap_or("Bring the window onto this desktop, or exit the full-screen app. Continue from the current page once input is available.")
            .into(),
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appshots::{AppshotPlatform, CaptureTarget};

    fn answer(labels: &[&str]) -> Vec<UserInputAnswer> {
        vec![UserInputAnswer {
            question_id: "consent".into(),
            labels: labels.iter().map(|l| l.to_string()).collect(),
        }]
    }

    #[test]
    fn engine_notes_map_to_their_cards_and_nothing_else_does() {
        assert_eq!(
            computer_notice(COMPUTER_CONTROL_ON_DISCLOSURE),
            Some(ComputerNotice::Disclosure)
        );
        assert_eq!(
            computer_notice(COMPUTER_ESCAPE_STOP_NOTICE),
            Some(ComputerNotice::EscapeStop)
        );
        assert_eq!(
            computer_notice("Computer control needs setup"),
            Some(ComputerNotice::SetupRequired)
        );
        assert_eq!(
            computer_notice("Computer is unavailable for this turn"),
            Some(ComputerNotice::Unavailable)
        );
        assert_eq!(computer_notice("Context compacted"), None);
    }

    #[test]
    fn consent_outcome_reads_the_persisted_answer() {
        assert_eq!(consent_outcome(false, None), None);
        assert_eq!(
            consent_outcome(true, Some(&answer(&["Allow"]))),
            Some(ConsentOutcome::Allowed)
        );
        assert_eq!(
            consent_outcome(true, Some(&answer(&["Decline"]))),
            Some(ConsentOutcome::Declined)
        );
        assert_eq!(
            consent_outcome(true, Some(&[])),
            Some(ConsentOutcome::Cancelled)
        );
        assert_eq!(consent_outcome(true, None), Some(ConsentOutcome::Cancelled));
    }

    #[test]
    fn consent_answers_use_the_engine_labels() {
        assert_eq!(consent_answers("consent", true), answer(&["Allow"]));
        assert_eq!(consent_answers("consent", false), answer(&["Decline"]));
        let decision = |allow| {
            zeron_engine::computer::engine_seams::consent_decision(&consent_answers(
                "consent", allow,
            ))
        };
        assert_eq!(
            decision(true),
            zeron_engine::computer::approval_gate::ApprovalDecision::Accept
        );
        assert_eq!(
            decision(false),
            zeron_engine::computer::approval_gate::ApprovalDecision::Decline
        );
    }

    #[test]
    fn the_task_question_drops_its_title_sentence() {
        assert_eq!(
            consent_detail(
                ComputerConsentScope::Task,
                "Allow Computer for this task? The agent wants to run computer_click."
            ),
            "The agent wants to run computer_click."
        );
        assert_eq!(
            consent_detail(ComputerConsentScope::Call, "Allow the agent to run x once?"),
            "Allow the agent to run x once?"
        );
    }

    #[test]
    fn control_denial_is_read_from_the_tool_error() {
        let output = r#"{"error":{"code":"capability_denied","message":"Computer control is off for this turn; no input was sent."}}"#;
        assert!(is_control_denied(true, Some(output)));
        assert!(!is_control_denied(false, Some(output)));
        assert!(!is_control_denied(true, Some("approval_denied")));
        assert!(!is_control_denied(true, None));
    }

    #[test]
    fn an_input_pause_names_its_window_and_clears_with_the_pause() {
        let mut state: ThreadComputerState = serde_json::from_value(serde_json::json!({
            "threadId": "chat-1",
            "version": 1,
            "computerId": "local",
            "windows": [{ "id": "w1", "title": "Invoices", "appName": "Numbers", "focused": false, "minimized": false, "visible": true }],
            "screenSize": { "width": 100, "height": 100 },
            "agentActive": true,
            "controlledByOtherThread": false,
            "availability": { "kind": "available", "backend": "mac" },
            "health": { "status": "connected", "consecutiveFailures": 0, "reconnects": 0, "captureAvailable": true },
            "capabilities": {
                "windows": true, "windowBounds": true, "stacking": false, "capture": true, "input": true,
                "clipboard": false, "focus": true, "raise": true, "ghostCursor": true, "visibleDesktop": true
            }
        }))
        .expect("thread state");
        assert_eq!(input_pause_notice(&state), None);
        state.input_pause = Some(zeron_proto::computer::ComputerInputPause {
            window_id: Some("w1".into()),
            pid: None,
            message: "Screen locked".into(),
        });
        assert_eq!(
            input_pause_notice(&state).unwrap().title,
            "Input paused — return to Numbers"
        );
        assert_eq!(input_pause_notice(&state).unwrap().body, "Screen locked");
        state.input_pause.as_mut().unwrap().window_id = Some("gone".into());
        assert_eq!(
            input_pause_notice(&state).unwrap().title,
            "Input paused — return to the target window"
        );
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
