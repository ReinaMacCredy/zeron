//! Typing while no input holds focus lands in the composer.
use super::*;
use gpui::{AppContext, TestAppContext, VisualTestContext};

struct TypingHost {
    shell: Entity<Shell>,
    other: FocusHandle,
}

impl Render for TypingHost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let other = self.other.clone();
        self.shell.update(cx, |shell, cx| {
            div()
                .size_full()
                .track_focus(&shell.shortcut_focus)
                .child(div().track_focus(&shell.unfocused))
                .on_key_down(cx.listener(Shell::on_key_down))
                .child(div().track_focus(&other).w(px(10.0)).h(px(10.0)))
                .child(
                    div()
                        .track_focus(&shell.navigation_focus.main)
                        .w(px(600.0))
                        .h(px(400.0))
                        .child(shell.composer.clone()),
                )
        })
    }
}

fn setup(cx: &mut TestAppContext) -> (Entity<TypingHost>, &mut VisualTestContext) {
    let dir = tempfile::tempdir().unwrap();
    cx.update(|cx| {
        gpui_base::init(cx);
        cx.set_global(Theme::default());
        crate::app_menus::init(cx);
        settings::init(UiSettings::default(), dir.path(), cx);
        crate::history::init(
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            cx,
        );
        apply_keymap(
            cx,
            &KeymapConfig::default(),
            ComposerSendBehavior::default(),
            Default::default(),
        );
    });
    let (host, cx) = cx.add_window_view(|_, cx| TypingHost {
        shell: cx.new(|cx| {
            let state = cx.new(|_| AppState::new());
            let mut shell = Shell::new(
                state,
                EngineBootConfig {
                    data_dir: dir.path().into(),
                    ipc_port: 0,
                    edge_url: "http://127.0.0.1:1".into(),
                    edge_token: None,
                    org_id: None,
                    workos_client_id: None,
                    default_harness: zeron_proto::HarnessId::Mock,
                },
                cx,
            );
            shell.reduced_motion = true;
            shell
        }),
        other: cx.focus_handle(),
    });
    cx.update(|window, cx| {
        window.activate_window();
        window.draw(cx).clear();
    });
    (host, cx)
}

fn focus(cx: &mut VisualTestContext, handle: impl Fn(&TypingHost, &App) -> FocusHandle) {
    cx.update(|window, cx| {
        let host = window.root::<TypingHost>().flatten().unwrap();
        let handle = handle(host.read(cx), cx);
        window.focus(&handle, cx);
        window.draw(cx).clear();
    });
}

fn composer_text(host: &Entity<TypingHost>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| {
        let composer = host.read(cx).shell.read(cx).composer.clone();
        composer.read(cx).input_text(cx)
    })
}

fn composer_focused(host: &Entity<TypingHost>, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| {
        let composer = host.read(cx).shell.read(cx).composer.clone();
        composer.focus_handle(cx).is_focused(window)
    })
}

#[gpui::test]
fn typing_from_a_parked_focus_moves_the_character_into_the_composer(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    for parked in [
        |host: &TypingHost, cx: &App| host.shell.read(cx).unfocused.clone(),
        |host: &TypingHost, cx: &App| host.shell.read(cx).navigation_focus.main.clone(),
    ] {
        focus(cx, parked);
        assert!(!composer_focused(&host, cx));
        cx.simulate_keystrokes("h");
        assert!(composer_focused(&host, cx));
    }
    assert_eq!(composer_text(&host, cx), "hh");
}

fn shell_window<'a>(
    dir: &std::path::Path,
    cx: &'a mut TestAppContext,
) -> (Entity<Shell>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_base::init(cx);
        cx.set_global(Theme::default());
        crate::app_menus::init(cx);
        settings::init(UiSettings::default(), dir, cx);
        crate::history::init(
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            cx,
        );
        apply_keymap(
            cx,
            &KeymapConfig::default(),
            ComposerSendBehavior::default(),
            Default::default(),
        );
    });
    let (shell, cx) = cx.add_window_view(|_, cx| {
        let state = cx.new(|_| {
            let mut state = AppState::new();
            state.connection = zeron_proto::view::ConnectionStatus::Ready;
            state
        });
        let mut shell = Shell::new(
            state,
            EngineBootConfig {
                data_dir: dir.into(),
                ipc_port: 0,
                edge_url: "http://127.0.0.1:1".into(),
                edge_token: None,
                org_id: None,
                workos_client_id: None,
                default_harness: zeron_proto::HarnessId::Mock,
            },
            cx,
        );
        shell.reduced_motion = true;
        shell.debug_gate = Some(GatePhase::Ready);
        shell.settings.sidebar_collapsed = false;
        shell
    });
    cx.simulate_resize(gpui::size(px(1600.0), px(900.0)));
    cx.update(|window, cx| {
        window.activate_window();
        window.draw(cx).clear();
    });
    cx.run_until_parked();
    (shell, cx)
}

