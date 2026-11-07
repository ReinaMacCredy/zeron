use zgx_script::{ScriptHost, Value};
use zgx_script::testing::TestApp;

fn ui_dir() -> std::path::PathBuf {
    std::env::var_os("ZERON_ZGX_TEST_UI").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui"))
}

fn mount(w: f32, h: f32) -> TestApp {
    let ui = ui_dir();
    let app = TestApp::new(|runtime| ScriptHost::new(runtime, ui.join("chat_fixture.luau")));
    app.ui.scene.borrow_mut().resize(w, h);
    app.frames(30);
    assert_eq!(app.error(), None, "the fixture failed");
    assert!(app.host.run_command("go_s2"));
    app.frames(30);
    app
}

fn messages(app: &TestApp) -> Vec<Value> {
    app.host.states().into_iter().find(|(n, _)| n == "chat.messages").map(|(_, v)| v.list().to_vec()).unwrap_or_default()
}

#[test]
fn a_settled_transcript_of_every_row_kind_runs_no_script_at_rest() {
    let app = mount(1100., 2400.);
    assert!(app.host.run_command("kinds_turn"), "the fixture has its every-row-kind seed");
    app.frames(30);
    let settled: Vec<Value> = messages(&app).into_iter().filter(|m| m.field("id").map(crate::auth::text).as_deref() != Some("ks2")).collect();
    app.host.set_state("chat.messages", Value::List(settled.into())).unwrap();
    app.frames(120);
    assert_eq!(app.error(), None);
    let before = app.calls();
    app.frames(180);
    assert_eq!(app.calls() - before, 0, "lines, questions, the secret card, Claude Code's cards and file links at rest run no script");
}

#[test]
fn a_running_claude_command_ticks_once_a_second() {
    let app = mount(1100., 2400.);
    assert!(app.host.run_command("kinds_turn"));
    app.frames(120);
    let before = app.calls();
    app.frames(180);
    let ran = app.calls() - before;
    assert!(ran > 0 && ran < 60, "three seconds of a pending card: its clock, not every frame ({ran})");
}

// Every third reply carries a question and a secret line, so the cards are in the measure.
fn long_chat(n: usize) -> Value {
    let rec = |pairs: Vec<(&str, Value)>| Value::record(pairs);
    let s = |t: &str| Value::from(t);
    let mut out = Vec::with_capacity(n * 2);
    for i in 0..n {
        out.push(rec(vec![("id", s(&format!("pu{i}"))), ("role", s("user")), ("text", s(&format!("Prompt {i}: look at `src/parse.rs` and tell me."))), ("at", s("Sep 8, 1:50 PM")), ("copy", s("x"))]));
        let tools = Value::List(
            vec![
                rec(vec![("id", s(&format!("r{i}"))), ("kind", s("read")), ("verb", s("Read")), ("target", s("parse.rs")), ("path", s("src/parse.rs"))]),
                rec(vec![("id", s(&format!("e{i}"))), ("kind", s("exec")), ("verb", s("Ran")), ("target", s("cargo test"))]),
            ]
            .into(),
        );
        let mut blocks = vec![
            rec(vec![("id", s("g")), ("kind", s("tools")), ("summary", s("Read 1 file · ran 1 command")), ("tools", tools)]),
            rec(vec![("id", s("t")), ("kind", s("text")), ("text", s(&format!("Reply {i}. The parser is [parse.rs](src/parse.rs:12); see **this** and `that`.\n\n- one\n- two\n\nMore words follow in a second paragraph so the row wraps.")))]),
        ];
        if i % 3 == 0 {
            blocks.push(rec(vec![("id", s("q")), ("kind", s("line")), ("line", s("input")), ("outcome", s("answered")), ("text", s("Scope → Branch")), ("answers", Value::List(vec![rec(vec![("label", s("Scope")), ("answer", s("Branch"))])].into()))]));
            blocks.push(rec(vec![("id", s("x")), ("kind", s("line")), ("line", s("secret")), ("title", s("Key")), ("status", s("saved")), ("text", s("Secret saved"))]));
        }
        out.push(rec(vec![("id", s(&format!("pa{i}"))), ("role", s("assistant")), ("text", s("x")), ("model", s("Opus")), ("at", s("Sep 8, 1:51 PM")), ("copy", s("x")), ("blocks", Value::List(blocks.into()))]));
    }
    Value::List(out.into())
}

fn p(sorted: &[f64], q: f64) -> f64 {
    sorted[((sorted.len() as f64 - 1.) * q).round() as usize]
}

