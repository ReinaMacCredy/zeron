use std::cell::RefCell;
use std::rc::Rc;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, Entity};
use zeron_rpc::methods;

use super::presence::{ChatPresence, DrivenApp};
use crate::state::AppState;

pub const MENU_TITLE: &str = "Computer Use Controls";
pub const NO_ACTIVE_SESSIONS: &str = "No Active Sessions";
const TITLE_LIMIT: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuAction {
    OpenChat(String),
    StopApp { app: DrivenApp, chats: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuRow {
    Header(String),
    Info(String),
    Separator,
    Action {
        label: String,
        symbol: Option<&'static str>,
        action: MenuAction,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusMenu {
    pub rows: Vec<MenuRow>,
    pub apps: Vec<DrivenApp>,
}

impl StatusMenu {
    pub fn active(&self) -> bool {
        self.rows
            .iter()
            .any(|row| matches!(row, MenuRow::Action { .. }))
    }
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

pub fn status_menu(
    presences: &[ChatPresence],
    title: impl Fn(&str) -> Option<String>,
    enabled: bool,
) -> Option<StatusMenu> {
    let holding: Vec<_> = presences
        .iter()
        .filter(|presence| presence.holds_desktop())
        .collect();
    if holding.is_empty() {
        return enabled.then(|| StatusMenu {
            rows: vec![
                MenuRow::Header(MENU_TITLE.into()),
                MenuRow::Separator,
                MenuRow::Info(NO_ACTIVE_SESSIONS.into()),
            ],
            apps: Vec::new(),
        });
    }
    let mut apps: Vec<(DrivenApp, Vec<String>)> = Vec::new();
    for presence in &holding {
        for app in &presence.apps {
            match apps.iter_mut().find(|(known, _)| known.same_app(&app.name)) {
                Some((_, chats)) => chats.push(presence.chat_id.clone()),
                None => apps.push((app.clone(), vec![presence.chat_id.clone()])),
            }
        }
    }
    let mut rows = vec![MenuRow::Header(MENU_TITLE.into())];
    if !apps.is_empty() {
        rows.push(MenuRow::Separator);
    }
    for (app, chats) in &apps {
        rows.push(MenuRow::Action {
            label: format!("Stop Using {}", app.name),
            symbol: Some("stop.circle"),
            action: MenuAction::StopApp {
                app: app.clone(),
                chats: chats.clone(),
            },
        });
    }
    rows.push(MenuRow::Separator);
    let several = holding.len() > 1;
    for presence in &holding {
        rows.push(MenuRow::Action {
            label: if several {
                format!("Open chat · {}", short_title(title(&presence.chat_id)))
            } else {
                "Open chat".into()
            },
            symbol: None,
            action: MenuAction::OpenChat(presence.chat_id.clone()),
        });
    }
    Some(StatusMenu {
        rows,
        apps: apps.into_iter().map(|(app, _)| app).collect(),
    })
}

fn stop_app(state: &Entity<AppState>, app: DrivenApp, chats: Vec<String>, cx: &mut App) {
    if let Some(store) = super::preview::store(cx) {
        super::preview::update_store(&store, cx, |store| {
            chats.iter().fold(false, |changed, chat_id| {
                store.stop_app(chat_id, &app.name) | changed
            })
        });
    }
    let Some(engine) = state.read(cx).engine().cloned() else {
        return;
    };
    for chat_id in chats {
        let (engine, params) = (
            engine.clone(),
            serde_json::json!({
                "chatId": chat_id,
                "app": { "name": app.name, "pid": app.pid },
            }),
        );
        cx.spawn(async move |_| {
            if let Err(error) = engine
                .client()
                .call(methods::COMPUTER_REVOKE_DESKTOP, params)
                .await
            {
                tracing::warn!(target: "desktop_cua", "stop using an app from the menu bar: {error}");
            }
        })
        .detach();
    }
}

pub fn drive(
    state: Entity<AppState>,
    open_chat: impl Fn(String, &mut App) + 'static,
    apply: impl Fn(Option<StatusMenu>, &mpsc::UnboundedSender<MenuAction>) + 'static,
    cx: &mut App,
) -> Option<mpsc::UnboundedSender<MenuAction>> {
    let map = super::presence::map(cx)?;
    let (actions, mut received) = mpsc::unbounded::<MenuAction>();
    let applied: Rc<RefCell<Option<Option<StatusMenu>>>> = Rc::default();
    let refresh = Rc::new({
        let (map, state, actions) = (map.clone(), state.clone(), actions.clone());
        move |cx: &mut App| {
            let chats = state.read(cx);
            let enabled = chats
                .computer_settings
                .as_ref()
                .is_some_and(|settings| settings.computer_control_enabled);
            let model = status_menu(
                map.read(cx).chats(),
                |chat_id| {
                    chats
                        .chats
                        .iter()
                        .find(|chat| chat.id == chat_id)
                        .and_then(|chat| chat.title.clone())
                },
                enabled,
            );
            if applied.borrow().as_ref() != Some(&model) {
                applied.replace(Some(model.clone()));
                apply(model, &actions);
            }
        }
    });
    cx.observe(&map, {
        let refresh = refresh.clone();
        move |_, cx| refresh(cx)
    })
    .detach();
    cx.observe(&state, {
        let refresh = refresh.clone();
        move |_, cx| refresh(cx)
    })
    .detach();
    refresh(cx);
    cx.spawn(async move |cx| {
        while let Some(action) = received.next().await {
            cx.update(|cx| match action {
                MenuAction::OpenChat(chat_id) => open_chat(chat_id, cx),
                MenuAction::StopApp { app, chats } => stop_app(&state, app, chats, cx),
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
    use crate::computer::presence::{PresenceTone, presences};
    use crate::computer::preview::ComputerPreviewStore;
    use crate::computer::preview::tests::{driving, thread_state};

    fn app(name: &str, pid: u32) -> DrivenApp {
        DrivenApp {
            name: name.into(),
            pid: Some(pid),
        }
    }

    fn presence(chat: &str, tone: PresenceTone, apps: Vec<DrivenApp>) -> ChatPresence {
        ChatPresence {
            chat_id: chat.into(),
            app: apps.first().map(|app| app.name.clone()),
            window_id: Some("cua:42:7".into()),
            tone,
            user_takeover: false,
            pending: None,
            apps,
        }
    }

    fn titles(chat: &str) -> Option<String> {
        match chat {
            "a" => Some("Agent Orchestration for the release train".into()),
            "b" => Some("Fix the build".into()),
            _ => None,
        }
    }

    fn stop(app: DrivenApp, chats: &[&str]) -> MenuRow {
        MenuRow::Action {
            label: format!("Stop Using {}", app.name),
            symbol: Some("stop.circle"),
            action: MenuAction::StopApp {
                app,
                chats: chats.iter().map(|chat| chat.to_string()).collect(),
            },
        }
    }

    fn open(label: &str, chat: &str) -> MenuRow {
        MenuRow::Action {
            label: label.into(),
            symbol: None,
            action: MenuAction::OpenChat(chat.into()),
        }
    }

    #[test]
    fn idle_reads_no_active_sessions_only_while_computer_use_is_on() {
        let idle = [
            presence("a", PresenceTone::Revoked { summing_up: true }, Vec::new()),
            presence("b", PresenceTone::Revoked { summing_up: false }, Vec::new()),
        ];
        assert_eq!(status_menu(&idle, titles, false), None);
        let menu = status_menu(&idle, titles, true).expect("the idle menu");
        assert_eq!(
            menu.rows,
            vec![
                MenuRow::Header("Computer Use Controls".into()),
                MenuRow::Separator,
                MenuRow::Info("No Active Sessions".into()),
            ]
        );
        assert!(!menu.active());
        assert!(menu.apps.is_empty());
    }

    #[test]
    fn one_stop_per_driven_app_for_every_chat_driving_it() {
        let calculator = app("Calculator", 11);
        let ghostty = app("Ghostty", 22);
        let menu = status_menu(
            &[
                presence(
                    "a",
                    PresenceTone::Driving,
                    vec![calculator.clone(), ghostty.clone()],
                ),
                presence("b", PresenceTone::Paused, vec![app("calculator", 11)]),
                presence(
                    "c",
                    PresenceTone::Revoked { summing_up: true },
                    vec![app("Notes", 33)],
                ),
            ],
            titles,
            false,
        )
        .expect("menu");
        assert!(menu.active());
        assert_eq!(
            menu.rows,
            vec![
                MenuRow::Header("Computer Use Controls".into()),
                MenuRow::Separator,
                stop(calculator.clone(), &["a", "b"]),
                stop(ghostty.clone(), &["a"]),
                MenuRow::Separator,
                open("Open chat · Agent Orchestration for the rel…", "a"),
                open("Open chat · Fix the build", "b"),
            ]
        );
        assert_eq!(menu.apps, vec![calculator, ghostty], "the icon stack");
        let single = status_menu(
            &[presence("b", PresenceTone::NeedsYou, Vec::new())],
            titles,
            true,
        )
        .unwrap();
        assert_eq!(
            single.rows,
            vec![
                MenuRow::Header("Computer Use Controls".into()),
                MenuRow::Separator,
                open("Open chat", "b"),
            ],
            "a chat with no app yet still opens"
        );
    }

    #[gpui::test]
    fn stop_using_an_app_revokes_only_that_app_in_every_chat_driving_it(
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
            state.chats = ["a", "b"]
                .into_iter()
                .map(|id| {
                    serde_json::from_value(serde_json::json!({
                        "id": id,
                        "deviceId": "mac",
                        "title": format!("Ship {id}"),
                        "archived": false,
                        "createdAt": "2026-09-23T00:00:00Z"
                    }))
                    .unwrap()
                })
                .collect();
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
        let click = |thread: &str, window: &str| ComputerEvent::Action {
            window_id: Some(window.into()),
            delivery: None,
            action: "computer_click".into(),
            ok: true,
            message: None,
            thread_id: Some(thread.into()),
            restored_window_id: None,
            restore_status: None,
            masked: false,
        };
        let stops = |menu: &Option<StatusMenu>| -> Vec<String> {
            menu.iter()
                .flat_map(|menu| menu.rows.iter())
                .filter_map(|row| match row {
                    MenuRow::Action {
                        label,
                        action: MenuAction::StopApp { .. },
                        ..
                    } => Some(label.clone()),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(
            applied.borrow().last(),
            Some(&None),
            "off and idle: no item"
        );

        for chat in ["a", "b"] {
            let mut active = thread_state(chat, true);
            active.windows[0].id = "cua:4242:9".into();
            active.windows.push(
                serde_json::from_value(serde_json::json!({
                    "id": "cua:77:5", "title": "Term", "appName": "Ghostty", "pid": 77,
                    "focused": false, "minimized": false, "visible": true
                }))
                .unwrap(),
            );
            apply(
                cx,
                ComputerEvent::ThreadState {
                    state: Box::new(active),
                },
            );
            apply(cx, click(chat, "cua:4242:9"));
        }
        apply(cx, click("a", "cua:77:5"));
        assert_eq!(
            stops(applied.borrow().last().unwrap()),
            vec!["Stop Using Calculator", "Stop Using Ghostty"]
        );

        let calculator = applied
            .borrow()
            .last()
            .unwrap()
            .as_ref()
            .unwrap()
            .rows
            .iter()
            .find_map(|row| match row {
                MenuRow::Action {
                    action: action @ MenuAction::StopApp { app, .. },
                    ..
                } if app.name == "Calculator" => Some(action.clone()),
                _ => None,
            })
            .unwrap();
        actions.unbounded_send(calculator).unwrap();
        actions
            .unbounded_send(MenuAction::OpenChat("b".into()))
            .unwrap();
        cx.run_until_parked();
        let mut revokes = Vec::new();
        while let Ok(frame) = frames.try_recv() {
            let frame: serde_json::Value = serde_json::from_str(&frame).unwrap();
            if frame["method"] == methods::COMPUTER_REVOKE_DESKTOP {
                revokes.push(frame["params"].clone());
            }
        }
        assert_eq!(
            revokes,
            vec![
                serde_json::json!({ "chatId": "a", "app": { "name": "Calculator", "pid": 4242 } }),
                serde_json::json!({ "chatId": "b", "app": { "name": "Calculator", "pid": 4242 } }),
            ],
            "one per chat driving it, each naming only that app"
        );
        assert_eq!(*opened.borrow(), vec!["b".to_string()]);
        assert_eq!(
            stops(applied.borrow().last().unwrap()),
            vec!["Stop Using Ghostty"],
            "the stopped app leaves the menu; the other stays"
        );

        for chat in ["a", "b"] {
            apply(
                cx,
                ComputerEvent::ThreadState {
                    state: Box::new(thread_state(chat, false)),
                },
            );
        }
        assert_eq!(applied.borrow().last(), Some(&None), "idle and off");
        state.update(cx, |state, cx| {
            state.computer_settings = Some(zeron_proto::computer::ComputerSettings {
                computer_control_enabled: true,
                ..Default::default()
            });
            cx.notify();
        });
        cx.run_until_parked();
        let idle = applied
            .borrow()
            .last()
            .cloned()
            .flatten()
            .expect("idle menu");
        assert_eq!(idle.rows[2], MenuRow::Info("No Active Sessions".into()));
    }

    #[test]
    fn labels_never_carry_tool_ids_or_typed_text() {
        let mut store = ComputerPreviewStore::default();
        driving(&mut store, "a");
        store.apply_event(&ComputerEvent::Action {
            window_id: Some("w1".into()),
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
            window_id: Some("w1".into()),
            pid: Some(4242),
            message: "computer_type_text refused for hunter2-secret".into(),
            user_takeover: false,
        });
        store.apply_event(&ComputerEvent::ThreadState {
            state: Box::new(typing),
        });
        let all = presences(&store);
        let menu = status_menu(&all, |_| Some("Ship it".into()), true).unwrap();
        let texts: Vec<String> = menu
            .rows
            .into_iter()
            .filter_map(|row| match row {
                MenuRow::Header(text) | MenuRow::Info(text) => Some(text),
                MenuRow::Action { label, .. } => Some(label),
                MenuRow::Separator => None,
            })
            .collect();
        assert!(
            texts.contains(&"Stop Using Calculator".to_string()),
            "{texts:?}"
        );
        for text in texts {
            for secret in ["computer_", "hunter2", "4242", "cua:"] {
                assert!(!text.contains(secret), "{text} carries {secret}");
            }
        }
    }
}
