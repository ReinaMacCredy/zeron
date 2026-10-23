use super::{ComputerConsentApp, ComputerConsentScope, ComputerPermission};
use crate::{UserInputAnswer, UserInputQuestion};

pub const COMPUTER_CONSENT_ALLOW: &str = "Allow";
pub const COMPUTER_CONSENT_DECLINE: &str = "Deny";
pub const COMPUTER_CONSENT_ALWAYS_PREFIX: &str = "Always allow ";
pub const COMPUTER_FOREGROUND_ALLOW: &str = "Allow for this turn";
pub const COMPUTER_FOREGROUND_DECLINE: &str = "No";
pub const COMPUTER_CONTROL_ON_DISCLOSURE: &str = "Computer control is on for this turn. If the user uses an app you are driving, input pauses: read fresh state and continue. If a computer tool returns user_revoked, the user pressed Stop: stop using computer tools and summarize.";
pub const COMPUTER_USER_PAUSE_NOTE_PREFIX: &str = "Paused while you used ";
pub const COMPUTER_ESCAPE_STOP_NOTICE: &str = "Computer use stopped: Escape was pressed";
pub const COMPUTER_DESKTOP_REVOKED_NOTE: &str = "Desktop access revoked by you";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevokeSource {
    Escape,
    Stop,
}

impl RevokeSource {
    pub fn hint(self) -> &'static str {
        match self {
            Self::Escape => "Esc",
            Self::Stop => "Stop",
        }
    }
}

pub fn consent_always_label(app: &ComputerConsentApp) -> Option<String> {
    app.verified()
        .then(|| format!("{COMPUTER_CONSENT_ALWAYS_PREFIX}{}", app.name))
}

pub fn foreground_allowed(answers: &[UserInputAnswer]) -> Option<bool> {
    let labels: Vec<&str> = answers
        .iter()
        .flat_map(|answer| answer.labels.iter().map(String::as_str))
        .collect();
    if labels.is_empty() {
        None
    } else {
        Some(labels.contains(&COMPUTER_FOREGROUND_ALLOW))
    }
}

pub const COMPUTER_DENIED_TITLE: &str = "Computer is off for ordinary turns";
pub const COMPUTER_DENIED_HINT: &str =
    "Send this request with /computer-use to let the agent use the desktop once.";
pub const COMPUTER_DENIED_ACTION: &str = "Use for this request";
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
    Foreground,
    LeftOpen,
}

pub fn left_open_app(note: &str) -> Option<&str> {
    let rest = note.strip_prefix("Left ")?;
    let rest = rest
        .strip_suffix(" windows open")
        .or_else(|| rest.strip_suffix(" window open"))?;
    let (count, app) = rest.split_once(' ')?;
    (count.parse::<usize>().is_ok_and(|count| count > 0) && !app.trim().is_empty()).then_some(app)
}

pub fn computer_notice(note: &str) -> Option<ComputerNotice> {
    if left_open_app(note).is_some() {
        return Some(ComputerNotice::LeftOpen);
    }
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
    pub fn answer_label(&self) -> &str {
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
        Some(crate::UserInputKind::Consent { app, .. }) => app.clone(),
        _ => None,
    }
}

pub fn consent_scope(question: &UserInputQuestion) -> ComputerConsentScope {
    match &question.kind {
        Some(crate::UserInputKind::Consent { scope, .. }) => *scope,
        _ => ComputerConsentScope::Task,
    }
}

pub fn foreground_outcome_label(
    questions: &[UserInputQuestion],
    answers: Option<&[UserInputAnswer]>,
) -> Option<String> {
    let app = questions.iter().find_map(|question| match &question.kind {
        Some(crate::UserInputKind::Foreground { app }) => Some(app.as_ref()),
        _ => None,
    })?;
    let app = app.map_or("the app", |app| app.name.as_str());
    Some(if foreground_allowed(answers?)? {
        format!("You allowed {app} to come to the front")
    } else {
        format!("You kept {app} in the background")
    })
}

pub const COMPUTER_FOREGROUND_DETAIL: &str =
    "Zeron needs it in front to type. It will cover your screen until the turn ends.";

pub fn foreground_title(app: Option<&str>) -> String {
    format!("Bring {} to the front?", app.unwrap_or("the app"))
}

fn consent_action(question: &UserInputQuestion) -> Option<(&str, Option<&str>)> {
    match &question.kind {
        Some(crate::UserInputKind::Consent {
            tool: Some(tool),
            params,
            ..
        }) => Some((tool.as_str(), params.as_deref())),
        _ => None,
    }
}