// Run with --ignored --nocapture; ZERON_ZGX_TEST_UI at another tree's ui/ measures that one alike.
#[test]
#[ignore]
fn a_2000_row_chat_opens_and_scrolls() {
    let app = mount(960., 800.);
    let chat = long_chat(1000);
    let calls = app.calls();
    let started = std::time::Instant::now();
    app.host.set_state("chat.messages", chat).unwrap();
    app.frame();
    let first = started.elapsed();
    app.frames(29);
    let open = started.elapsed();
    let open_calls = app.calls() - calls;
    assert_eq!(app.error(), None);
    let mut app = app;
    let mut times = Vec::new();
    let mut scripts = Vec::new();
    for step in 0..240 {
        let before = app.calls();
        let t = std::time::Instant::now();
        app.scroll("#transcript", if step < 160 { -120. } else { 120. });
        app.frame();
        times.push(t.elapsed().as_secs_f64() * 1000.);
        scripts.push((app.calls() - before) as f64);
    }
    times.sort_by(f64::total_cmp);
    scripts.sort_by(f64::total_cmp);
    eprintln!(
        "u24-perf ui={} open_first_ms={:.1} open_30_frames_ms={:.1} open_calls={} scroll_p50_ms={:.2} scroll_p95_ms={:.2} scroll_max_ms={:.2} script_calls_p50={} script_calls_max={}",
        ui_dir().display(),
        first.as_secs_f64() * 1000.,
        open.as_secs_f64() * 1000.,
        open_calls,
        p(&times, 0.5),
        p(&times, 0.95),
        times.last().copied().unwrap_or_default(),
        p(&scripts, 0.5),
        scripts.last().copied().unwrap_or_default(),
    );
}

fn state(app: &TestApp, name: &str) -> String {
    app.host.states().into_iter().find(|(n, _)| n == name).map(|(_, v)| crate::auth::text(&v)).unwrap_or_default()
}

#[test]
fn a_selection_across_a_file_link_reads_and_copies_without_its_glyph_s_joiner() {
    let mut app = mount(1100., 2400.);
    assert!(app.host.run_command("kinds_turn"));
    app.frames(30);
    let paragraph = app.target("role:text ~\"The parser is\"").expect("the reply's paragraph");
    assert!(app.ui.select_all_text(paragraph), "its text selects");
    let read = crate::selected(&app.ui);
    let text = read.field("text").map(crate::auth::text).unwrap_or_default();
    assert!(text.contains("The parser is parse.rs and the host main.rs"), "the names read as written: {text:?}");
    assert!(!text.contains('\u{2060}'), "annotations and side chats read no joiner: {text:?}");
    assert!(!app.ui.selected_text().contains('\u{2060}') && !app.ui.selected_html().contains('\u{2060}'), "copied, neither has one");
}

#[test]
fn a_cmd_click_on_a_file_link_opens_nothing_and_on_a_web_link_goes_to_the_browser() {
    use zgx::input::{InputEvent, Modifiers, PointerButton};
    let mut app = mount(1100., 2400.);
    assert!(app.host.run_command("kinds_turn"));
    app.frames(30);
    let cmd = Modifiers { meta: cfg!(target_os = "macos"), control: !cfg!(target_os = "macos"), ..Default::default() };
    let click = |app: &mut TestApp, target: &str, held: Modifiers| {
        let (_, bounds) = zgx_script::driver::reach(&app.ui, &app.handle, target).unwrap();
        let (x, y) = zgx_script::driver::middle(bounds);
        let button = PointerButton::Primary;
        app.ui.dispatch_with_modifiers(InputEvent::PointerDown { x, y, button }, held);
        app.ui.dispatch_with_modifiers(InputEvent::PointerUp { x, y, button }, held);
        app.ui.input.forget_clicks();
        app.frames(5);
    };
    click(&mut app, "role:link \"parse.rs\"", cmd);
    click(&mut app, "role:link \"the docs\"", cmd);
    assert!(!state(&app, "calls").contains("open_file:s2:src/parse.rs"), "{}", state(&app, "calls"));
    assert_eq!(state(&app, "ds/links.luau::links_last"), "system:https://example.com/a.rs");
    click(&mut app, "role:link \"parse.rs\"", Modifiers::default());
    assert!(state(&app, "calls").contains("open_file:s2:src/parse.rs@12"), "a plain click opens it: {}", state(&app, "calls"));
}
