use serde_json::Value;
use zeron_proto::computer::{ComputerConsentApp, ComputerConsentScope, ComputerStillFrame};
use zeron_proto::{ToolCall, UserInputAnswer, UserInputKind, UserInputQuestion};

// Copies of zeron-engine's labels and notes: this crate cannot depend on the engine.
pub const CONSENT_ALLOW: &str = "Allow";
pub const CONSENT_DECLINE: &str = "Deny";
pub const CONSENT_ALWAYS_PREFIX: &str = "Always allow ";
pub const CONTROL_ON_DISCLOSURE: &str = "Computer control ON for this turn: the agent is driving the desktop and the user can switch it off in Settings.";
pub const ESCAPE_STOP_NOTICE: &str = "Computer use stopped: Escape was pressed";
pub const SETUP_NOTE: &str = "Computer control needs setup";
pub const UNAVAILABLE_NOTE: &str = "Computer is unavailable for this turn";

pub const DENIED_TITLE: &str = "Computer control is off";
pub const DENIED_HINT: &str = "Turn it on in Settings to let the agent use the desktop.";
pub const DENIED_ENABLE: &str = "Enable";
const CAPABILITY_DENIED_CODE: &str = "capability_denied";
const COMPUTER_USE_COMMAND: &str = "/computer-use";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComputerNotice {
    Disclosure,
    EscapeStop,
    SetupRequired,
    Unavailable,
    ControlDenied,
}

pub fn notice(note: &str) -> Option<ComputerNotice> {
    match note {
        CONTROL_ON_DISCLOSURE => Some(ComputerNotice::Disclosure),
        ESCAPE_STOP_NOTICE => Some(ComputerNotice::EscapeStop),
        SETUP_NOTE => Some(ComputerNotice::SetupRequired),
        UNAVAILABLE_NOTE => Some(ComputerNotice::Unavailable),
        _ => None,
    }
}

