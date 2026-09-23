use std::sync::LazyLock;

use regex::Regex;
use zeron_doc::{MessagePart, MessageRole, MessageStatus, SessionMessageEntry};
use zeron_proto::UserInputQuestion;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ComputerForegroundAuthorization {
    pub user_requested_visible_use: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ComputerForegroundContext<'a> {
    pub known_app_names: &'a [String],
}

pub const COMPUTER_FOREGROUND_NOT_AUTHORIZED: ComputerForegroundAuthorization =
    ComputerForegroundAuthorization {
        user_requested_visible_use: false,
    };

const AUTHORIZED: ComputerForegroundAuthorization = ComputerForegroundAuthorization {
    user_requested_visible_use: true,
};

pub const COMPUTER_FOREGROUND_NOT_REQUESTED_CODE: &str = "foreground_not_requested";

pub const COMPUTER_FOREGROUND_USER_INTERACTION_CODE: &str = "foreground_user_interaction";

pub const COMPUTER_USER_INTERACTION_QUIET_MS: u64 = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VisibleUseRole {
    #[default]
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VisibleUseInput {
    pub request_id: String,
    pub question_titles: Vec<String>,
    pub answers: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VisibleUseEntry {
    pub role: VisibleUseRole,
    pub text: String,
    pub from_chat_id: Option<String>,
    pub fork_import: bool,
    pub streaming: bool,
    pub has_attachments: bool,
    pub has_skills: bool,
    pub has_mentions: bool,
    pub inputs: Vec<VisibleUseInput>,
    pub answers_request_id: Option<String>,
}

fn pattern(source: &str) -> Regex {
    Regex::new(source).expect("visible-use pattern")
}

// No lookahead in `regex`: Synara's trailing lookahead is consumed, equivalent for is_match.
static VISIBLE_USE_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"(?i)(?-u:\b)show (?:me )?(?:the |my )?(?:[0-9A-Za-z_-]+ ){0,3}(?:window|app|screen|desktop|browser|page)\s*(?:[.!?,;:]|$)",
        r"(?i)(?-u:\b)show me what you(?: are|'re) doing(?-u:\b)",
        r"(?i)(?-u:\b)(?:i (?:want|would like|'d like) to |let me )watch (?:you|it|the (?:app|browser|window))(?-u:\b)",
        r"(?i)(?-u:\b)let me see (?:it|you) work(?:ing)?(?-u:\b)",
        r"(?i)(?-u:\b)i (?:want|would like|'d like) to see (?:the |my )?(?:[0-9A-Za-z_-]+ ){0,3}(?:window|app|screen|desktop|browser|page)\s*(?:[.!?,;:]|$)",
        r"(?i)(?-u:\b)(?:put|show|display)(?-u:\b)[^.!?\n]{0,40}(?-u:\b)on (?:my|the) screen(?-u:\b)",
        r"(?i)(?-u:\b)(?:make|keep) (?:it|(?:the |my )?(?:[0-9A-Za-z_-]+ ){0,3}(?:app|window|browser)) visible(?-u:\b)",
        r"(?i)(?-u:\b)(?:bring|put|move)(?-u:\b)[^.!?]{0,40}(?-u:\b)(?:front|foreground)\s*(?:[.!?,;:]|$)",
        r"(?i)(?-u:\b)use (?:the )?foreground(?: mode)?\s*(?:[.!?,;:]|$)",
        r"(?i)(?-u:\b)take over (?:my|the) (?:screen|desktop|computer)(?-u:\b)",
        r"(?i)(?-u:\b)drive (?:my|the) (?:screen|desktop|computer)(?-u:\b)",
        r"(?i)(?-u:\b)(?:mostra(?:mi|re)?|porta(?:re)?|metti|mettere)(?-u:\b)[^.!?\n]{0,60}(?-u:\b)(?:sullo schermo|in primo piano)(?-u:\b)",
        r"(?i)(?-u:\b)voglio vedere (?:la finestra|il browser|lo schermo|il desktop)(?-u:\b)",
    ]
    .into_iter()
    .map(pattern)
    .collect()
});

