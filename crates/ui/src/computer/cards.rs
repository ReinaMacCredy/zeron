use gpui::{
    AnyElement, App, ClickEvent, FontWeight, Hsla, SharedString, Window, div, prelude::*, px,
};
use zeron_engine::computer::engine_seams::{
    COMPUTER_CONSENT_ALLOW, COMPUTER_CONSENT_ALWAYS_PREFIX, COMPUTER_CONSENT_DECLINE,
    consent_always_label, consent_app_title,
};
use zeron_engine::computer::manager::{
    COMPUTER_CONTROL_ON_DISCLOSURE, COMPUTER_USER_PAUSE_NOTE_PREFIX,
};
use zeron_engine::computer::runtime::{
    COMPUTER_DESKTOP_REVOKED_NOTE, COMPUTER_ESCAPE_STOP_NOTICE, RevokeSource,
};
use zeron_proto::computer::{ComputerConsentApp, ComputerConsentScope, ComputerPermission};
use zeron_proto::{UserInputAnswer, UserInputQuestion};

use crate::appshots::{AppshotCapabilities, CapabilityState};
use crate::theme::Theme;

pub const COMPUTER_SETUP_NOTE: &str = "Computer control needs setup";
pub const COMPUTER_UNAVAILABLE_NOTE: &str = "Computer is unavailable for this turn";
const CAPABILITY_DENIED_CODE: &str = "capability_denied";
const LEGACY_CONTROL_ON_NOTE: &str = "Computer control ON for this turn: the agent is driving the desktop and the user can switch it off in Settings.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputerNotice {
    Disclosure,
    Revoked,
    Paused,
    SetupRequired,
    Unavailable,
    ControlDenied,
}

pub fn computer_notice(note: &str) -> Option<ComputerNotice> {
    if note
        .strip_prefix(COMPUTER_USER_PAUSE_NOTE_PREFIX)
        .is_some_and(|app| !app.is_empty())
    {
        return Some(ComputerNotice::Paused);
    }
    match note {
        COMPUTER_CONTROL_ON_DISCLOSURE | LEGACY_CONTROL_ON_NOTE => Some(ComputerNotice::Disclosure),
        COMPUTER_DESKTOP_REVOKED_NOTE | COMPUTER_ESCAPE_STOP_NOTICE => {
            Some(ComputerNotice::Revoked)
        }
        COMPUTER_SETUP_NOTE => Some(ComputerNotice::SetupRequired),
        COMPUTER_UNAVAILABLE_NOTE => Some(ComputerNotice::Unavailable),
        _ => None,
    }
}

pub fn revoked_label(text: &str, hint: Option<&str>) -> String {
    let escape = text == COMPUTER_ESCAPE_STOP_NOTICE || hint == Some(RevokeSource::Escape.hint());
    let suffix = if escape { " (Esc)" } else { "" };
    format!("{COMPUTER_DESKTOP_REVOKED_NOTE}{suffix}")
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
    AlwaysAllowed,
    Declined,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsentChoice {
    Allow,
    AlwaysAllow(String),
    Decline,
}

impl ConsentChoice {
    fn answer_label(&self) -> &str {
        match self {
            Self::Allow => COMPUTER_CONSENT_ALLOW,
            Self::AlwaysAllow(label) => label,
            Self::Decline => COMPUTER_CONSENT_DECLINE,
        }
    }

    pub fn outcome(&self) -> ConsentOutcome {
        match self {
            Self::Allow => ConsentOutcome::Allowed,
            Self::AlwaysAllow(_) => ConsentOutcome::AlwaysAllowed,
            Self::Decline => ConsentOutcome::Declined,
        }
    }
}

pub fn consent_question(questions: &[UserInputQuestion]) -> Option<&UserInputQuestion> {
    questions.iter().find(|question| question.is_consent())
}

pub fn consent_app(question: &UserInputQuestion) -> Option<ComputerConsentApp> {
    match &question.kind {
        Some(zeron_proto::UserInputKind::Consent { app, .. }) => app.clone(),
        None => None,
    }
}

pub fn consent_scope(question: &UserInputQuestion) -> ComputerConsentScope {
    match &question.kind {
        Some(zeron_proto::UserInputKind::Consent { scope, .. }) => *scope,
        None => ComputerConsentScope::Task,
    }
}

fn consent_action(question: &UserInputQuestion) -> Option<(&str, Option<serde_json::Value>)> {
    match &question.kind {
        Some(zeron_proto::UserInputKind::Consent {
            tool: Some(tool),
            params,
            ..
        }) => Some((
            tool.as_str(),
            params
                .as_deref()
                .and_then(|params| serde_json::from_str(params).ok()),
        )),
        _ => None,
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
    } else if labels
        .iter()
        .any(|label| label.starts_with(COMPUTER_CONSENT_ALWAYS_PREFIX))
    {
        Some(ConsentOutcome::AlwaysAllowed)
    } else if !labels.is_empty() {
        Some(ConsentOutcome::Declined)
    } else if resolved {
        Some(ConsentOutcome::Cancelled)
    } else {
        None
    }
}

pub fn consent_answers(question_id: &str, choice: &ConsentChoice) -> Vec<UserInputAnswer> {
    vec![UserInputAnswer {
        question_id: question_id.to_owned(),
        labels: vec![choice.answer_label().to_owned()],
    }]
}

pub fn consent_detail(question: &UserInputQuestion) -> String {
    let app = consent_app(question);
    if let Some((tool, params)) = consent_action(question) {
        let tool = super::presentation::computer_tool_name(tool).unwrap_or(tool);
        let mut sentence = super::presentation::describe_computer_call(
            tool,
            params.as_ref().and_then(serde_json::Value::as_object),
        );
        if let Some(app) = &app
            && !sentence.contains(app.name.as_str())
        {
            sentence.push_str(" in ");
            sentence.push_str(&app.name);
        }
        return sentence;
    }
    let text = question.question.trim();
    match consent_scope(question) {
        ComputerConsentScope::Task => text
            .split_once("? ")
            .map_or(text, |(_, rest)| rest.trim_start())
            .to_owned(),
        ComputerConsentScope::Call => text.to_owned(),
    }
}

pub fn consent_title(scope: ComputerConsentScope) -> &'static str {
    match scope {
        ComputerConsentScope::Task => "Allow Computer for this task?",
        ComputerConsentScope::Call => "Approve this tool call?",
    }
}

