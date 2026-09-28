use std::cell::RefCell;
use std::rc::Rc;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, Entity};
use zeron_rpc::methods;

use super::presence::{ChatPresence, PresenceTone};
use super::strip;
use crate::state::AppState;

const TITLE_LIMIT: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuAction {
    OpenChat(String),
    Stop(String),
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
        let menu = status_menu(&[paused], titles).unwrap();
        assert_eq!(
            menu.tone,
            PresenceTone::Paused,
            "only paused chats: warning"
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
        cx.run_until_parked();
        let mut revokes = Vec::new();
        while let Ok(frame) = frames.try_recv() {
            let frame: serde_json::Value = serde_json::from_str(&frame).unwrap();
            if frame["method"] == methods::COMPUTER_REVOKE_DESKTOP {
                revokes.push(frame["params"].clone());
            }
        }
        assert_eq!(revokes, vec![serde_json::json!({ "chatId": "a" })]);
        assert_eq!(*opened.borrow(), vec!["a".to_string()]);

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
        });
        store.apply_event(&ComputerEvent::ThreadState {
            state: Box::new(typing),
        });
        let mut all = presences(&store);
        assert_eq!(all[0].tone, PresenceTone::Paused);
        let mut texts = Vec::new();
        for tone in [
            PresenceTone::Paused,
            PresenceTone::Driving,
            PresenceTone::NeedsYou,
        ] {
            all[0].tone = tone;
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
            for secret in ["computer_", "hunter2", "4242", "9191", "cua:"] {
                assert!(!text.contains(secret), "{text} carries {secret}");
            }
        }
    }
}