static BACKGROUND_USE_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"(?i)(?-u:\b)(?:do not|don['’]t|never|not|avoid|without|stop)(?-u:\b)[^.!?\n]{0,100}(?-u:\b)(?:show|watch|visible|foreground|front|focus|raise|screen|desktop)(?-u:\b)",
        r"(?i)(?-u:\b)(?:keep|stay|remain|work|run|use)(?-u:\b)[^.!?\n]{0,60}(?-u:\b)(?:background|hidden|invisible)(?-u:\b)",
        r"(?i)(?-u:\b)background[- ]only(?-u:\b)",
        r"(?i)(?-u:\b)(?:non|senza|evita|smetti di)(?-u:\b)[^.!?\n]{0,100}(?-u:\b)(?:mostrare|mostrarmi|primo piano|schermo|focus)(?-u:\b)",
        r"(?i)(?-u:\b)(?:lavora|resta|rimani|mantieni)(?-u:\b)[^.!?\n]{0,60}(?-u:\b)(?:background|nascost[ao])(?-u:\b)",
    ]
    .into_iter()
    .map(pattern)
    .collect()
});

static UNTRUSTED_TEXT: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"(?i)<untrusted_text(?-u:\b)[^>]*>(?s:.)*?(?:</untrusted_text>|$)"));

static QUOTED_TEXT: LazyLock<Regex> = LazyLock::new(|| {
    pattern(r#"```(?s:.)*?(?:```|$)|`[^`]*(?:`|$)|"[^"\n]*"|“[^”\n]*”|‘[^’\n]*’"#)
});

static SINGLE_QUOTED: LazyLock<Regex> = LazyLock::new(|| pattern(r"(^|[\s(])'[^'\n]+'"));

static BLOCK_QUOTE_LINE: LazyLock<Regex> = LazyLock::new(|| pattern(r"(?m)^\s*>.*$"));

static KNOWN_APP_REQUEST: LazyLock<Regex> = LazyLock::new(|| {
    pattern(r"(?i)^(?:please[, ]+)?(?:show(?: me)?|display|mostra(?:mi)?)\s+(.+?)[.!?]*$")
});

static AFFIRMATIVE_REPLY: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r"(?i)^(?:yes|yeah|yep|ok(?:ay)?|sure|go ahead|s[iì]|va bene|certo|procedi|vai)(?:[, ]+(?:please|go ahead|per favore|fallo))?[.!]*$",
    )
});

static VISIBLE_USE_QUESTION: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r"(?i)(?:^|[.!?]\s+)(?:can i|may i|shall i|do you want me to|would you like me to|is it (?:ok(?:ay)?|alright) (?:if i|to)|posso|vuoi che)\s+([^?]*\?)$",
    )
});

static ROUTINE_CONTINUATION: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r"(?i)^(?:(?:ok(?:ay)?|yes|s[iì])[, ]+)?(?:please[, ]+)?(?:continue(?: (?:working|with (?:the |this |our )?(?:same |current )?(?:task|work|plan)))?|keep (?:going|working)|carry on|go (?:on|ahead)|proceed(?: with (?:the |this |our )?(?:same |current )?(?:task|work|plan))?|(?:try|retry)(?: (?:again|that|it|the same step))?|continua(?: pure)?|prosegui|procedi|vai|riprova)(?:[, ]+(?:please|per favore))?[.!]*$",
    )
});

// Re-emits the character Synara's lookahead kept, so it can still open the next quote.
fn strip_single_quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut from = 0;
    while let Some(found) = SINGLE_QUOTED.captures_at(text, from) {
        let whole = found.get(0).expect("match");
        let follows = text[whole.end()..].chars().next();
        let closes = follows.is_none_or(|c| c.is_whitespace() || ".,;:)".contains(c));
        if closes {
            out.push_str(&text[copied..whole.start()]);
            out.push_str(found.get(1).map_or("", |prefix| prefix.as_str()));
            copied = whole.end();
            from = whole.end();
        } else {
            from = whole.start()
                + text[whole.start()..]
                    .chars()
                    .next()
                    .map_or(1, char::len_utf8);
        }
        if from > text.len() {
            break;
        }
    }
    out.push_str(&text[copied..]);
    out
}

fn unquoted_request(text: &str) -> String {
    let text = UNTRUSTED_TEXT.replace_all(text, "");
    let text = QUOTED_TEXT.replace_all(&text, "");
    let text = strip_single_quoted(&text);
    BLOCK_QUOTE_LINE.replace_all(&text, "").into_owned()
}