pub fn consent_card_title(scope: ComputerConsentScope, app: Option<&ComputerConsentApp>) -> String {
    match (scope, app) {
        (ComputerConsentScope::Task, Some(app)) => consent_app_title(app),
        _ => consent_title(scope).to_owned(),
    }
}

pub fn consent_allow_label(
    scope: ComputerConsentScope,
    app: Option<&ComputerConsentApp>,
) -> &'static str {
    match (scope, app) {
        (ComputerConsentScope::Task, Some(_)) => "Allow for this task",
        (ComputerConsentScope::Task, None) => "Allow Computer for this task",
        (ComputerConsentScope::Call, _) => "Approve once",
    }
}

pub fn consent_allow_description(
    scope: ComputerConsentScope,
    app: Option<&ComputerConsentApp>,
) -> String {
    match (scope, app) {
        (ComputerConsentScope::Task, Some(app)) => format!(
            "Continue actions in {} until this response ends. Other apps still ask. Stop cancels access.",
            app.name
        ),
        (ComputerConsentScope::Task, None) => "Continue routine desktop actions until this response ends. Stop cancels access. Clipboard reads still ask separately.".to_owned(),
        (ComputerConsentScope::Call, _) => "Allow just this request".to_owned(),
    }
}

pub fn consent_choices(
    scope: ComputerConsentScope,
    app: Option<&ComputerConsentApp>,
) -> Vec<(ConsentChoice, String)> {
    let mut choices = vec![(
        ConsentChoice::Allow,
        consent_allow_label(scope, app).to_owned(),
    )];
    if scope == ComputerConsentScope::Task
        && let Some(label) = app.and_then(consent_always_label)
    {
        choices.push((ConsentChoice::AlwaysAllow(label.clone()), label));
    }
    choices.push((ConsentChoice::Decline, COMPUTER_CONSENT_DECLINE.to_owned()));
    choices
}

