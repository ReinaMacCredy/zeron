//! (Here, not in Luau: the Luau tests can't read an opacity or count script calls.)

use zgx_script::{ScriptHost, testing::TestApp};

fn mount() -> TestApp {
    let ui = std::env::var_os("ZERON_ZGX_TEST_UI")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui"));
    let app = TestApp::new(|runtime| ScriptHost::new(runtime, ui.join("chat_fixture.luau")));
    app.ui.scene.borrow_mut().resize(1100., 2000.);
    app.frames(30);
    assert_eq!(app.error(), None, "the fixture failed");
    assert!(app.host.run_command("go_s1"));
    app.frames(30);
    assert!(app.host.run_command("threads_seed"));
    app.frames(60);
    assert_eq!(app.error(), None, "the seed failed");
    app
}

#[test]
fn a_message_on_its_way_is_dimmed_and_not_once_it_lands() {
    let mut app = mount();
    let pending = app.opacity("sent-t7.m0");
    assert!((pending - 0.65).abs() < 0.01, "on its way: dimmed ({pending})");
    let sent = app.opacity("sent-t8.m0");
    assert!((sent - 1.).abs() < 0.01, "sent: full ink ({sent})");
    let failed = app.opacity("sent-t9.m0");
    assert!((failed - 1.).abs() < 0.01, "failed: full ink, its words say so ({failed})");
    assert!(app.host.run_command("thread_sent_lands"));
    app.frames(30);
    let landed = app.opacity("sent-t7.m0");
    assert!((landed - 1.).abs() < 0.01, "landed: full ink ({landed})");
}

#[test]
fn rows_at_rest_run_no_script_once_the_countdown_is_gone() {
    let app = mount();
    app.frames(10);
    let before = app.calls();
    app.frames(130);
    assert!(app.calls() > before, "the countdown ticks while it counts");
    // (Running agents tick the pill row's dots, U17's: settle them first.)
    assert!(app.host.run_command("threads_settle"));
    app.frames(60);
    let before = app.calls();
    app.frames(180);
    assert_eq!(app.calls() - before, 0, "rows at rest run no script");
}