fn requests_known_app_visibility(text: &str, context: ComputerForegroundContext<'_>) -> bool {
    let Some(app) = KNOWN_APP_REQUEST
        .captures(text.trim())
        .and_then(|captures| captures.get(1))
    else {
        return false;
    };
    let name = app.as_str().trim().to_lowercase();
    context
        .known_app_names
        .iter()
        .any(|candidate| candidate.trim().to_lowercase() == name)
}

pub fn message_requests_visible_use(text: &str, context: ComputerForegroundContext<'_>) -> bool {
    let request = unquoted_request(text);
    !BACKGROUND_USE_PATTERNS
        .iter()
        .any(|pattern| pattern.is_match(&request))
        && (VISIBLE_USE_PATTERNS
            .iter()
            .any(|pattern| pattern.is_match(&request))
            || requests_known_app_visibility(&request, context))
}

fn is_affirmative_reply(text: &str) -> bool {
    AFFIRMATIVE_REPLY.is_match(text.trim())
}

fn asks_visible_use_permission(text: &str, context: ComputerForegroundContext<'_>) -> bool {
    let question = unquoted_request(text);
    VISIBLE_USE_QUESTION
        .captures(question.trim())
        .and_then(|captures| captures.get(1))
        .is_some_and(|action| message_requests_visible_use(action.as_str(), context))
}

fn is_local_human_message(entry: &VisibleUseEntry) -> bool {
    entry.role == VisibleUseRole::User && entry.from_chat_id.is_none() && !entry.fork_import
}

fn is_local_assistant_message(entry: Option<&VisibleUseEntry>) -> bool {
    entry.is_some_and(|entry| {
        entry.role == VisibleUseRole::Assistant && !entry.fork_import && !entry.streaming
    })
}

fn confirms_visible_use(
    reply: &VisibleUseEntry,
    preceding: Option<&VisibleUseEntry>,
    context: ComputerForegroundContext<'_>,
) -> bool {
    is_local_assistant_message(preceding)
        && is_affirmative_reply(&reply.text)
        && preceding.is_some_and(|question| asks_visible_use_permission(&question.text, context))
}

// The asking entry may still stream: Zeron's run continues inside it after the answer.
fn answered_input_confirms(
    entry: &VisibleUseEntry,
    input: &VisibleUseInput,
    context: ComputerForegroundContext<'_>,
) -> bool {
    let Some(answers) = &input.answers else {
        return false;
    };
    entry.role == VisibleUseRole::Assistant
        && !entry.fork_import
        && input.question_titles.len() == 1
        && answers.len() == 1
        && is_affirmative_reply(&answers[0])
        && asks_visible_use_permission(&input.question_titles[0], context)
}

fn dead_run_answer_confirms(
    entries: &[VisibleUseEntry],
    reply_index: usize,
    request_id: &str,
    context: ComputerForegroundContext<'_>,
) -> bool {
    for question in entries[..reply_index].iter().rev() {
        if question.role == VisibleUseRole::User {
            return false;
        }
        if let Some(input) = question
            .inputs
            .iter()
            .find(|input| input.request_id == request_id)
        {
            return answered_input_confirms(question, input, context);
        }
    }
    false
}

fn is_routine_continuation(entry: &VisibleUseEntry) -> bool {
    !entry.has_attachments
        && !entry.has_skills
        && !entry.has_mentions
        && ROUTINE_CONTINUATION.is_match(entry.text.trim())
}

pub fn latest_user_authored_message(entries: &[VisibleUseEntry]) -> Option<&VisibleUseEntry> {
    entries
        .iter()
        .rev()
        .find(|entry| entry.role == VisibleUseRole::User && entry.from_chat_id.is_none())
}

pub fn computer_foreground_authorization_for_messages(
    entries: &[VisibleUseEntry],
    context: ComputerForegroundContext<'_>,
) -> ComputerForegroundAuthorization {
    for (index, entry) in entries.iter().enumerate().rev() {
        if entry.role == VisibleUseRole::Assistant {
            if let Some(input) = entry
                .inputs
                .iter()
                .rev()
                .find(|input| input.answers.is_some())
            {
                return ComputerForegroundAuthorization {
                    user_requested_visible_use: answered_input_confirms(entry, input, context),
                };
            }
            continue;
        }
        if !is_local_human_message(entry) {
            return COMPUTER_FOREGROUND_NOT_AUTHORIZED;
        }
        if let Some(request_id) = &entry.answers_request_id {
            return ComputerForegroundAuthorization {
                user_requested_visible_use: dead_run_answer_confirms(
                    entries, index, request_id, context,
                ),
            };
        }
        if message_requests_visible_use(&entry.text, context)
            || confirms_visible_use(entry, index.checked_sub(1).map(|i| &entries[i]), context)
        {
            return AUTHORIZED;
        }
        if !is_routine_continuation(entry) {
            return COMPUTER_FOREGROUND_NOT_AUTHORIZED;
        }
    }
    COMPUTER_FOREGROUND_NOT_AUTHORIZED
}