pub fn consent_outcome_label(outcome: ConsentOutcome) -> &'static str {
    match outcome {
        ConsentOutcome::Allowed => "Allowed",
        ConsentOutcome::AlwaysAllowed => "Always allowed",
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
            computer_notice(LEGACY_CONTROL_ON_NOTE),
            Some(ComputerNotice::Disclosure)
        );
        assert_eq!(
            computer_notice(COMPUTER_ESCAPE_STOP_NOTICE),
            Some(ComputerNotice::Revoked)
        );
        assert_eq!(
            computer_notice(COMPUTER_DESKTOP_REVOKED_NOTE),
            Some(ComputerNotice::Revoked)
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
    fn a_takeover_note_is_a_paused_event_whatever_the_app() {
        use zeron_engine::computer::manager::COMPUTER_USER_PAUSE_NOTE_PREFIX;
        for app in ["Ghostty", "the target window"] {
            assert_eq!(
                computer_notice(&format!("{COMPUTER_USER_PAUSE_NOTE_PREFIX}{app}")),
                Some(ComputerNotice::Paused)
            );
        }
        assert_eq!(computer_notice("Paused"), None);
    }

    #[test]
    fn a_revoke_row_says_who_took_the_desktop_and_how() {
        assert_eq!(
            revoked_label(COMPUTER_DESKTOP_REVOKED_NOTE, Some("Esc")),
            "Desktop access revoked by you (Esc)"
        );
        assert_eq!(
            revoked_label(COMPUTER_DESKTOP_REVOKED_NOTE, Some("Stop")),
            "Desktop access revoked by you"
        );
        assert_eq!(
            revoked_label(COMPUTER_ESCAPE_STOP_NOTICE, None),
            "Desktop access revoked by you (Esc)"
        );
    }

    #[test]
    fn consent_outcome_reads_the_persisted_answer() {
        assert_eq!(consent_outcome(false, None), None);
        assert_eq!(
            consent_outcome(true, Some(&answer(&["Allow"]))),
            Some(ConsentOutcome::Allowed)
        );
        assert_eq!(
            consent_outcome(true, Some(&answer(&["Deny"]))),
            Some(ConsentOutcome::Declined)
        );
        assert_eq!(
            consent_outcome(true, Some(&answer(&["Decline"]))),
            Some(ConsentOutcome::Declined),
            "cards answered before the Deny label still read as declined"
        );
        assert_eq!(
            consent_outcome(true, Some(&[])),
            Some(ConsentOutcome::Cancelled)
        );
        assert_eq!(consent_outcome(true, None), Some(ConsentOutcome::Cancelled));
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

    fn safari(bundle_id: Option<&str>) -> ComputerConsentApp {
        ComputerConsentApp {
            name: "Safari".into(),
            bundle_id: bundle_id.map(str::to_owned),
            pid: Some(42),
        }
    }

    #[test]
    fn an_app_card_names_the_app_and_offers_always_only_when_verified() {
        let verified = safari(Some("com.apple.Safari"));
        let task = ComputerConsentScope::Task;
        assert_eq!(
            consent_card_title(task, Some(&verified)),
            "Allow Computer to use Safari?"
        );
        assert_eq!(
            consent_choices(task, Some(&verified))
                .into_iter()
                .map(|(_, label)| label)
                .collect::<Vec<_>>(),
            ["Allow for this task", "Always allow Safari", "Deny"]
        );
        let unverified = safari(None);
        assert_eq!(
            consent_card_title(task, Some(&unverified)),
            "Allow Computer to use Safari?"
        );
        assert_eq!(
            consent_choices(task, Some(&unverified))
                .into_iter()
                .map(|(choice, _)| choice)
                .collect::<Vec<_>>(),
            [ConsentChoice::Allow, ConsentChoice::Decline]
        );
        assert_eq!(
            consent_choices(task, None)
                .into_iter()
                .map(|(_, label)| label)
                .collect::<Vec<_>>(),
            ["Allow Computer for this task", "Deny"]
        );
    }

    fn consent(
        scope: ComputerConsentScope,
        app: Option<ComputerConsentApp>,
        tool: Option<&str>,
        params: Option<&str>,
        question: &str,
    ) -> UserInputQuestion {
        UserInputQuestion {
            id: "consent".into(),
            header: "Computer".into(),
            question: question.into(),
            options: vec!["Allow".into(), "Deny".into()],
            multi_select: false,
            kind: Some(zeron_proto::UserInputKind::Consent {
                scope,
                app,
                tool: tool.map(str::to_owned),
                params: params.map(str::to_owned),
            }),
        }
    }

    #[test]
    fn the_task_question_drops_its_title_sentence() {
        let task = ComputerConsentScope::Task;
        assert_eq!(
            consent_detail(&consent(
                task,
                None,
                None,
                None,
                "Allow Computer for this task? The agent will use this Mac's apps until this response ends."
            )),
            "The agent will use this Mac's apps until this response ends."
        );
        assert_eq!(
            consent_detail(&consent(
                task,
                Some(safari(Some("com.apple.Safari"))),
                None,
                None,
                "Allow Computer to use Safari (com.apple.Safari)? The agent wants to run computer_click."
            )),
            "The agent wants to run computer_click.",
            "an old doc's title sentence drops even with its bundle id"
        );
        assert_eq!(
            consent_detail(&consent(
                ComputerConsentScope::Call,
                None,
                None,
                None,
                "Allow the agent to read the clipboard once?"
            )),
            "Allow the agent to read the clipboard once?"
        );
    }

    #[test]
    fn consent_body_names_the_action_never_the_tool_or_typed_text() {
        let ghostty = ComputerConsentApp {
            name: "Ghostty".into(),
            bundle_id: Some("com.mitchellh.ghostty".into()),
            pid: Some(4242),
        };
        let question = consent(
            ComputerConsentScope::Task,
            Some(ghostty.clone()),
            Some("computer_type_text"),
            Some(r#"{"window_id":"cua:4242:17","text":"hunter2"}"#),
            "Allow Computer to use Ghostty? The agent will use Ghostty until this response ends.",
        );
        let detail = consent_detail(&question);
        assert_eq!(detail, "Type in Ghostty");
        for leak in ["computer_", "hunter2", "com.mitchellh", "4242"] {
            assert!(!detail.contains(leak), "{leak}: {detail}");
        }
        let launch = consent(
            ComputerConsentScope::Task,
            Some(ghostty),
            Some("mcp__zeron__computer_launch_app"),
            Some(r#"{"app":"Ghostty"}"#),
            "",
        );
        assert_eq!(consent_detail(&launch), "Open Ghostty");
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
