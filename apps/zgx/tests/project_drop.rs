//! A folder from Finder dropped on New project adds it, as Zeron's picker
//! does (crates/ui shell/spaces.rs): on this device's Folders step the card
//! says "Drop to add project" while files are over it; anywhere else it says
//! nothing. Files from outside arrive as the platform delivers them
//! (`FileHover`, `FilesDrop`, `FileHoverCancelled`), which the Lua test kit
//! can't send. On the shell in `ui/shell_fixture.luau`.

use std::path::PathBuf;

use zgx::input::InputEvent;
use zgx_script::{ScriptHost, testing::TestApp};

fn ui_dir() -> PathBuf {
    std::env::var_os("ZERON_ZGX_TEST_UI")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui"))
}

fn picker() -> TestApp {
    let app = TestApp::new(|runtime| ScriptHost::new(runtime, ui_dir().join("shell_fixture.luau")));
    app.ui.scene.borrow_mut().resize(1200., 800.);
    app.frames(30);
    assert!(app.host.run_command("shell.new_project"));
    app.frames(20);
    app
}

fn choose(app: &mut TestApp, row: &str) {
    let target = format!(r##"#palette-list >> role:option "{row}""##);
    assert!(app.wait_until(|app| app.find(&target).is_some()), "{row} lists");
    app.click(&target);
    app.frames(20);
}

fn over_list(app: &mut TestApp) -> (f32, f32) {
    let r = app.bounds("#palette-list");
    (r.x + r.width / 2., r.y + r.height / 2.)
}

fn shown(app: &mut TestApp) -> bool {
    app.find("#project-drop").is_some() && app.shown("#project-drop")
}

fn added(app: &TestApp) -> String {
    app.host.states().into_iter().find(|(name, _)| name.ends_with("calls")).map(|(_, calls)| format!("{calls:?}")).unwrap_or_default()
}

#[test]
fn a_folder_over_this_devices_folders_says_so_and_a_drop_adds_it() {
    let mut app = picker();
    choose(&mut app, "MacBook Pro");
    choose(&mut app, "Home");
    assert!(app.wait_until(|app| app.find(r#"#palette-list >> role:option "code""#).is_some()));
    assert!(!shown(&mut app), "nothing carried: no overlay");
    let (x, y) = over_list(&mut app);
    let path = PathBuf::from("/Users/me/notes");
    app.ui.dispatch(InputEvent::FileHover { x, y, path: path.clone() });
    app.frames(20);
    assert!(shown(&mut app), "a folder over the card shows the overlay");
    assert!(app.find(r#"#project-drop >> ~"Drop to add project""#).is_some());
    app.ui.dispatch(InputEvent::FilesDrop { x, y, paths: vec![path].into() });
    app.frames(30);
    assert!(!shown(&mut app), "a drop takes the overlay away");
    let calls = added(&app);
    assert!(calls.contains("folders:mac:/Users/me/notes"), "the drop reads the folder: {calls}");
    assert!(calls.contains("create:mac:/Users/me/notes"), "and adds it: {calls}");
}

#[test]
fn leaving_the_window_takes_the_overlay_with_it() {
    let mut app = picker();
    choose(&mut app, "MacBook Pro");
    choose(&mut app, "Home");
    let (x, y) = over_list(&mut app);
    app.ui.dispatch(InputEvent::FileHover { x, y, path: PathBuf::from("/Users/me/notes") });
    app.frames(20);
    assert!(shown(&mut app));
    app.ui.dispatch(InputEvent::FileHoverCancelled);
    app.frames(20);
    assert!(!shown(&mut app), "the drag gone, the overlay goes");
}

#[test]
fn another_devices_folders_and_the_devices_step_take_no_drop() {
    let mut app = picker();
    let (x, y) = over_list(&mut app);
    let path = PathBuf::from("/Users/me/notes");
    app.ui.dispatch(InputEvent::FileHover { x, y, path: path.clone() });
    app.frames(20);
    assert!(!shown(&mut app), "no device chosen: Finder's folder isn't anywhere yet");
    app.ui.dispatch(InputEvent::FilesDrop { x, y, paths: vec![path.clone()].into() });
    app.frames(20);
    assert!(!added(&app).contains("create:"), "and a drop adds nothing");
    choose(&mut app, "devbox");
    let (x, y) = over_list(&mut app);
    app.ui.dispatch(InputEvent::FileHover { x, y, path });
    app.frames(20);
    assert!(!shown(&mut app), "a folder on this Mac isn't on devbox");
}
