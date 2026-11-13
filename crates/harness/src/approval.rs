//! Approvals asked of the person. A run that is not in full access asks
//! before what its [`AccessMode`] does not cover; every driver asks the same
//! way — a yes/no question through [`crate::RunControls::request_input`] —
//! so an approval reads alike whichever agent raised it.
use zeron_proto::{AccessMode, ApprovalAction, UserInputAnswer, UserInputQuestion};

pub const ALLOW: &str = zeron_proto::APPROVAL_ALLOW;
pub const DENY: &str = zeron_proto::APPROVAL_DENY;

/// What an agent wants to do, for deciding whether its mode covers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Write, edit or delete files.
    Edit,
    /// Anything else: a shell command, network, an MCP or unknown tool.
    Other,
}

/// Whether `access` lets the agent go ahead with `action` without asking.
pub fn allowed_without_asking(access: AccessMode, action: Action) -> bool {
    match access {
        AccessMode::FullAccess => true,
        AccessMode::AutoAcceptEdits => action == Action::Edit,
        AccessMode::Supervised => false,
    }
}

/// A yes/no approval question: `header` names the kind ("Approve command"),
/// `question` says what and ends asking ("… Allow it?").
pub fn question(header: impl Into<String>, question: impl Into<String>) -> UserInputQuestion {
    UserInputQuestion {
        id: uuid::Uuid::new_v4().to_string(),
        header: header.into(),
        question: question.into(),
        options: vec![ALLOW.into(), DENY.into()],
        multi_select: false,
        kind: None,
        prefill: None,
        multiline: false,
        approval: true,
        action: None,
    }
}

pub fn covered(access: AccessMode, questions: &[UserInputQuestion]) -> bool {
    !questions.is_empty()
        && questions.iter().all(|q| {
            q.action
                .filter(|_| q.approval && q.kind.is_none())
                .is_some_and(|action| {
                    let action = match action {
                        ApprovalAction::Edit => Action::Edit,
                        ApprovalAction::Command | ApprovalAction::Tool => Action::Other,
                    };
                    allowed_without_asking(access, action)
                })
        })
}

pub fn least_covering(questions: &[UserInputQuestion]) -> Option<AccessMode> {
    AccessMode::ALL
        .into_iter()
        .find(|access| covered(*access, questions))
}

pub fn allow_all(questions: &[UserInputQuestion]) -> Vec<UserInputAnswer> {
    questions
        .iter()
        .map(|q| UserInputAnswer {
            question_id: q.id.clone(),
            labels: vec![ALLOW.into()],
        })
        .collect()
}

/// Whether `answers` approve `question`. Anything but an explicit yes —
/// a no, free text, or the questions closed unanswered — is a refusal, so
/// the agent is never allowed silently.
pub fn approved(question: &UserInputQuestion, answers: &[UserInputAnswer]) -> bool {
    answers.iter().any(|a| {
        a.question_id == question.id && a.labels.iter().any(|l| l.eq_ignore_ascii_case(ALLOW))
    })
}

/// "`agent` wants to run `command`. Allow it?"
pub fn command(agent: &str, command: &str) -> UserInputQuestion {
    let command = command.trim();
    typed(
        ApprovalAction::Command,
        question(
            "Approve command",
            if command.is_empty() {
                format!("{agent} wants to run a command. Allow it?")
            } else {
                format!("{agent} wants to run `{command}`. Allow it?")
            },
        ),
    )
}

/// "`agent` wants to modify a, b. Allow it?"
pub fn edit(agent: &str, paths: &[&str]) -> UserInputQuestion {
    typed(
        ApprovalAction::Edit,
        question(
            "Approve file change",
            if paths.is_empty() {
                format!("{agent} wants to modify files. Allow it?")
            } else {
                format!("{agent} wants to modify {}. Allow it?", paths.join(", "))
            },
        ),
    )
}

/// "`agent` wants to use `tool`. Allow it?", with what it says it is for.
pub fn tool(agent: &str, tool: &str, detail: Option<&str>) -> UserInputQuestion {
    let detail = detail
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(|d| format!(" ({d})"))
        .unwrap_or_default();
    typed(
        ApprovalAction::Tool,
        question(
            "Approve tool",
            format!("{agent} wants to use {tool}{detail}. Allow it?"),
        ),
    )
}

fn typed(action: ApprovalAction, question: UserInputQuestion) -> UserInputQuestion {
    UserInputQuestion {
        action: Some(action),
        ..question
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_cover_what_they_say() {
        use AccessMode::*;
        assert!(allowed_without_asking(FullAccess, Action::Other));
        assert!(allowed_without_asking(AutoAcceptEdits, Action::Edit));
        assert!(!allowed_without_asking(AutoAcceptEdits, Action::Other));
        assert!(!allowed_without_asking(Supervised, Action::Edit));
    }

    #[test]
    fn a_mode_covers_only_typed_approvals_it_allows() {
        use AccessMode::*;
        let edit = edit("Muse", &["a.rs"]);
        let command = command("Muse", "ls");
        let tool = tool("Muse", "fetch", None);
        let plan = question(zeron_proto::PLAN_APPROVAL_HEADER, "Start the plan?");
        assert!(covered(AutoAcceptEdits, std::slice::from_ref(&edit)));
        assert!(!covered(AutoAcceptEdits, &[edit.clone(), command.clone()]));
        assert!(covered(FullAccess, &[edit.clone(), command.clone(), tool.clone()]));
        assert!(!covered(Supervised, std::slice::from_ref(&edit)));
        assert!(!covered(FullAccess, std::slice::from_ref(&plan)));
        assert!(!covered(FullAccess, &[]));
        let consent = UserInputQuestion {
            kind: Some(zeron_proto::UserInputKind::Foreground { app: None }),
            ..command.clone()
        };
        assert!(!covered(FullAccess, &[consent]));
        assert_eq!(least_covering(&[edit.clone()]), Some(AutoAcceptEdits));
        assert_eq!(least_covering(&[edit.clone(), tool]), Some(FullAccess));
        assert_eq!(least_covering(&[command, plan]), None);
        let yes = allow_all(std::slice::from_ref(&edit));
        assert!(approved(&edit, &yes));
    }

    #[test]
    fn only_an_explicit_yes_approves() {
        let q = command("Muse", "cargo test");
        assert_eq!(q.question, "Muse wants to run `cargo test`. Allow it?");
        let answer = |labels: &[&str]| {
            vec![UserInputAnswer {
                question_id: q.id.clone(),
                labels: labels.iter().map(|l| l.to_string()).collect(),
            }]
        };
        assert!(approved(&q, &answer(&["Yes"])));
        assert!(!approved(&q, &answer(&["No"])));
        assert!(!approved(&q, &answer(&["sure, go ahead"])));
        assert!(!approved(&q, &[]));
    }
}
