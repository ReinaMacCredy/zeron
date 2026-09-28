use std::cell::RefCell;
use std::rc::Rc;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, Entity};
use zeron_engine::computer::engine_seams::{
    COMPUTER_FOREGROUND_ALLOW, COMPUTER_FOREGROUND_DECLINE,
};
use zeron_proto::UserInputAnswer;
use zeron_rpc::methods;

use super::presence::{ChatPresence, PresenceTone};
use super::strip;
use crate::state::AppState;

const TITLE_LIMIT: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuAction {
    OpenChat(String),
    Stop(String),
    Foreground {
        chat_id: String,
        input_id: String,
        question_id: String,
        allow: bool,
    },
    Consent {
        chat_id: String,
        input_id: String,
        question_id: String,
        label: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuRow {
    Header(String),
    Info(String),
    Separator,
    Action {
        label: String,
        hint: Option<String>,
        action: MenuAction,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusMenu {
    pub rows: Vec<MenuRow>,
    pub tone: PresenceTone,
}

fn short_title(title: Option<String>) -> String {
    let title = title
        .as_deref()
        .and_then(|title| title.lines().map(str::trim).find(|line| !line.is_empty()))
        .unwrap_or("New session");
    if title.chars().count() <= TITLE_LIMIT {
        return title.to_owned();
    }
    let cut: String = title.chars().take(TITLE_LIMIT - 1).collect();
    format!("{}…", cut.trim_end())
}

fn activity(presence: &ChatPresence) -> String {
    if let Some(ask) = strip::consent_request(presence) {
        return ask.sentence;
    }
    if strip::foreground_request(presence).is_some() {
        return format!(
            "{} needs to come to the front",
            presence.app.as_deref().unwrap_or("The app")
        );
    }
    let copy = strip::strip_copy(presence);
    match presence.tone {
        PresenceTone::Paused => copy.lead.unwrap_or(copy.detail),
        _ => copy.detail,
    }
}

pub fn status_menu(
    presences: &[ChatPresence],
    title: impl Fn(&str) -> Option<String>,
) -> Option<StatusMenu> {
    let holding: Vec<_> = presences
        .iter()
        .filter(|presence| presence.holds_desktop())
        .collect();
    let tone = [
        PresenceTone::NeedsYou,
        PresenceTone::Driving,
        PresenceTone::Paused,
    ]
    .into_iter()
    .find(|tone| holding.iter().any(|presence| presence.tone == *tone))?;
    let mut rows = vec![MenuRow::Header("Zeron is controlling this Mac".into())];
    for presence in holding {
        rows.push(MenuRow::Separator);
        rows.push(MenuRow::Info(format!(
            "{} · {}",
            short_title(title(&presence.chat_id)),
            presence.app.as_deref().unwrap_or("Desktop")
        )));
        rows.push(MenuRow::Info(activity(presence)));
        if let Some(ask) = strip::consent_request(presence) {
            for (_, label, answers) in strip::consent_buttons(&ask) {
                for answer in answers {
                    rows.push(MenuRow::Action {
                        label: label.clone(),
                        hint: None,
                        action: MenuAction::Consent {
                            chat_id: presence.chat_id.clone(),
                            input_id: ask.input_id.clone(),
                            question_id: answer.question_id,
                            label: answer.labels.concat(),
                        },
                    });
                }
            }
        }
        if let Some(pending) = strip::foreground_request(presence) {
            for (label, allow) in [
                (COMPUTER_FOREGROUND_ALLOW, true),
                (COMPUTER_FOREGROUND_DECLINE, false),
            ] {
                rows.push(MenuRow::Action {
                    label: label.into(),
                    hint: None,
                    action: MenuAction::Foreground {
                        chat_id: presence.chat_id.clone(),
                        input_id: pending.input_id.clone(),
                        question_id: pending.question_id.clone(),
                        allow,
                    },
                });
            }
        }
        rows.push(MenuRow::Action {
            label: "Open chat".into(),
            hint: None,
            action: MenuAction::OpenChat(presence.chat_id.clone()),
        });
        if strip::shows_stop(presence) {
            rows.push(MenuRow::Action {
                label: "Stop".into(),
                hint: Some("Esc".into()),
                action: MenuAction::Stop(presence.chat_id.clone()),
            });
        }
    }
    Some(StatusMenu { rows, tone })
}

fn revoke_desktop(state: &Entity<AppState>, chat_id: String, cx: &mut App) {
    let Some(engine) = state.read(cx).engine().cloned() else {
        return;
    };
    cx.spawn(async move |_| {
        if let Err(error) = engine
            .client()
            .call(
                methods::COMPUTER_REVOKE_DESKTOP,
                serde_json::json!({ "chatId": chat_id }),
            )
            .await
        {
            tracing::warn!(target: "desktop_cua", "revoke desktop from the menu bar: {error}");
        }
    })
    .detach();
}

pub(crate) fn answer_pending(
    state: &Entity<AppState>,
    chat_id: String,
    input_id: String,
    answers: Vec<UserInputAnswer>,
    cx: &mut App,
) {
    let Some(engine) = state.read(cx).engine().cloned() else {
        return;
    };
    super::presence::set_answered(&chat_id, &input_id, true, cx);
    cx.spawn(async move |cx| {
        if let Err(error) =
            crate::composer::respond_input(&engine, &chat_id, &input_id, answers).await
        {
            tracing::warn!(target: "desktop_cua", "answer the pending computer request: {error}");
            cx.update(|cx| super::presence::set_answered(&chat_id, &input_id, false, cx));
        }
    })
    .detach();
}

fn answer_foreground(
    state: &Entity<AppState>,
    chat_id: String,
    input_id: String,
    question_id: String,
    allow: bool,
    cx: &mut App,
) {
    let label = if allow {
        COMPUTER_FOREGROUND_ALLOW
    } else {
        COMPUTER_FOREGROUND_DECLINE
    };
    let answers = vec![UserInputAnswer {
        question_id,
        labels: vec![label.into()],
    }];
    answer_pending(state, chat_id, input_id, answers, cx);
}

pub fn drive(
    state: Entity<AppState>,
    open_chat: impl Fn(String, &mut App) + 'static,
    apply: impl Fn(Option<StatusMenu>, &mpsc::UnboundedSender<MenuAction>) + 'static,
    cx: &mut App,
) -> Option<mpsc::UnboundedSender<MenuAction>> {
    let map = super::presence::map(cx)?;
    let (actions, mut received) = mpsc::unbounded::<MenuAction>();
    let applied: Rc<RefCell<Option<StatusMenu>>> = Rc::default();
    let refresh = Rc::new({
        let (map, state, actions) = (map.clone(), state.clone(), actions.clone());
        move |cx: &mut App| {
            let chats = state.read(cx);
            let model = status_menu(map.read(cx).chats(), |chat_id| {
                chats
                    .chats
                    .iter()
                    .find(|chat| chat.id == chat_id)
                    .and_then(|chat| chat.title.clone())
            });
            if *applied.borrow() != model {
                applied.replace(model.clone());
                apply(model, &actions);
            }
        }
    });
    cx.observe(&map, {
        let refresh = refresh.clone();
        move |_, cx| refresh(cx)
    })
    .detach();
    cx.observe(&state, move |_, cx| refresh(cx)).detach();
    cx.spawn(async move |cx| {
        while let Some(action) = received.next().await {
            cx.update(|cx| match action {
                MenuAction::OpenChat(chat_id) => open_chat(chat_id, cx),
                MenuAction::Stop(chat_id) => revoke_desktop(&state, chat_id, cx),
                MenuAction::Foreground {
                    chat_id,
                    input_id,
                    question_id,
                    allow,
                } => answer_foreground(&state, chat_id, input_id, question_id, allow, cx),
                MenuAction::Consent {
                    chat_id,
                    input_id,
                    question_id,
                    label,
                } => answer_pending(
                    &state,
                    chat_id,
                    input_id,
                    vec![UserInputAnswer {
                        question_id,
                        labels: vec![label],
                    }],
                    cx,
                ),
            });
        }
    })
    .detach();
    Some(actions)
}

#[cfg(test)]
mod tests {
    use zeron_proto::computer::{ComputerEvent, ComputerInputPause};

    use super::*;
    use crate::computer::presence::presences;
    use crate::computer::preview::ComputerPreviewStore;
    use crate::computer::preview::tests::{driving, thread_state};

    fn presence(chat: &str, tone: PresenceTone) -> ChatPresence {
        ChatPresence {
            chat_id: chat.into(),
            app: Some("Ghostty".into()),
            window_id: Some("cua:42:7".into()),
            activity: Some("Typing".into()),
            tone,
            user_takeover: false,
            pending: None,
        }
    }

    fn titles(chat: &str) -> Option<String> {
        match chat {
            "a" => Some("Agent Orchestration for the release train".into()),
            "b" => Some("Fix the build".into()),
            _ => None,
        }
    }

    fn info(text: &str) -> MenuRow {
        MenuRow::Info(text.into())
    }

    fn open(chat: &str) -> MenuRow {
        MenuRow::Action {
            label: "Open chat".into(),
            hint: None,
            action: MenuAction::OpenChat(chat.into()),
        }
    }

    fn stop(chat: &str) -> MenuRow {
        MenuRow::Action {
            label: "Stop".into(),
            hint: Some("Esc".into()),
            action: MenuAction::Stop(chat.into()),
        }
    }

    #[test]
    fn no_menu_when_nothing_drives() {
        assert_eq!(status_menu(&[], titles), None);
        let revoked = [
            presence("a", PresenceTone::Revoked { summing_up: true }),
            presence("b", PresenceTone::Revoked { summing_up: false }),
        ];
        assert_eq!(status_menu(&revoked, titles), None);
    }

    #[test]
    fn one_block_per_driving_chat_with_open_and_stop() {
        let mut paused = presence("b", PresenceTone::Paused);
        paused.app = Some("Calculator".into());
        let menu = status_menu(
            &[
                presence("a", PresenceTone::Driving),
                paused.clone(),
                presence("c", PresenceTone::Revoked { summing_up: true }),
            ],
            titles,
        )
        .expect("menu");
        assert_eq!(menu.tone, PresenceTone::Driving);
        assert_eq!(
            menu.rows,
            vec![
                MenuRow::Header("Zeron is controlling this Mac".into()),
                MenuRow::Separator,
                info("Agent Orchestration for the rel… · Ghostty"),
                info("Typing"),
                open("a"),
                stop("a"),
                MenuRow::Separator,
                info("Fix the build · Calculator"),
                info("Calculator isn't reachable"),
                open("b"),
            ]
        );
        let menu = status_menu(&[paused.clone()], titles).unwrap();
        assert_eq!(
            menu.tone,
            PresenceTone::Paused,
            "only paused chats: warning"
        );
        let taken = ChatPresence {
            user_takeover: true,
            ..paused
        };
        assert_eq!(
            status_menu(&[taken], titles).unwrap().rows[2..],
            [
                info("Fix the build · Calculator"),
                info("You're using Calculator"),
                open("b"),
            ],
            "no Stop while the user holds the window"
        );
        let bare = ChatPresence {
            app: None,
            activity: None,
            ..presence("z", PresenceTone::NeedsYou)
        };
        let menu = status_menu(&[presence("a", PresenceTone::Driving), bare], titles).unwrap();
        assert_eq!(menu.tone, PresenceTone::NeedsYou);
        assert_eq!(
            &menu.rows[6..],
            &[
                MenuRow::Separator,
                info("New session · Desktop"),
                info("Working"),
                open("z"),
            ],
            "no Stop while a chat waits for the user"
        );
    }

    #[test]
    fn a_foreground_request_offers_allow_and_no_with_its_ids() {
        let asking = ChatPresence {
            pending: Some(crate::computer::strip::tests::foreground_pending(Some(
                "Ghostty",
            ))),
            ..presence("b", PresenceTone::NeedsYou)
        };
        let menu = status_menu(&[presence("a", PresenceTone::Driving), asking], titles).unwrap();
        assert_eq!(menu.tone, PresenceTone::NeedsYou);
        let answer = |label: &str, allow| MenuRow::Action {
            label: label.into(),
            hint: None,
            action: MenuAction::Foreground {
                chat_id: "b".into(),
                input_id: "computer-foreground-1".into(),
                question_id: "foreground".into(),
                allow,
            },
        };
        assert_eq!(
            &menu.rows[6..],
            &[
                MenuRow::Separator,
                info("Fix the build · Ghostty"),
                info("Ghostty needs to come to the front"),
                answer("Allow for this turn", true),
                answer("No", false),
                open("b"),
            ]
        );
    }

    #[test]
    fn a_consent_request_offers_the_strip_choices_with_its_ids() {
        use zeron_proto::computer::ComputerConsentScope;
        let asking = ChatPresence {
            pending: Some(crate::computer::strip::tests::consent_pending(
                ComputerConsentScope::Task,
                Some("Ghostty"),
                true,
            )),
            ..presence("b", PresenceTone::NeedsYou)
        };
        let menu = status_menu(&[asking], titles).unwrap();
        assert_eq!(menu.tone, PresenceTone::NeedsYou);
        let answer = |label: &str, engine: &str| MenuRow::Action {
            label: label.into(),
            hint: None,
            action: MenuAction::Consent {
                chat_id: "b".into(),
                input_id: "computer-consent-1".into(),
                question_id: "consent".into(),
                label: engine.into(),
            },
        };
        assert_eq!(
            &menu.rows[1..],
            &[
                MenuRow::Separator,
                info("Fix the build · Ghostty"),
                info("Type in Ghostty"),
                answer("Allow for this task", "Allow"),
                answer("Always allow Ghostty", "Always allow Ghostty"),
                answer("No", "Deny"),
                open("b"),
            ]
        );
    }

    #[gpui::test]
    fn the_menu_follows_presence_and_titles_and_its_actions_reach_the_engine(
        cx: &mut gpui::TestAppContext,
    ) {
        use gpui::AppContext;

        use crate::computer::preview;

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        cx.update(preview::init);
        let (out, mut frames) = tokio::sync::mpsc::channel::<String>(16);
        let (replies, inbound) = tokio::sync::mpsc::channel::<String>(16);
        std::mem::forget(replies);
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| {
            state.set_test_engine(crate::state::EngineHandle::from_test_client(
                zeron_rpc::RpcClient::new(out, inbound),
            ));
            state.chats = vec![
                serde_json::from_value(serde_json::json!({
                    "id": "a",
                    "deviceId": "mac",
                    "title": "Ship it",
                    "archived": false,
                    "createdAt": "2026-09-23T00:00:00Z"
                }))
                .unwrap(),
            ];
        });
        let applied: Rc<RefCell<Vec<Option<StatusMenu>>>> = Rc::default();
        let opened: Rc<RefCell<Vec<String>>> = Rc::default();
        let actions = cx
            .update(|cx| {
                drive(
                    state.clone(),
                    {
                        let opened = opened.clone();
                        move |chat_id, _| opened.borrow_mut().push(chat_id)
                    },
                    {
                        let applied = applied.clone();
                        move |menu, _| applied.borrow_mut().push(menu)
                    },
                    cx,
                )
            })
            .expect("presence map");
        let store = cx.update(|cx| preview::store(cx).unwrap());
        let apply = |cx: &mut gpui::TestAppContext, event: ComputerEvent| {
            cx.update(|cx| preview::update_store(&store, cx, |store| store.apply_event(&event)));
            cx.run_until_parked();
        };
        let info_rows = |menu: &Option<StatusMenu>| -> Vec<String> {
            menu.iter()
                .flat_map(|menu| menu.rows.iter())
                .filter_map(|row| match row {
                    MenuRow::Info(text) => Some(text.clone()),
                    _ => None,
                })
                .collect()
        };

        let mut typing = thread_state("a", true);
        typing.activity = Some("Typing".into());
        apply(
            cx,
            ComputerEvent::ThreadState {
                state: Box::new(typing),
            },
        );
        apply(
            cx,
            ComputerEvent::Action {
                window_id: Some("w1".into()),
                delivery: None,
                action: "computer_click".into(),
                ok: true,
                message: None,
                thread_id: Some("a".into()),
                restored_window_id: None,
                restore_status: None,
                masked: false,
            },
        );
        assert_eq!(
            info_rows(applied.borrow().last().unwrap()),
            vec!["Ship it · Calculator", "Typing"]
        );
        let count = applied.borrow().len();
        cx.update(|cx| {
            preview::update_store(&store, cx, |store| {
                store.accept_tap_frame("a", crate::computer::preview::tests::image(1, 960, 600))
            })
        });
        state.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        assert_eq!(
            applied.borrow().len(),
            count,
            "frames and unrelated state changes reapply nothing"
        );

        state.update(cx, |state, cx| {
            state.chats[0].title = Some("Renamed".into());
            cx.notify();
        });
        cx.run_until_parked();
        assert_eq!(
            info_rows(applied.borrow().last().unwrap())[0],
            "Renamed · Calculator"
        );

        actions
            .unbounded_send(MenuAction::Stop("a".into()))
            .unwrap();
        actions
            .unbounded_send(MenuAction::OpenChat("a".into()))
            .unwrap();
        actions
            .unbounded_send(MenuAction::Foreground {
                chat_id: "a".into(),
                input_id: "computer-foreground-3".into(),
                question_id: "foreground".into(),
                allow: false,
            })
            .unwrap();
        actions
            .unbounded_send(MenuAction::Consent {
                chat_id: "a".into(),
                input_id: "computer-consent-4".into(),
                question_id: "consent".into(),
                label: "Always allow Ghostty".into(),
            })
            .unwrap();
        cx.run_until_parked();
        let mut revokes = Vec::new();
        let mut answers = Vec::new();
        while let Ok(frame) = frames.try_recv() {
            let frame: serde_json::Value = serde_json::from_str(&frame).unwrap();
            if frame["method"] == methods::COMPUTER_REVOKE_DESKTOP {
                revokes.push(frame["params"].clone());
            }
            if frame["method"] == methods::QUEUE_COMMAND {
                answers.push(frame["params"].clone());
            }
        }
        assert_eq!(revokes, vec![serde_json::json!({ "chatId": "a" })]);
        assert_eq!(answers.len(), 2, "{answers:?}");
        assert_eq!(
            answers[0]["command"],
            serde_json::to_value(zeron_doc::SessionCommandPayload::RespondInput {
                request_id: "computer-foreground-3".into(),
                answers: vec![UserInputAnswer {
                    question_id: "foreground".into(),
                    labels: vec!["No".into()],
                }],
            })
            .unwrap()
        );
        assert_eq!(
            answers[1]["command"],
            serde_json::to_value(zeron_doc::SessionCommandPayload::RespondInput {
                request_id: "computer-consent-4".into(),
                answers: vec![UserInputAnswer {
                    question_id: "consent".into(),
                    labels: vec!["Always allow Ghostty".into()],
                }],
            })
            .unwrap()
        );
        assert_eq!(*opened.borrow(), vec!["a".to_string()]);
        cx.update(|cx| {
            for input in ["computer-foreground-3", "computer-consent-4"] {
                assert!(
                    crate::computer::presence::answered("a", input, cx),
                    "{input}: a menu answer marks the id as answered here"
                );
            }
            assert!(!crate::computer::presence::answered(
                "b",
                "computer-consent-4",
                cx
            ));
        });

        apply(
            cx,
            ComputerEvent::ThreadState {
                state: Box::new(thread_state("a", false)),
            },
        );
        assert_eq!(
            applied.borrow().last(),
            Some(&None),
            "idle removes the item"
        );
    }

    #[test]
    fn labels_never_carry_tool_ids_or_typed_text() {
        let mut store = ComputerPreviewStore::default();
        driving(&mut store, "a");
        store.apply_event(&ComputerEvent::Action {
            window_id: Some("cua:4242:9191".into()),
            delivery: None,
            action: "computer_type_text".into(),
            ok: true,
            message: Some("typed hunter2-secret".into()),
            thread_id: Some("a".into()),
            restored_window_id: None,
            restore_status: None,
            masked: false,
        });
        let mut typing = thread_state("a", true);
        typing.activity = Some("Typing".into());
        typing.input_pause = Some(ComputerInputPause {
            window_id: Some("cua:4242:9191".into()),
            pid: Some(4242),
            message: "computer_type_text refused for hunter2-secret".into(),
            user_takeover: false,
        });
        store.apply_event(&ComputerEvent::ThreadState {
            state: Box::new(typing),
        });
        let mut all = presences(&store);
        assert_eq!(all[0].tone, PresenceTone::Paused);
        let mut texts = Vec::new();
        for (tone, user_takeover) in [
            (PresenceTone::Paused, false),
            (PresenceTone::Paused, true),
            (PresenceTone::Driving, false),
            (PresenceTone::NeedsYou, false),
        ] {
            all[0].tone = tone;
            all[0].user_takeover = user_takeover;
            all[0].pending = (tone == PresenceTone::NeedsYou)
                .then(|| crate::computer::strip::tests::foreground_pending(Some("Calculator")));
            let menu = status_menu(&all, |_| Some("Ship it".into())).unwrap();
            texts.extend(menu.rows.into_iter().filter_map(|row| match row {
                MenuRow::Header(text) | MenuRow::Info(text) => Some(text),
                MenuRow::Action { label, hint, .. } => {
                    Some(format!("{label} {}", hint.unwrap_or_default()))
                }
                MenuRow::Separator => None,
            }));
            all[0].pending = (tone == PresenceTone::NeedsYou).then(|| {
                crate::computer::strip::tests::consent_pending(
                    zeron_proto::computer::ComputerConsentScope::Task,
                    Some("Calculator"),
                    true,
                )
            });
            let menu = status_menu(&all, |_| Some("Ship it".into())).unwrap();
            texts.extend(menu.rows.into_iter().filter_map(|row| match row {
                MenuRow::Header(text) | MenuRow::Info(text) => Some(text),
                MenuRow::Action { label, hint, .. } => {
                    Some(format!("{label} {}", hint.unwrap_or_default()))
                }
                MenuRow::Separator => None,
            }));
        }
        assert!(
            texts.contains(&"Ship it · Desktop".to_string()),
            "{texts:?}"
        );
        for text in texts {
            for secret in [
                "computer_",
                "hunter2",
                "4242",
                "9191",
                "cua:",
                "com.mitchellh",
            ] {
                assert!(!text.contains(secret), "{text} carries {secret}");
            }
        }
    }
}