#[gpui::test]
fn typing_on_a_rail_control_reaches_the_composer(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (shell, cx) = shell_window(dir.path(), cx);
    let help = cx.debug_bounds("rail-help").expect("rail help");
    cx.simulate_click(help.center(), gpui::Modifiers::default());
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, cx| {
        let shell = shell.read(cx);
        assert!(shell.rail_focus.contains_focused(window, cx));
        assert!(!shell.rail_focus.is_focused(window));
        assert!(!shell.composer.focus_handle(cx).contains_focused(window, cx));
    });
    cx.simulate_keystrokes("h");
    cx.update(|window, cx| {
        let composer = shell.read(cx).composer.clone();
        assert!(composer.focus_handle(cx).contains_focused(window, cx));
        assert_eq!(composer.read(cx).input_text(cx), "h");
    });
}

#[gpui::test]
fn typing_in_the_rail_capture_popover_stays_in_it(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (shell, cx) = shell_window(dir.path(), cx);
    let plus = cx.debug_bounds("rail-quick-capture").expect("rail +");
    cx.simulate_click(plus.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    cx.simulate_keystrokes("h");
    cx.update(|_, cx| {
        let shell = shell.read(cx);
        let input = shell
            .quick_capture
            .rail_input
            .clone()
            .expect("popover input");
        assert_eq!(input.read(cx).text(), "h");
        assert_eq!(shell.composer.read(cx).input_text(cx), "");
    });
}

#[gpui::test]
fn typing_beside_an_expanded_side_chat_reaches_its_composer(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (shell, cx) = shell_window(dir.path(), cx);
    let side_id = shell.update(cx, |shell, cx| {
        let main: zeron_proto::Chat = serde_json::from_value(serde_json::json!({
            "id": "main", "deviceId": "local", "archived": false, "createdAt": Utc::now(),
        }))
        .unwrap();
        shell.state.update(cx, |state, _| {
            state.chats = vec![main];
            state.selected_chat = Some("main".into());
        });
        shell.active_chat = "main".into();
        let side: zeron_proto::Chat = serde_json::from_value(serde_json::json!({
            "id": "side", "createdByChatId": "main", "deviceId": "local",
            "archived": false, "createdAt": Utc::now(),
        }))
        .unwrap();
        shell.open_side_chat(side, shell.panel_key(cx), cx);
        shell.right_pane_expanded = true;
        cx.notify();
        shell.side_chat_seq
    });
    cx.update(|window, cx| window.draw(cx).clear());
    cx.run_until_parked();
    let text = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            let shell = shell.read(cx);
            (
                shell.side_chats[&side_id].composer.read(cx).input_text(cx),
                shell.composer.read(cx).input_text(cx),
            )
        })
    };
    for parked in [
        |shell: &Shell| shell.unfocused.clone(),
        |shell: &Shell| shell.navigation_focus.main.clone(),
    ] {
        cx.update(|window, cx| {
            let handle = parked(shell.read(cx));
            window.focus(&handle, cx);
            window.draw(cx).clear();
        });
        cx.simulate_keystrokes("h");
    }
    assert_eq!(text(cx), ("hh".to_string(), String::new()));
}

#[gpui::test]
fn typing_in_the_focused_composer_inserts_once(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    focus(cx, |host, cx| host.shell.read(cx).composer.focus_handle(cx));
    cx.simulate_keystrokes("a b");
    assert_eq!(composer_text(&host, cx), "ab");
}

#[gpui::test]
fn typing_with_another_element_focused_leaves_the_composer_alone(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    focus(cx, |host, _| host.other.clone());
    cx.simulate_keystrokes("h cmd-a");
    assert!(!composer_focused(&host, cx));
    assert_eq!(composer_text(&host, cx), "");
    focus(cx, |host, cx| host.shell.read(cx).unfocused.clone());
    cx.simulate_keystrokes("cmd-a ctrl-a escape");
    assert_eq!(composer_text(&host, cx), "");
}

#[gpui::test]
fn paste_from_a_parked_focus_lands_in_the_composer(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("pasted".into()));
    for keys in ["cmd-v", "ctrl-v"] {
        focus(cx, |host, cx| host.shell.read(cx).unfocused.clone());
        cx.simulate_keystrokes(keys);
        assert!(composer_focused(&host, cx));
    }
    assert_eq!(composer_text(&host, cx), "pastedpasted");
    focus(cx, |host, _| host.other.clone());
    cx.simulate_keystrokes("cmd-v cmd-shift-v");
    assert!(!composer_focused(&host, cx));
    assert_eq!(composer_text(&host, cx), "pastedpasted");
}