pub fn visible_use_entries(entries: &[SessionMessageEntry]) -> Vec<VisibleUseEntry> {
    entries
        .iter()
        .filter_map(|entry| {
            let role = match entry.role {
                MessageRole::User => VisibleUseRole::User,
                MessageRole::Assistant => VisibleUseRole::Assistant,
                MessageRole::System => return None,
            };
            let text = entry
                .parts
                .iter()
                .filter_map(|part| match part {
                    MessagePart::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            let inputs = entry
                .parts
                .iter()
                .filter_map(|part| match part {
                    MessagePart::Input {
                        request_id,
                        questions,
                        answers,
                        ..
                    } if !questions.iter().any(UserInputQuestion::is_consent) => {
                        Some(VisibleUseInput {
                            request_id: request_id.clone(),
                            question_titles: questions
                                .iter()
                                .map(|question| question.question.clone())
                                .collect(),
                            answers: answers.as_ref().map(|answers| {
                                answers
                                    .iter()
                                    .flat_map(|answer| answer.labels.iter().cloned())
                                    .collect()
                            }),
                        })
                    }
                    _ => None,
                })
                .collect();
            Some(VisibleUseEntry {
                role,
                text,
                from_chat_id: entry.from_chat_id.clone(),
                fork_import: entry.fork_import,
                streaming: entry.status == Some(MessageStatus::Streaming),
                has_attachments: entry
                    .parts
                    .iter()
                    .any(|part| matches!(part, MessagePart::Image { .. })),
                has_skills: false,
                has_mentions: false,
                inputs,
                answers_request_id: entry.answers_request_id.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: ComputerForegroundContext<'static> = ComputerForegroundContext {
        known_app_names: &[],
    };

    fn user(text: &str) -> VisibleUseEntry {
        VisibleUseEntry {
            text: text.to_owned(),
            ..VisibleUseEntry::default()
        }
    }

    fn assistant(text: &str) -> VisibleUseEntry {
        VisibleUseEntry {
            role: VisibleUseRole::Assistant,
            text: text.to_owned(),
            ..VisibleUseEntry::default()
        }
    }

    fn agent(text: &str) -> VisibleUseEntry {
        VisibleUseEntry {
            from_chat_id: Some("lead-chat".to_owned()),
            ..user(text)
        }
    }

    fn forked(entry: VisibleUseEntry) -> VisibleUseEntry {
        VisibleUseEntry {
            fork_import: true,
            ..entry
        }
    }

    fn asking(title: &str, answers: Option<&[&str]>) -> VisibleUseEntry {
        VisibleUseEntry {
            inputs: vec![VisibleUseInput {
                request_id: "visibility-question".to_owned(),
                question_titles: vec![title.to_owned()],
                answers: answers.map(|answers| answers.iter().map(|a| (*a).to_owned()).collect()),
            }],
            ..assistant("")
        }
    }

    fn dead_run_answer(request_id: &str, title: &str, answer: &str) -> VisibleUseEntry {
        VisibleUseEntry {
            answers_request_id: Some(request_id.to_owned()),
            ..user(&format!(
                "Answering your earlier question: {title}\n{answer}"
            ))
        }
    }

    fn authorized(entries: &[VisibleUseEntry]) -> bool {
        computer_foreground_authorization_for_messages(entries, NONE).user_requested_visible_use
    }

    fn authorized_with(entries: &[VisibleUseEntry], apps: &[&str]) -> bool {
        let names: Vec<String> = apps.iter().map(|app| (*app).to_owned()).collect();
        computer_foreground_authorization_for_messages(
            entries,
            ComputerForegroundContext {
                known_app_names: &names,
            },
        )
        .user_requested_visible_use
    }

    #[test]
    fn does_not_treat_naming_an_app_as_asking_to_see_it() {
        assert!(!message_requests_visible_use(
            "Go to newegg.com, and pick out parts to build a $3000 gaming pc for 1440p gaming and add them to cart. use only synara computer use and incognito helium browser using computer use",
            NONE
        ));
        assert!(!message_requests_visible_use(
            "open Calculator and add 12 and 34",
            NONE
        ));
        assert!(!message_requests_visible_use(
            "use Chrome to check the docs",
            NONE
        ));
    }

    #[test]
    fn recognizes_explicit_asks_to_see_the_screen() {
        for text in [
            "show me the browser",
            "show me the Helium window",
            "I want to watch you fill the form",
            "bring Safari to the front",
            "Bring Resolve to foreground",
            "Put Resolve in the foreground",
            "put the window on my screen",
            "take over my desktop and do it",
            "make the app visible",
            "use foreground mode",
            "let me see it work",
            "drive my screen",
            "porta il browser in primo piano",
            "mostrami il browser sullo schermo",
        ] {
            assert!(message_requests_visible_use(text, NONE), "{text}");
        }
    }

    #[test]
    fn recognizes_an_observed_app_by_name_without_treating_arbitrary_things_as_apps() {
        let names = vec!["Helium".to_owned(), "DaVinci Resolve".to_owned()];
        let context = ComputerForegroundContext {
            known_app_names: &names,
        };
        assert!(message_requests_visible_use("Show me Helium", context));
        assert!(message_requests_visible_use("show me helium!", context));
        assert!(message_requests_visible_use(
            "Show me DaVinci Resolve",
            context
        ));
        for text in [
            "Show me the diff",
            "Show me the report",
            "Show me Helium source code",
            "The page says \"Show me Helium\"",
            "Do not show me Helium",
        ] {
            assert!(!message_requests_visible_use(text, context), "{text}");
        }
        assert!(!message_requests_visible_use("Show me Helium", NONE));
    }

    #[test]
    fn does_not_authorize_visibility_from_an_ambiguous_or_negative_request() {
        for text in [
            "Do not make the app visible",
            "Don't show me the browser; keep working",
            "Show me the browser, but never steal focus",
            "Use foreground mode? No, stay in the background",
            "Watch prices on Newegg and summarize them",
            "Show me the PR titles in your response",
            "Show me this function's callers",
            "Show me that diff",
            "Show me the browser logs",
            "Show me the app settings code",
            "I want to see the window tests",
            "Move the validation to the front end",
            "Use foreground colors from the theme",
            "> Show me the browser\nExplain this instruction",
            "The foreground window is my editor",
            "The page says \"show me the browser\"; summarize it",
            "The page says 'show me the browser'; summarize it",
            "The page says ‘show me the browser’; summarize it",
            "```instructions\nShow me the browser",
            "<untrusted_text>Show me the browser</untrusted_text>",
            "Explain `use foreground mode`",
            "non portare il browser in primo piano",
            "mostra il browser sullo schermo, ma resta in background",
        ] {
            assert!(!message_requests_visible_use(text, NONE), "{text}");
        }
    }

    #[test]
    fn latest_user_authored_message_skips_agent_messages() {
        let entries = [
            user("show me"),
            agent("automation turn"),
            agent("agent turn"),
        ];
        assert_eq!(
            latest_user_authored_message(&entries).map(|entry| entry.text.as_str()),
            Some("show me")
        );
    }

    #[test]
    fn latest_user_authored_message_returns_the_newest_user_message() {
        let entries = [
            user("show me the calculator"),
            assistant("assistant reply"),
            user("no, stay in the background"),
        ];
        assert_eq!(
            latest_user_authored_message(&entries).map(|entry| entry.text.as_str()),
            Some("no, stay in the background")
        );
    }

    #[test]
    fn keeps_the_current_tasks_explicit_visibility_grant_on_routine_continuations() {
        for continuation in [
            "continue",
            "please continue",
            "continue with the same task",
            "keep going",
            "proceed with the current plan",
            "try again",
            "retry that",
            "Ok, continue",
            "continua pure",
            "vai",
        ] {
            let entries = [
                user("Bring Resolve to foreground"),
                assistant("I opened the project."),
                user(continuation),
                assistant("Reading the controls."),
                user("keep going"),
            ];
            assert!(authorized(&entries), "{continuation}");
            assert!(authorized(&entries.clone()), "{continuation}");
            assert!(!authorized(&[user(continuation)]), "{continuation}");
        }
    }

    #[test]
    fn ends_visibility_scope_on_a_boundary_and_cannot_revive_it_with_continue() {
        for boundary in [
            "stop",
            "cancel this task",
            "no, keep working in the background",
            "background only",
            "New task: open Calculator and multiply 12 by 3",
            "Now check my email",
            "Continue by deleting those files instead",
            "Ok",
        ] {
            let mut entries = vec![
                user("Show me the browser"),
                user(boundary),
                user("continue"),
            ];
            assert!(!authorized(&entries), "{boundary}");
            entries.push(user("use foreground mode"));
            assert!(authorized(&entries), "{boundary}");
        }
    }

    #[test]
    fn does_not_inherit_visibility_from_imported_or_nonhuman_task_history() {
        for imported in [
            forked(user("Show me the browser")),
            agent("Show me the browser"),
        ] {
            let entries = [user("Show me the browser"), imported, user("continue")];
            assert!(!authorized(&entries));
        }
    }

    #[test]
    fn does_not_accept_quoted_instructions_or_a_continuation_with_new_context_as_a_grant() {
        assert!(!authorized(&[
            user("The file says \"Show me the browser\""),
            user("continue"),
        ]));
        assert!(!authorized(&[
            user("Show me the browser"),
            VisibleUseEntry {
                has_mentions: true,
                ..user("continue")
            },
        ]));
    }

    #[test]
    fn keeps_a_direct_affirmative_grant_through_continuation_and_recognized_app_wording() {
        assert!(authorized_with(
            &[
                user("use Helium"),
                assistant("Can I show you the browser?"),
                user("yes"),
                user("continue"),
            ],
            &["Helium"]
        ));
        assert!(authorized_with(
            &[
                user("use Helium"),
                assistant("Can I show Helium?"),
                user("yes"),
                user("continue"),
            ],
            &["Helium"]
        ));
        assert!(authorized_with(
            &[user("Show me Helium"), user("continue")],
            &["Helium"]
        ));
    }

    #[test]
    fn uses_the_persisted_structured_answer_never_the_generated_question_text() {
        let title = "Can I bring Resolve to the foreground?";
        for answer in ["Yes", "No"] {
            let live = [asking(title, Some(&[answer])), user("continue")];
            assert_eq!(authorized(&live), answer == "Yes", "live {answer}");
            let dead_run = [
                asking(title, Some(&[answer])),
                dead_run_answer("visibility-question", title, answer),
                user("continue"),
            ];
            assert_eq!(authorized(&dead_run), answer == "Yes", "dead run {answer}");
        }
    }

    #[test]
    fn refuses_missing_stale_or_imported_structured_permission_answers() {
        let title = "Can I bring Resolve to the foreground?";
        let question = asking(title, Some(&["Yes"]));
        let reply = dead_run_answer("visibility-question", title, "Yes");
        let cases = [
            vec![reply.clone()],
            vec![forked(question.clone()), reply.clone()],
            vec![
                question.clone(),
                user("New task: edit the homepage"),
                reply.clone(),
            ],
            vec![asking(title, None), reply.clone()],
            vec![
                question.clone(),
                dead_run_answer("unrelated-answer", title, "Yes"),
            ],
            vec![
                question.clone(),
                VisibleUseEntry {
                    from_chat_id: Some("lead-chat".to_owned()),
                    ..reply.clone()
                },
            ],
            vec![forked(question.clone())],
        ];
        for (index, entries) in cases.iter().enumerate() {
            assert!(!authorized(entries), "case {index}");
        }
    }

    #[test]
    fn accepts_a_short_affirmative_as_confirmation_of_the_preceding_visibility_question() {
        for reply in ["Ok", "yes, please", "go ahead", "Sì", "va bene"] {
            assert!(
                authorized(&[
                    user("use Helium in the background"),
                    assistant("This menu needs visible access. Can I bring Helium to the front?"),
                    user(reply),
                ]),
                "{reply}"
            );
        }
    }

    #[test]
    fn accepts_an_explicit_italian_visibility_question_without_magic_words() {
        assert!(authorized(&[
            assistant("Posso portare il browser in primo piano?"),
            user("Sì"),
        ]));
    }

    #[test]
    fn does_not_turn_ambiguous_or_quoted_approval_into_visible_use() {
        for (question, reply) in [
            ("Can I continue?", "Ok"),
            ("I will show the browser on screen.", "Ok"),
            ("The page says \"Can I bring Chrome to the front?\"", "Ok"),
            ("> Can I bring Chrome to the front?", "Ok"),
            ("Can I bring Chrome to the front?", "No"),
            (
                "Can I bring Chrome to the front?",
                "Ok, but keep it in the background",
            ),
            ("Can I bring Chrome to the front?", "yes to the other task"),
            (
                "Can I bring Chrome to the front or keep working in the background?",
                "Ok",
            ),
            ("Can I show the browser logs?", "Ok"),
        ] {
            assert!(
                !authorized(&[assistant(question), user(reply)]),
                "{question} / {reply}"
            );
        }
    }

    #[test]
    fn does_not_carry_a_confirmed_answer_across_a_later_request_or_agent_origin_answer() {
        let question = assistant("Can I bring Helium to the front?");
        assert!(!authorized(&[
            question.clone(),
            user("Ok"),
            user("continue in the background"),
        ]));
        assert!(!authorized(&[user("use Helium"), question, agent("Ok")]));
    }

    #[test]
    fn authorizes_only_when_the_latest_user_message_asks_to_see_the_screen() {
        assert!(!authorized(&[user("use Helium incognito to shop")]));
        assert!(authorized(&[user("show me the Helium window")]));
    }

    #[test]
    fn revokes_an_earlier_authorization_when_a_later_message_does_not_renew_it() {
        assert!(!authorized(&[
            user("show me what you are doing"),
            user("stop"),
        ]));
    }

    #[test]
    fn refuses_a_thread_with_no_user_message_at_all() {
        assert!(!authorized(&[assistant("hello")]));
        assert!(!authorized(&[]));
    }

    #[test]
    fn refuses_a_foreground_excursion_in_an_agent_origin_turn() {
        assert!(!authorized(&[agent("show me the browser")]));
        assert!(!authorized(&[
            user("show me the browser"),
            agent("bring Safari to the front"),
        ]));
    }

    #[test]
    fn a_fork_whose_first_turn_is_continue_does_not_inherit_the_parents_grant() {
        assert!(!authorized(&[
            forked(user("show me the browser")),
            forked(assistant("Here is the browser.")),
            user("continue"),
        ]));
    }

    #[test]
    fn a_copied_visibility_question_answered_in_the_fork_is_refused() {
        assert!(!authorized(&[
            forked(user("use Safari")),
            forked(assistant("Can I bring Safari to the front?")),
            user("yes"),
        ]));
    }

    #[test]
    fn a_live_structured_no_after_a_visible_request_refuses_the_excursion() {
        assert!(!authorized(&[
            user("show me the browser"),
            asking("Can I bring Safari to the front?", Some(&["No"])),
        ]));
    }

    #[test]
    fn a_dead_run_no_after_an_engine_restart_refuses_the_excursion() {
        let title = "Can I bring Safari to the front?";
        assert!(!authorized(&[
            user("show me the browser"),
            asking(title, Some(&["No"])),
            dead_run_answer("visibility-question", title, "No"),
        ]));
    }

    #[test]
    fn an_unanswered_input_is_not_a_walk_position() {
        assert!(authorized(&[
            user("show me the browser"),
            asking("Can I bring Safari to the front?", None),
        ]));
    }

    #[test]
    fn a_vietnamese_request_does_not_authorize_a_raise() {
        for text in [
            "cho tôi xem trình duyệt",
            "hiển thị cửa sổ Safari lên màn hình",
            "đưa Safari lên phía trước",
        ] {
            assert!(!authorized(&[user(text)]), "{text}");
        }
    }

    #[test]
    fn single_quote_stripping_keeps_the_closing_neighbour_for_the_next_quote() {
        assert_eq!(strip_single_quoted("a 'b' 'c' d"), "a   d");
        assert_eq!(strip_single_quoted("it's 'x'y"), "it's 'x'y");
        assert_eq!(strip_single_quoted("('quoted')"), "()");
    }

    fn doc_entry(role: MessageRole, parts: Vec<MessagePart>) -> SessionMessageEntry {
        SessionMessageEntry {
            id: "m".into(),
            role,
            parts,
            created_at: 1,
            device_id: "dev-a".into(),
            status: Some(MessageStatus::Complete),
            continuation_of: None,
            from_chat_id: None,
            fork_import: false,
            answers_request_id: None,
        }
    }

    fn text(text: &str) -> MessagePart {
        MessagePart::Text {
            id: "t0".into(),
            text: text.into(),
        }
    }

    fn input_part(
        request_id: &str,
        question: &str,
        kind: Option<zeron_proto::UserInputKind>,
        answer: Option<&str>,
    ) -> MessagePart {
        MessagePart::Input {
            id: request_id.into(),
            request_id: request_id.into(),
            questions: vec![UserInputQuestion {
                id: "q1".into(),
                header: "Q".into(),
                question: question.into(),
                options: vec!["Yes".into(), "No".into()],
                multi_select: false,
                kind,
            }],
            resolved: answer.is_some(),
            answers: answer.map(|label| {
                vec![zeron_proto::UserInputAnswer {
                    question_id: "q1".into(),
                    labels: vec![label.into()],
                }]
            }),
        }
    }

    #[test]
    fn builds_walk_entries_from_the_transcript() {
        let mut copied = doc_entry(MessageRole::User, vec![text("show me the browser")]);
        copied.fork_import = true;
        let mut asked = doc_entry(
            MessageRole::Assistant,
            vec![
                text("Working."),
                input_part("r1", "Can I bring Safari to the front?", None, Some("Yes")),
            ],
        );
        asked.status = Some(MessageStatus::Streaming);
        let note = doc_entry(MessageRole::System, vec![text("Computer use is on")]);
        let mut from_agent = doc_entry(MessageRole::User, vec![text("continue")]);
        from_agent.from_chat_id = Some("lead".into());
        let mut dead = doc_entry(
            MessageRole::User,
            vec![text("Answering your earlier question")],
        );
        dead.answers_request_id = Some("r1".into());

        let walk = visible_use_entries(&[copied, asked, note, from_agent, dead]);
        assert_eq!(walk.len(), 4);
        assert!(walk[0].fork_import && walk[0].role == VisibleUseRole::User);
        assert_eq!(walk[1].role, VisibleUseRole::Assistant);
        assert_eq!(walk[1].text, "Working.");
        assert!(walk[1].streaming);
        assert_eq!(
            walk[1].inputs,
            vec![VisibleUseInput {
                request_id: "r1".into(),
                question_titles: vec!["Can I bring Safari to the front?".into()],
                answers: Some(vec!["Yes".into()]),
            }]
        );
        assert_eq!(walk[2].from_chat_id.as_deref(), Some("lead"));
        assert_eq!(walk[3].answers_request_id.as_deref(), Some("r1"));
    }

    #[test]
    fn a_consent_card_is_never_a_visible_use_question() {
        let consent = Some(zeron_proto::UserInputKind::Consent {
            scope: zeron_proto::computer::ComputerConsentScope::Task,
        });
        let walk = visible_use_entries(&[
            doc_entry(MessageRole::User, vec![text("show me the browser")]),
            doc_entry(
                MessageRole::Assistant,
                vec![input_part(
                    "consent",
                    "Can I bring Safari to the front?",
                    consent,
                    Some("Allow"),
                )],
            ),
        ]);
        assert!(walk[1].inputs.is_empty());
        assert!(authorized(&walk));
    }

    #[test]
    fn transcript_walks_hold_the_fork_and_dead_run_barriers() {
        let mut parent = doc_entry(MessageRole::User, vec![text("show me the browser")]);
        parent.fork_import = true;
        let fork = [parent, doc_entry(MessageRole::User, vec![text("continue")])];
        assert!(!authorized(&visible_use_entries(&fork)));

        let title = "Can I bring Safari to the front?";
        let mut answer = doc_entry(
            MessageRole::User,
            vec![text(&format!(
                "Answering your earlier question: {title}\nNo"
            ))],
        );
        answer.answers_request_id = Some("r1".into());
        let dead = [
            doc_entry(MessageRole::User, vec![text("show me the browser")]),
            doc_entry(
                MessageRole::Assistant,
                vec![input_part("r1", title, None, Some("No"))],
            ),
            answer,
        ];
        assert!(!authorized(&visible_use_entries(&dead)));
    }
}