pub fn consent_action_sentence(
    tool: &str,
    params: Option<&str>,
    app: Option<&ComputerConsentApp>,
) -> String {
    let params: Option<serde_json::Value> =
        params.and_then(|params| serde_json::from_str(params).ok());
    let tool = super::presentation::computer_tool_name(tool).unwrap_or(tool);
    let mut sentence = super::presentation::describe_computer_call(
        tool,
        params.as_ref().and_then(serde_json::Value::as_object),
    );
    if let Some(app) = app
        && !sentence.contains(app.name.as_str())
    {
        sentence.push_str(" in ");
        sentence.push_str(&app.name);
    }
    sentence
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
    if let Some((tool, params)) = consent_action(question) {
        return consent_action_sentence(tool, params, consent_app(question).as_ref());
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
        ComputerConsentScope::Task => "Allow Zeron to use this Mac?",
        ComputerConsentScope::Call => "Approve this tool call?",
    }
}

pub fn consent_card_title(scope: ComputerConsentScope, app: Option<&ComputerConsentApp>) -> String {
    match (scope, app) {
        (ComputerConsentScope::Task, Some(app)) => format!("Allow Zeron to use {}?", app.name),
        _ => consent_title(scope).to_owned(),
    }
}

pub const COMPUTER_FIRST_USE_DISCLOSURE: &str = "Computer use lets Zeron use apps on your computer. It may take screenshots and act in the background. You can stop at any time from the menu bar.";

pub fn consent_choices(
    scope: ComputerConsentScope,
    app: Option<&ComputerConsentApp>,
) -> Vec<(ConsentChoice, String)> {
    let allow = match scope {
        ComputerConsentScope::Task => "Allow this conversation",
        ComputerConsentScope::Call => "Allow once",
    };
    let mut choices = vec![(ConsentChoice::Allow, allow.to_owned())];
    if scope == ComputerConsentScope::Task
        && let Some(label) = app.and_then(consent_always_label)
    {
        choices.push((ConsentChoice::AlwaysAllow(label), "Always allow".to_owned()));
    }
    choices.push((ConsentChoice::Decline, COMPUTER_CONSENT_DECLINE.to_owned()));
    choices
}

pub fn consent_choice_key(choice: &ConsentChoice) -> &'static str {
    match choice {
        ConsentChoice::Allow => "allow",
        ConsentChoice::AlwaysAllow(_) => "always",
        ConsentChoice::Decline => "deny",
    }
}

const TERMINALS: &[&str] = &[
    "com.apple.terminal",
    "com.googlecode.iterm2",
    "com.mitchellh.ghostty",
    "dev.warp.warp-stable",
    "net.kovidgoyal.kitty",
    "org.alacritty",
    "io.alacritty",
    "com.github.wez.wezterm",
    "co.zeit.hyper",
    "terminal",
    "iterm",
    "iterm2",
    "ghostty",
    "warp",
    "kitty",
    "alacritty",
    "wezterm",
    "hyper",
];

const MESSAGING: &[&str] = &[
    "com.apple.mail",
    "com.apple.mobilesms",
    "com.tinyspeck.slackmacgap",
    "com.hnc.discord",
    "ru.keepcoder.telegram",
    "net.whatsapp.whatsapp",
    "com.microsoft.teams",
    "com.microsoft.teams2",
    "com.microsoft.outlook",
    "org.whispersystems.signal-desktop",
    "mail",
    "messages",
    "slack",
    "discord",
    "telegram",
    "whatsapp",
    "microsoft teams",
    "microsoft outlook",
    "outlook",
    "signal",
];

const BROWSERS: &[&str] = &[
    "com.apple.safari",
    "com.google.chrome",
    "com.google.chrome.canary",
    "org.mozilla.firefox",
    "com.microsoft.edgemac",
    "company.thebrowser.browser",
    "com.brave.browser",
    "com.operasoftware.opera",
    "com.vivaldi.vivaldi",
    "net.imput.helium",
    "company.thebrowser.dia",
    "safari",
    "google chrome",
    "chrome",
    "firefox",
    "microsoft edge",
    "arc",
    "brave browser",
    "opera",
    "vivaldi",
    "helium",
    "dia",
];

const FILES: &[&str] = &["com.apple.finder", "finder"];

pub fn elevated_risk(app: &ComputerConsentApp) -> Option<&'static str> {
    let bundle = app.bundle_id.as_deref().map(str::to_lowercase);
    let name = app.name.trim().to_lowercase();
    let listed = |list: &[&str]| {
        list.iter()
            .any(|entry| bundle.as_deref() == Some(*entry) || name == *entry)
    };
    if listed(TERMINALS) {
        Some("Zeron could run commands that change files and settings on this Mac.")
    } else if listed(MESSAGING) {
        Some("Zeron could read your messages and send them as you.")
    } else if listed(BROWSERS) {
        Some("Zeron could act on websites where you are signed in.")
    } else if listed(FILES) {
        Some("Zeron could move, rename or delete your files.")
    } else {
        None
    }
}

