//! (Here, not in Luau: the Luau tests can't count script calls or reach zeron_proto.)

use zgx_script::{ScriptHost, Value, testing::TestApp};

fn mount() -> TestApp {
    let ui = std::env::var_os("ZERON_ZGX_TEST_UI")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui"));
    let mut app = TestApp::new(|runtime| ScriptHost::new(runtime, ui.join("chat_fixture.luau")));
    app.ui.scene.borrow_mut().resize(960., 700.);
    app.frames(30);
    assert_eq!(app.error(), None, "the fixture failed");
    assert!(app.host.run_command("go_s2"));
    app.frames(30);
    assert!(app.host.run_command("annotations_seed"));
    app.frames(60);
    assert_eq!(app.error(), None, "the seed failed");
    // (The second was selected in a file: its badge is the preview's, not the transcript's.)
    assert!(app.find("#annotation-badge-1").is_some() && app.find("#annotation-badge-2").is_none(), "the conversation's badge drawn");
    app
}

#[test]
fn annotations_at_rest_run_no_script() {
    let app = mount();
    app.frames(30);
    let before = app.calls();
    app.frames(180);
    assert_eq!(app.calls() - before, 0, "badges and the chip at rest run no script");
}

#[test]
fn a_prompt_carries_its_annotations_as_zeron_writes_them() {
    use zeron_proto::quotes::{Quote, with_quotes};
    let mut app = mount();
    assert!(app.host.run_command("stage_selection"));
    assert!(app.host.run_command("stage_terminal"));
    app.frames(30);
    app.type_text("role:textbox \"Message\"", "look");
    app.keys("Enter");
    app.frames(60);
    let calls = app
        .host
        .states()
        .into_iter()
        .find(|(name, _)| name == "calls" || name.ends_with("::calls"))
        .map(|(_, v)| match v {
            Value::Str(s) => s.to_string(),
            _ => String::new(),
        })
        .unwrap_or_default();
    let note = |text: &str, comment: &str, source: Option<&str>| Quote { comment: Some(comment.into()), ..Quote::annotation(text, source.map(str::to_owned)) };
    let zeron = with_quotes(
        "look",
        &[note("What shall we work on?", "why this?\nand how", None), note("Hello there", "", Some("docs/plan.md")), Quote::new("Layout dominates on both machines."), Quote::terminal("cargo build\nerror[E0425]: cannot find value")],
    );
    assert!(calls.contains(&format!("send:s2:{zeron}")), "sent as Zeron writes it:\n{zeron}\n---\n{calls}");
}

#[test]
fn a_selection_pill_put_away_runs_no_script() {
    let mut app = mount();
    let reply = "#transcript >> ~\"Hi! What shall we work on?\"";
    let b = app.bounds(reply);
    let line = || Value::record([("x", Value::Num(f64::from(b.x) + 10.)), ("y", Value::Num(f64::from(b.y))), ("w", Value::Num(160.)), ("h", Value::Num(20.))]);
    app.host.set_state("fake_selection", Value::record([("text", Value::from("What shall we work on?")), ("top", line()), ("bottom", line())])).unwrap();
    app.click(reply);
    app.frames(30);
    assert!(app.find("#annotation-pill").is_some(), "the selection's pill shows");
    app.keys("Escape");
    app.frames(30);
    assert!(app.find("#annotation-pill").is_none(), "Escape put the pill away");
    let before = app.calls();
    app.frames(180);
    assert_eq!(app.calls() - before, 0, "the selection kept, its pill gone, runs no script");
}