pub fn is_control_denied(is_error: bool, output: Option<&str>) -> bool {
    is_error && output.is_some_and(|output| output.contains(CAPABILITY_DENIED_CODE))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConsentOutcome {
    Allowed,
    AlwaysAllowed,
    Declined,
    Cancelled,
}

pub fn consent_question(questions: &[UserInputQuestion]) -> Option<&UserInputQuestion> {
    questions.iter().find(|question| question.is_consent())
}

pub fn consent_scope(question: &UserInputQuestion) -> ComputerConsentScope {
    match &question.kind {
        Some(UserInputKind::Consent { scope, .. }) => *scope,
        None => ComputerConsentScope::Task,
    }
}

pub fn consent_app(question: &UserInputQuestion) -> Option<&ComputerConsentApp> {
    match &question.kind {
        Some(UserInputKind::Consent { app, .. }) => app.as_ref(),
        None => None,
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
    if labels.contains(&CONSENT_ALLOW) {
        Some(ConsentOutcome::Allowed)
    } else if labels
        .iter()
        .any(|label| label.starts_with(CONSENT_ALWAYS_PREFIX))
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

pub fn consent_answers(question_id: &str, label: &str) -> Vec<UserInputAnswer> {
    vec![UserInputAnswer {
        question_id: question_id.to_owned(),
        labels: vec![label.to_owned()],
    }]
}

pub fn consent_title(scope: ComputerConsentScope) -> &'static str {
    match scope {
        ComputerConsentScope::Task => "Allow Computer for this task?",
        ComputerConsentScope::Call => "Approve this tool call?",
    }
}

fn consent_app_title(app: &ComputerConsentApp) -> String {
    match &app.bundle_id {
        Some(bundle_id) => format!("Allow Computer to use {} ({bundle_id})?", app.name),
        None => format!("Allow Computer to use {} (unverified app)?", app.name),
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

pub fn consent_detail(
    scope: ComputerConsentScope,
    app: Option<&ComputerConsentApp>,
    question: &str,
) -> String {
    let question = question.trim();
    match scope {
        ComputerConsentScope::Task => question
            .strip_prefix(consent_card_title(scope, app).as_str())
            .map_or(question, str::trim_start)
            .to_owned(),
        ComputerConsentScope::Call => question.to_owned(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsentChoice {
    pub answer: String,
    pub title: String,
    pub primary: bool,
}

pub fn consent_choices(
    scope: ComputerConsentScope,
    app: Option<&ComputerConsentApp>,
) -> Vec<ConsentChoice> {
    let mut choices = vec![ConsentChoice {
        answer: CONSENT_ALLOW.to_owned(),
        title: consent_allow_label(scope, app).to_owned(),
        primary: true,
    }];
    if scope == ComputerConsentScope::Task
        && let Some(app) = app.filter(|app| app.verified())
    {
        let label = format!("{CONSENT_ALWAYS_PREFIX}{}", app.name);
        choices.push(ConsentChoice {
            answer: label.clone(),
            title: label,
            primary: false,
        });
    }
    choices.push(ConsentChoice {
        answer: CONSENT_DECLINE.to_owned(),
        title: CONSENT_DECLINE.to_owned(),
        primary: false,
    });
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

pub const TOOL_TITLES: &[(&str, &str)] = &[
    ("computer_screenshot", "Take a screenshot"),
    ("computer_get_state", "Read the screen"),
    ("computer_get_screen_size", "Measure the screen"),
    ("computer_list_windows", "Find open windows"),
    ("computer_list_apps", "List apps"),
    ("computer_verify_state", "Verify state"),
    ("computer_zoom", "Zoom into a window"),
    ("computer_get_accessibility_tree", "List apps and windows"),
    ("computer_get_cursor_position", "Read the cursor position"),
    ("computer_help", "Read the Computer playbook"),
    ("computer_click", "Click"),
    ("computer_move_cursor", "Move the agent cursor"),
    ("computer_drag", "Drag"),
    ("computer_scroll", "Scroll"),
    ("computer_type_text", "Type"),
    ("computer_press_key", "Press a key"),
    ("computer_set_value", "Set a field"),
    ("computer_select_text", "Select text"),
    ("computer_perform_action", "Activate a control"),
    ("computer_launch_app", "Open an app"),
    ("computer_activate_window", "Activate a window"),
    ("computer_set_window_frame", "Move or resize a window"),
    ("computer_invoke_menu", "Invoke a menu item"),
    ("computer_kill_app", "Force-quit an app"),
    (
        "computer_set_window_minimized",
        "Minimize or restore a window",
    ),
    ("computer_set_app_visibility", "Hide or unhide an app"),
    ("computer_wait", "Wait"),
    ("computer_read_clipboard", "Read the clipboard"),
    ("computer_write_clipboard", "Write to the clipboard"),
    ("computer_paste", "Paste text"),
    ("computer_run", "Run a sequence"),
    ("computer_inspect", "Inspect the computer"),
    ("computer_spaces", "Inspect desktop Spaces"),
    ("computer_browser_state", "Read the browser page"),
    ("computer_browser_prepare", "Prepare a browser"),
    ("computer_browser_navigate", "Open a browser page"),
    ("computer_browser_click", "Click in the browser"),
    ("computer_browser_type", "Type in a browser field"),
    ("computer_browser_dialog", "Handle a browser dialog"),
    ("computer_browser_upload", "Attach files in the browser"),
    ("computer_browser_download", "Download a file"),
    ("computer_browser_pointer", "Use the pointer in the browser"),
    ("computer_browser_press", "Press Enter in the browser"),
];

pub fn tool_title(tool: &str) -> &'static str {
    TOOL_TITLES
        .iter()
        .find(|(name, _)| *name == tool)
        .map_or("Computer", |(_, title)| title)
}

pub fn tool_name(candidate: &str) -> Option<&'static str> {
    let mut normalized = String::with_capacity(candidate.len());
    for ch in candidate.trim().chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            normalized.push(ch);
        } else if !normalized.ends_with('_') {
            normalized.push('_');
        }
    }
    TOOL_TITLES.iter().map(|(name, _)| *name).find(|name| {
        normalized == *name
            || normalized
                .strip_suffix(name)
                .is_some_and(|head| head.ends_with('_'))
    })
}

fn zeron_tool_name(candidate: &str) -> Option<&'static str> {
    let tool = tool_name(candidate)?;
    let normalized = candidate
        .trim()
        .to_lowercase()
        .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_");
    let head = normalized.strip_suffix(tool)?.trim_end_matches('_');
    (head == "zeron" || head.ends_with("_zeron")).then_some(tool)
}

pub fn computer_tool(call: &ToolCall) -> Option<&'static str> {
    let (tool, input) = match call {
        ToolCall::Mcp {
            server,
            tool,
            input,
        } if server == "zeron" => (tool_name(tool), input.as_ref()),
        ToolCall::Unknown { name, input } => (zeron_tool_name(name), input.as_ref()),
        _ => return None,
    };
    if tool.is_some() {
        return tool;
    }
    let raw = input.and_then(Value::as_object)?;
    ["tool", "toolName", "tool_name", "name", "_toolName"]
        .iter()
        .filter_map(|key| raw.get(*key).and_then(Value::as_str))
        .find_map(zeron_tool_name)
}

pub fn is_computer_invocation(text: &str) -> bool {
    let lead = text.len() - text.trim_start_matches(' ').len();
    if lead > 3 {
        return false;
    }
    let rest = &text[lead..];
    let Some(head) = rest.get(..COMPUTER_USE_COMMAND.len()) else {
        return false;
    };
    let after = &rest[COMPUTER_USE_COMMAND.len()..];
    head.eq_ignore_ascii_case(COMPUTER_USE_COMMAND)
        && (after.is_empty() || after.starts_with([' ', '\t', '\r', '\n']))
}

pub fn enabling_computer_use(draft: &str) -> String {
    if is_computer_invocation(draft) {
        draft.to_owned()
    } else if draft.trim().is_empty() {
        format!("{COMPUTER_USE_COMMAND} ")
    } else {
        format!("{COMPUTER_USE_COMMAND} {draft}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputerStill {
    pub sequence: u64,
    pub image: Vec<u8>,
}

pub fn parse_still_reply(reply: &Value) -> Option<ComputerStill> {
    let frame: ComputerStillFrame = serde_json::from_value(reply.get("frame")?.clone()).ok()?;
    let image = crate::live::b64::decode(&frame.data)?;
    (!image.is_empty()).then_some(ComputerStill {
        sequence: frame.sequence,
        image,
    })
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

    fn safari(bundle_id: Option<&str>) -> ComputerConsentApp {
        ComputerConsentApp {
            name: "Safari".into(),
            bundle_id: bundle_id.map(str::to_owned),
            pid: Some(42),
        }
    }

    #[test]
    fn engine_notes_map_to_their_notices_and_nothing_else_does() {
        assert_eq!(
            notice(CONTROL_ON_DISCLOSURE),
            Some(ComputerNotice::Disclosure)
        );
        assert_eq!(notice(ESCAPE_STOP_NOTICE), Some(ComputerNotice::EscapeStop));
        assert_eq!(
            notice("Computer control needs setup"),
            Some(ComputerNotice::SetupRequired)
        );
        assert_eq!(
            notice("Computer is unavailable for this turn"),
            Some(ComputerNotice::Unavailable)
        );
        assert_eq!(notice("Context compacted"), None);
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
            Some(ConsentOutcome::Declined)
        );
        assert_eq!(
            consent_outcome(true, Some(&answer(&["Always allow Safari"]))),
            Some(ConsentOutcome::AlwaysAllowed)
        );
        assert_eq!(
            consent_outcome(false, Some(&answer(&["Allow"]))),
            Some(ConsentOutcome::Allowed),
            "an answer in flight settles the card before the host resolves it"
        );
        assert_eq!(
            consent_outcome(true, Some(&[])),
            Some(ConsentOutcome::Cancelled)
        );
        assert_eq!(consent_outcome(true, None), Some(ConsentOutcome::Cancelled));
    }

    #[test]
    fn consent_answers_carry_the_engine_labels() {
        assert_eq!(
            consent_answers("consent", CONSENT_ALLOW),
            answer(&["Allow"])
        );
        assert_eq!(
            consent_answers("consent", CONSENT_DECLINE),
            answer(&["Deny"])
        );
    }

    #[test]
    fn an_app_card_names_the_app_and_offers_always_only_when_verified() {
        let task = ComputerConsentScope::Task;
        let verified = safari(Some("com.apple.Safari"));
        assert_eq!(
            consent_card_title(task, Some(&verified)),
            "Allow Computer to use Safari (com.apple.Safari)?"
        );
        let titles = |app| {
            consent_choices(task, app)
                .into_iter()
                .map(|c| c.title)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            titles(Some(&verified)),
            ["Allow for this task", "Always allow Safari", "Deny"]
        );
        let unverified = safari(None);
        assert_eq!(titles(Some(&unverified)), ["Allow for this task", "Deny"]);
        assert_eq!(titles(None), ["Allow Computer for this task", "Deny"]);
        let answers: Vec<String> = consent_choices(task, None)
            .into_iter()
            .map(|c| c.answer)
            .collect();
        assert_eq!(answers, ["Allow", "Deny"]);
        assert_eq!(
            consent_choices(ComputerConsentScope::Call, Some(&verified))
                .into_iter()
                .map(|c| c.title)
                .collect::<Vec<_>>(),
            ["Approve once", "Deny"]
        );
    }

    #[test]
    fn the_task_question_drops_its_title_sentence() {
        assert_eq!(
            consent_detail(
                ComputerConsentScope::Task,
                None,
                "Allow Computer for this task? The agent wants to run computer_click."
            ),
            "The agent wants to run computer_click."
        );
        assert_eq!(
            consent_detail(
                ComputerConsentScope::Task,
                Some(&safari(Some("com.apple.Safari"))),
                "Allow Computer to use Safari (com.apple.Safari)? The agent wants to run computer_click."
            ),
            "The agent wants to run computer_click."
        );
        assert_eq!(
            consent_detail(
                ComputerConsentScope::Call,
                None,
                "Allow the agent to run x once?"
            ),
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

    fn mcp(server: &str, tool: &str) -> ToolCall {
        ToolCall::Mcp {
            server: server.into(),
            tool: tool.into(),
            input: None,
        }
    }

    fn unknown(name: &str) -> ToolCall {
        ToolCall::Unknown {
            name: name.into(),
            input: None,
        }
    }

    #[test]
    fn computer_tools_are_recognized_in_every_harness_shape() {
        for call in [
            mcp("zeron", "computer_type_text"),
            unknown("mcp__zeron__computer_type_text"),
            unknown("Tool: zeron/computer_type_text"),
            unknown("zeron_computer_type_text"),
        ] {
            assert_eq!(computer_tool(&call), Some("computer_type_text"));
        }
        let wrapped = ToolCall::Unknown {
            name: "call_tool".into(),
            input: Some(serde_json::json!({
                "toolName": "mcp__zeron__computer_click",
                "arguments": { "x": 1 }
            })),
        };
        assert_eq!(computer_tool(&wrapped), Some("computer_click"));
        assert_eq!(tool_title("computer_type_text"), "Type");
        assert_eq!(tool_title("computer_browser_click"), "Click in the browser");
        assert_eq!(tool_title("computer_read_clipboard"), "Read the clipboard");
        assert_eq!(TOOL_TITLES.len(), 43);
    }

    #[test]
    fn only_zerons_own_server_is_recognized() {
        assert_eq!(computer_tool(&mcp("other", "computer_click")), None);
        assert_eq!(computer_tool(&mcp("github", "list_issues")), None);
        for name in [
            "computer_click",
            "mcp__other__computer_click",
            "notzeron_computer_click",
            "precomputer_click",
        ] {
            assert_eq!(computer_tool(&unknown(name)), None, "{name}");
        }
        assert_eq!(
            computer_tool(&ToolCall::Exec {
                command: "computer_click".into()
            }),
            None
        );
    }

    #[test]
    fn enable_prefixes_the_draft_once_like_the_mac() {
        for (draft, expected) in [
            ("", "/computer-use "),
            ("  \n", "/computer-use "),
            ("open Calculator", "/computer-use open Calculator"),
            (
                "/computer-use open Calculator",
                "/computer-use open Calculator",
            ),
            ("/computer-use ", "/computer-use "),
            ("/COMPUTER-USE x", "/COMPUTER-USE x"),
            ("   /computer-use", "   /computer-use"),
            ("/computer-use\nline two", "/computer-use\nline two"),
            ("    /computer-use", "/computer-use     /computer-use"),
            ("/computer-used", "/computer-use /computer-used"),
            ("é", "/computer-use é"),
        ] {
            assert_eq!(enabling_computer_use(draft), expected, "{draft:?}");
        }
    }

    #[test]
    fn a_still_reply_decodes_its_image() {
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
        let still = parse_still_reply(&serde_json::json!({
            "frame": { "sequence": 7, "timestampMs": 12.5, "keyframe": true, "data": png }
        }))
        .expect("frame");
        assert_eq!(still.sequence, 7);
        assert_eq!(&still.image[1..4], b"PNG");
        assert_eq!(
            parse_still_reply(&serde_json::json!({ "frame": null })),
            None
        );
        assert_eq!(parse_still_reply(&serde_json::json!({})), None);
    }
}
