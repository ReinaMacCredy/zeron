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
