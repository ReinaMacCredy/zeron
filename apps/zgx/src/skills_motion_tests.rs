//! The Skills page (ui/skills_fixture.luau): an agent's filter fades its grid in over 150 ms, as gpui's
//! `fade_quick` on `skills-grid-{filter}`. (The Lua tests can't read an opacity.)

use std::time::Duration;

use zgx_script::{ScriptHost, testing::TestApp};

/// The grid's opacity as drawn: its own and every node's over it (the presence plays its enter on the grid's holder).
fn drawn(app: &mut TestApp) -> f32 {
    let mut node = app.find("skills-grid");
    let scene = app.ui.scene.borrow();
    let mut out = 1.0;
    while let Some(id) = node {
        out *= scene.effects(id).opacity;
        node = scene.parent(id);
    }
    out
}

#[test]
fn an_agent_switch_fades_the_skills_grid_in() {
    let ui = std::env::var_os("ZERON_ZGX_TEST_UI")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui"));
    let mut app = TestApp::new(|runtime| ScriptHost::new(runtime, ui.join("skills_fixture.luau")));
    app.ui.scene.borrow_mut().resize(1200., 900.);
    app.frames(20);
    app.click("#rail-more");
    app.frames(12);
    app.click("#rail-skills");
    app.frames(20);
    std::thread::sleep(Duration::from_millis(300));
    app.frames(5);
    assert!((drawn(&mut app) - 1.0).abs() < 0.01, "the grid shows at rest");
    app.click("#skills-tab-codex");
    let mut seen = Vec::new();
    for _ in 0..30 {
        app.frames(1);
        seen.push(drawn(&mut app));
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(seen.iter().any(|o| *o < 0.9), "the grid fades in after the switch: {seen:?}");
    std::thread::sleep(Duration::from_millis(300));
    app.frames(5);
    assert!((drawn(&mut app) - 1.0).abs() < 0.01, "and ends shown");
}