pub fn consent_event_label(
    outcome: ConsentOutcome,
    scope: ComputerConsentScope,
    app: Option<&ComputerConsentApp>,
) -> String {
    let name = app.map(|app| app.name.as_str());
    match (outcome, scope, name) {
        (ConsentOutcome::Allowed, ComputerConsentScope::Task, Some(name)) => {
            format!("Allowed {name} for this task")
        }
        (ConsentOutcome::Allowed, ComputerConsentScope::Task, None) => {
            "Allowed Computer for this task".into()
        }
        (ConsentOutcome::Allowed, ComputerConsentScope::Call, Some(name)) => {
            format!("Allowed {name} once")
        }
        (ConsentOutcome::Allowed, ComputerConsentScope::Call, None) => "Allowed once".into(),
        (ConsentOutcome::AlwaysAllowed, _, Some(name)) => format!("Always allowed {name}"),
        (ConsentOutcome::AlwaysAllowed, _, None) => "Always allowed".into(),
        (ConsentOutcome::Declined, _, Some(name)) => format!("Declined {name}"),
        (ConsentOutcome::Declined, _, None) => "Declined".into(),
        (ConsentOutcome::Cancelled, _, _) => "Cancelled".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn terminals_messaging_browsers_and_finder_carry_an_elevated_risk() {
        let app = |name: &str, bundle: Option<&str>| ComputerConsentApp {
            name: name.into(),
            bundle_id: bundle.map(str::to_owned),
            pid: None,
        };
        for (name, bundle) in [
            ("Ghostty", Some("com.mitchellh.ghostty")),
            ("Terminal", None),
            ("iTerm2", Some("com.googlecode.iterm2")),
            ("Messages", Some("com.apple.MobileSMS")),
            ("Slack", None),
            ("Discord", Some("com.hnc.Discord")),
            ("Mail", Some("com.apple.mail")),
            ("Safari", Some("com.apple.Safari")),
            ("Google Chrome", None),
            ("Helium", Some("net.imput.helium")),
            ("Dia", Some("company.thebrowser.dia")),
            ("Finder", Some("com.apple.finder")),
        ] {
            assert!(
                elevated_risk(&app(name, bundle)).is_some(),
                "{name} is high risk"
            );
        }
        for (name, bundle) in [
            ("Calculator", Some("com.apple.calculator")),
            ("Notes", None),
            ("Terminally Chill", None),
        ] {
            assert_eq!(elevated_risk(&app(name, bundle)), None, "{name}");
        }
        assert_eq!(
            elevated_risk(&app("Ghostty", Some("com.mitchellh.ghostty"))),
            Some("Zeron could run commands that change files and settings on this Mac.")
        );
    }

    #[test]
    fn a_settled_consent_reads_as_its_outcome() {
        let app = safari(Some("com.apple.Safari"));
        let task = ComputerConsentScope::Task;
        let call = ComputerConsentScope::Call;
        let cases = [
            (
                ConsentOutcome::Allowed,
                task,
                Some(&app),
                "Allowed Safari for this task",
            ),
            (
                ConsentOutcome::Allowed,
                task,
                None,
                "Allowed Computer for this task",
            ),
            (ConsentOutcome::Allowed, call, None, "Allowed once"),
            (
                ConsentOutcome::AlwaysAllowed,
                task,
                Some(&app),
                "Always allowed Safari",
            ),
            (
                ConsentOutcome::Declined,
                task,
                Some(&app),
                "Declined Safari",
            ),
            (ConsentOutcome::Declined, call, None, "Declined"),
            (ConsentOutcome::Cancelled, task, Some(&app), "Cancelled"),
        ];
        for (outcome, scope, app, expected) in cases {
            let label = consent_event_label(outcome, scope, app);
            assert_eq!(label, expected);
            assert!(
                !label.contains("com.apple") && !label.contains("42"),
                "{label}"
            );
        }
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
            "Allow Zeron to use Safari?"
        );
        assert_eq!(
            consent_choices(task, Some(&verified))
                .into_iter()
                .map(|(_, label)| label)
                .collect::<Vec<_>>(),
            ["Allow this conversation", "Always allow", "Deny"]
        );
        let unverified = safari(None);
        assert_eq!(
            consent_card_title(task, Some(&unverified)),
            "Allow Zeron to use Safari?"
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
            ["Allow this conversation", "Deny"]
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
            approval: false,
            id: "consent".into(),
            header: "Computer".into(),
            question: question.into(),
            options: vec!["Allow".into(), "Deny".into()],
            multi_select: false,
            kind: Some(crate::UserInputKind::Consent {
                scope,
                app,
                tool: tool.map(str::to_owned),
                params: params.map(str::to_owned),
            }),
            prefill: None,
            multiline: false,
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
}
