//! (Here, not in Luau: the Luau tests can't count script calls.)

use zgx_script::{ScriptHost, testing::TestApp};

/// The fixtures' stand-in host settles its answers on a 16 ms clock of its own (ui/files_stub.luau): what the fixture runs at rest with nothing open is that clock alone.
fn stub_clock(fixture: &str) -> u64 {
    let app = mount(fixture);
    app.frames(120);
    let before = app.calls();
    app.frames(180);
    app.calls() - before
}

fn mount(fixture: &str) -> TestApp {
    let ui = std::env::var_os("ZERON_ZGX_TEST_UI")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui"));
    let app = TestApp::new(|runtime| ScriptHost::new(runtime, ui.join(fixture)));
    app.ui.scene.borrow_mut().resize(1200., 900.);
    app.frames(30);
    assert_eq!(app.error(), None, "the fixture failed");
    app
}

#[test]
fn the_panel_at_rest_with_a_terminal_and_a_file_open_runs_no_script() {
    let mut app = mount("panel_fixture.luau");
    assert!(app.wait_until(|app| app.find("#panel-open-terminal").is_some()), "the panel's home");
    app.click("#panel-open-terminal");
    assert!(app.wait_until(|app| app.find("#stub-terminal").is_some()), "the terminal tab");
    app.click("role:button \"New Tab\"");
    app.frames(10);
    app.click("#panel-open-files");
    assert!(app.wait_until(|app| app.find("role:treeitem \"long.txt\"").is_some()), "the files tab");
    app.click("role:treeitem \"long.txt\"");
    assert!(app.wait_until(|app| app.find("role:textbox \"long.txt\"").is_some()), "the file open");
    app.frames(120);
    let before = app.calls();
    app.frames(180);
    assert_eq!(app.calls() - before, stub_clock("panel_fixture.luau"), "the panel at rest runs no script past the stub's clock");
}

#[test]
fn a_lanes_tab_at_rest_with_its_working_dot_runs_no_script() {
    let mut app = mount("lanes_fixture.luau");
    app.click("#session-lanes >> role:button \"Open subagent scan the repo\"");
    assert!(app.wait_until(|app| app.find(".panel-tab-running").is_some()), "the dot on the running subagent's tab");
    app.frames(120);
    let before = app.calls();
    app.frames(180);
    assert_eq!(app.calls() - before, stub_clock("lanes_fixture.luau"), "a running lane's tab at rest runs no script past the stub's clock");
}

