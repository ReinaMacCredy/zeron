//! The composer between a new chat and a session (ui/screens/chat/composer,
//! mounted headless on ui/shell_fixture.luau): it travels from one place to
//! the other, never faded, as gpui's dock does; another session puts it
//! where it goes at once. (The Lua tests can't read an opacity:
//! ui/tests/shell_route_motion.test.luau has the panel's side of it.)

use std::time::Duration;

use zgx_script::{ScriptHost, testing::TestApp};

fn mount() -> TestApp {
    // (This checkout's `ui/`, or `ZERON_ZGX_TEST_UI`'s: another copy, to see
    // the test fail on code from before a fix.)
    let ui = std::env::var_os("ZERON_ZGX_TEST_UI")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui"));
    let path = ui.join("shell_fixture.luau");
    let app = TestApp::new(|runtime| ScriptHost::new(runtime, path));
    app.ui.scene.borrow_mut().resize(1200., 800.);
    app.frames(30);
    app
}

/// The composer where it's painted: its box through what moves it on its way.
fn painted(app: &mut TestApp) -> zgx::scene::Rect {
    let node = app.find("#chat-composer").expect("the composer");
    app.ui.prepare_frame();
    app.ui.scene.borrow().transformed_bounds(node)
}

/// A frame and a little of the clock, `n` times: each look at the dock's
/// opacity, the composer's top and its height (the pill's, with what hangs
/// off it).
fn watch(app: &mut TestApp, n: usize) -> Vec<(f32, f32, f32)> {
    (0..n)
        .map(|_| {
            app.frames(1);
            std::thread::sleep(Duration::from_millis(12));
            let b = painted(app);
            (app.opacity("chat-composer-dock"), b.y, b.height)
        })
        .collect()
}

/// Every look as it travels: shown in full (no fade), setting out from
/// where it stood (`from`), seen on its way, and ending where it goes
/// (`to`).
fn travels(seen: &[(f32, f32, f32)], from: f32, to: f32, what: &str) {
    let way = (from - to).abs();
    let ys = seen.iter().map(|s| s.1).collect::<Vec<_>>();
    for (i, (opacity, _, _)) in seen.iter().enumerate() {
        assert!((opacity - 1.0).abs() < 0.01, "{what}: faded, frame {}: opacity {opacity}", i + 1);
    }
    assert!((ys[0] - from).abs() < (ys[0] - to).abs(), "{what}: it sets out from where it stood ({from} to {to}): {ys:?}");
    let between = ys.iter().filter(|y| (**y - from).abs() > way / 10. && (**y - to).abs() > way / 10.).count();
    assert!(between >= 2, "{what}: it's seen on its way ({from} to {to}): {ys:?}");
    assert!((ys.last().unwrap() - to).abs() <= 1., "{what}: it ends where it goes ({to}): {ys:?}");
}

#[test]
fn the_composer_travels_between_a_new_chat_and_a_session_and_is_at_once_between_sessions() {
    let mut app = mount();
    let hero = painted(&mut app).y;
    app.click("#shell-sidebar >> role:button \"Fix the build\"");
    let seen = watch(&mut app, 80);
    let docked = painted(&mut app).y;
    assert!(hero < docked - 50., "it stands mid-page on a new chat ({hero} vs {docked})");
    travels(&seen, hero, docked, "a chat opened from a new chat");

    // Another session: where it is from the first frame, never faded.
    app.click("#shell-sidebar >> role:button \"Tidy the tests\"");
    for (i, (opacity, y, _)) in watch(&mut app, 40).iter().enumerate() {
        assert!((opacity - 1.0).abs() < 0.01, "another session: faded, frame {}: opacity {opacity}", i + 1);
        assert!((y - docked).abs() <= 1., "another session: frame {} at {y}, not docked ({docked})", i + 1);
    }

    // To a new chat: up to mid-page.
    app.click("#app-rail >> role:button \"Home\"");
    let seen = watch(&mut app, 80);
    assert!((painted(&mut app).y - hero).abs() <= 1., "mid-page again");
    travels(&seen, docked, hero, "to a new chat");

    // And back to the session: down to the dock.
    app.click("#shell-sidebar >> role:button \"Fix the build\"");
    let seen = watch(&mut app, 80);
    travels(&seen, hero, docked, "back to the session");

    // Growing in the same chat (lines typed) still eases: seen on its way.
    std::thread::sleep(Duration::from_millis(500));
    let short = painted(&mut app).height;
    app.fill("#chat-composer >> role:textbox", "one\ntwo\nthree\nfour");
    let seen = watch(&mut app, 40);
    let tall = seen.last().unwrap().2;
    assert!(tall > short + 20., "the lines opened it out ({short} → {tall})");
    assert!(
        seen.iter().any(|(_, _, h)| *h > short + 1. && *h < tall - 1.),
        "it grew at once, not eased ({short} → {tall}): {:?}",
        seen.iter().map(|s| s.2).collect::<Vec<_>>()
    );
}

/// What comes and goes with the panel stands still and only fades a little
/// (ds/motion `content`), never popping: the panel's tabs as it opens and
/// closes, the page as the panel expands over it and gives it back. Over
/// each, it stays where it's laid out (no drift: a hundredth of a point at
/// most), its opacity passes through between, a step at a time, never
/// under the fade's floor (0.3) as it shows, and ends where it goes (the
/// page under the panel expanded, hidden; the panel's content, closed, at
/// the floor in a panel that shows none of it); settled, the layout warns
/// of nothing new. (ui/tests/shell_route_motion.test.luau has their widths.)
#[test]
fn the_panel_s_content_and_the_page_only_fade_as_they_come_and_go() {
    const FLOOR: f32 = 0.3;
    let mut app = mount();
    app.click("#shell-sidebar >> role:button \"Fix the build\"");
    watch(&mut app, 20);
    let warnings = |app: &mut TestApp| -> Vec<String> {
        app.ui.prepare_frame();
        zgx::inspect::layout_warnings(&app.ui).iter().map(|w| w.to_string()).collect()
    };
    // (Where its right edge is painted: it stands at its column's side by
    // the window's edge, the panel's content at the window's right, the
    // page at its left.)
    let edge = |app: &mut TestApp, id: &str| -> f32 {
        let node = app.find(&format!("#{id}")).expect("it");
        app.ui.prepare_frame();
        let b = app.ui.scene.borrow().transformed_bounds(node);
        if id == "shell-page-room" { b.x } else { b.x + b.width }
    };
    // (What the fixture has before the panel ever opens.)
    let before = warnings(&mut app);
    let mut fade = |click: &str, id: &str, to: f32, what: &str| {
        app.click(click);
        let seen: Vec<(f32, f32, bool)> = (0..30)
            .map(|_| {
                app.frames(1);
                std::thread::sleep(Duration::from_millis(12));
                // (Whether its column shows: the panel under 2 points shows none.)
                let shows = id == "shell-page-room" || app.opacity("shell-files") > 0.;
                (app.opacity(id), edge(&mut app, id), shows)
            })
            .collect();
        let (lo, hi) = seen.iter().filter(|s| s.2).fold((f32::MAX, f32::MIN), |(lo, hi), s| (lo.min(s.1), hi.max(s.1)));
        let drift = hi - lo;
        assert!(drift <= 0.011, "{what}: {id} moved {drift} points: {seen:?}");
        let between = seen.iter().filter(|s| s.0 > 0.02 && s.0 < 0.98).count();
        let jump = seen.windows(2).map(|w| (w[1].0 - w[0].0).abs()).fold(0., f32::max);
        assert!(between >= 2 && jump <= 0.7, "{what}: {id} at {seen:?}");
        let showing = seen.iter().filter(|s| (s.0 - to).abs() > 0.01).map(|s| s.0).fold(1., f32::min);
        assert!(showing >= FLOOR - 0.01, "{what}: {id} fades only to the floor as it shows: {seen:?}");
        assert!((seen.last().unwrap().0 - to).abs() < 0.01, "{what}: {id} ends at {seen:?}");
        // At rest, nothing it holds is laid out in a box that shows none of
        // it (`zgx inspect errors` names nothing new).
        app.frames(30);
        assert_eq!(warnings(&mut app), before, "{what}: settled, the layout's warnings");
    };
    fade("#titlebar >> role:button \"Panel\"", "shell-files-content", 1., "opening");
    fade("#panel-expand", "shell-page-room", 0., "expanding");
    fade("#panel-expand", "shell-page-room", 1., "collapsing");
    fade("#titlebar >> role:button \"Panel\"", "shell-files-content", FLOOR, "closing");
    assert!(app.opacity("shell-files") < 0.01, "closed, the panel shows none of it");
}

/// The sidebar folds and comes back as the panel opens and closes: its
/// card's left edge and what's in it stand still as its right edge sweeps
/// over them (no slide: the window's gap before it goes only with its last
/// 5 points), what's in it only fades a little, from 0.3 (ds/motion
/// `content`), and the page beside it keeps the gap between cards (4) as
/// it follows the edge. (It used to slide 5 points with the gap closing on
/// its own clock, the page's gap easing from 4 to 5 beside it.)
#[test]
fn the_sidebar_s_content_stands_still_and_only_fades_as_it_folds() {
    const FLOOR: f32 = 0.3;
    let mut app = mount();
    app.click("#shell-sidebar >> role:button \"Fix the build\"");
    watch(&mut app, 20);
    let at = |app: &mut TestApp, sel: &str| -> Option<(f32, f32)> {
        let node = app.find(sel)?;
        app.ui.prepare_frame();
        let b = app.ui.scene.borrow().transformed_bounds(node);
        Some((b.x, b.width))
    };
    for what in ["folding", "coming back"] {
        assert!(app.host.run_command("shell.toggle_sidebar"));
        let mut rows = Vec::new();
        for _ in 0..24 {
            app.frames(1);
            std::thread::sleep(Duration::from_millis(12));
            let side = at(&mut app, "#shell-sidebar").expect("its column");
            let page = at(&mut app, "#shell-page-room").expect("the page");
            let row = at(&mut app, "#shell-sidebar >> role:button \"Fix the build\"");
            let fade = if app.find("#sidebar-content").is_some() { app.opacity("sidebar-content") } else { 1. };
            // (Its card more than the window's gap wide: the gap is whole.)
            if side.1 > 10. {
                rows.push(row.expect("its row").0);
                assert!((page.0 - (side.0 + side.1) - 4.).abs() < 0.05, "{what}: the gap beside it, {}", page.0 - side.0 - side.1);
                assert!(fade >= FLOOR - 0.01, "{what}: it fades only to the floor as it shows: {fade}");
            }
        }
        let (lo, hi) = rows.iter().fold((f32::MAX, f32::MIN), |(lo, hi), x| (lo.min(*x), hi.max(*x)));
        assert!(rows.len() >= 4 && hi - lo <= 0.011, "{what}: its row stands at {lo}..{hi} over {} frames", rows.len());
        let end = if what == "folding" { FLOOR } else { 1. };
        assert!((app.opacity("sidebar-content") - end).abs() < 0.01, "{what}: it ends faded to {end}");
        app.frames(30);
    }
}

/// Inspect the composer pill, text viewport clip, scroll offset, topmost
/// rendered text line, and caret bounds/visibility on the current frame.
#[derive(Clone, Copy, Debug)]
struct TextFrame {
    composer: Rect,
    field: Rect,
    scroll_y: f32,
    line_left_full: f32,
    line_top_full: f32,
    line_top_visible: Option<f32>,
    line_height_full: f32,
    line_height_visible: f32,
    caret_full: Rect,
    caret_visible: Option<Rect>,
    caret_painted_over: bool,
}

fn inspect_composer_text(app: &mut TestApp) -> TextFrame {
    let field = app
        .find(FIELD)
        .or_else(|| app.find("role:textbox"))
        .expect("draft field");
    let composer = app.find("#chat-composer").unwrap_or(field);
    let editor = app.ui.editor_within(field).expect("editor handle");
    let scroll_y = editor.scroll_offset().1;
    let scene = app.ui.scene.borrow();
    let composer_rect = scene.transformed_bounds(composer);
    let field_rect = scene.transformed_bounds(field);
    let caret_full = scene.transformed_bounds(editor.caret);
    let caret_visible = scene.visible_bounds(editor.caret);
    let caret_painted_over = scene
        .plain_box(editor.caret)
        .is_some_and(|r| scene.painted_over(editor.caret, r));
    // Find the topmost non-empty painted text node inside the editor.
    let mut stack = vec![editor.node];
    let mut top_line: Option<(Rect, Option<Rect>)> = None;
    while let Some(id) = stack.pop() {
        let has_text = match scene.kind(id) {
            zgx::scene::NodeKind::Text { text, .. } => !text.is_empty(),
            zgx::scene::NodeKind::RichText { text } => !text.text().is_empty(),
            _ => false,
        };
        if has_text {
            let full = scene.transformed_bounds(id);
            let vis = scene.visible_bounds(id);
            if top_line.is_none_or(|(best, _)| full.y < best.y) {
                top_line = Some((full, vis));
            }
        }
        stack.extend_from_slice(scene.children(id));
    }
    let (full, vis) = top_line.expect("non-empty rendered text line inside composer");
    TextFrame {
        composer: composer_rect,
        field: field_rect,
        scroll_y,
        line_left_full: full.x,
        line_top_full: full.y,
        line_top_visible: vis.map(|r| r.y),
        line_height_full: full.height,
        line_height_visible: vis.map_or(0., |r| r.height),
        caret_full,
        caret_visible,
        caret_painted_over,
    }
}

fn assert_caret_unclipped(frame: TextFrame, context: &str) {
    let vis = frame
        .caret_visible
        .unwrap_or_else(|| panic!("{context}: caret completely clipped out: {frame:?}"));
    assert!(
        (vis.y - frame.caret_full.y).abs() <= 0.5,
        "{context}: caret top clipped by {}px: {frame:?}",
        vis.y - frame.caret_full.y
    );
    assert!(
        vis.height >= frame.caret_full.height - 0.5,
        "{context}: caret bottom clipped (visible {} of {}): {frame:?}",
        vis.height,
        frame.caret_full.height
    );
    assert!(
        !frame.caret_painted_over,
        "{context}: caret painted over by another control: {frame:?}"
    );
}

fn assert_no_top_clip(frame: TextFrame, context: &str) {
    assert!(
        frame.scroll_y <= 0.05,
        "{context}: top scrolled off by {}px during resize: {frame:?}",
        frame.scroll_y
    );
    let vis_y = frame
        .line_top_visible
        .unwrap_or_else(|| panic!("{context}: top line completely clipped out: {frame:?}"));
    assert!(
        (vis_y - frame.line_top_full).abs() <= 0.5,
        "{context}: top of first line clipped by {}px (full.y={}, visible.y={}): {frame:?}",
        vis_y - frame.line_top_full,
        frame.line_top_full,
        vis_y
    );
    // A wrapped paragraph is a single logical text node whose height is N * 22px;
    // the full first visual line (at least min(line_height_full, 22px)) must stay
    // completely unclipped at the top of the viewport on every frame.
    let first_visual_line = frame.line_height_full.min(22.0);
    assert!(
        frame.line_height_visible >= first_visual_line - 0.5,
        "{context}: first visual line clipped vertically (visible {} of {}): {frame:?}",
        frame.line_height_visible,
        first_visual_line
    );
    assert!(
        frame.field.height >= first_visual_line - 0.5,
        "{context}: editor height collapsed below a line: {frame:?}"
    );
}

#[test]
fn composer_resize_never_clips_top_of_text_and_keeps_text_position_stable() {
    // 1. Expanded growth & shrink in both New-Chat (top-anchored) and Docked (bottom-anchored).
    for session in [false, true] {
        let mode = if session { "docked" } else { "new-chat" };
        let mut shell = Shell::new(false);
        if session {
            shell.app.click(BUILD);
            shell.frames(60);
        }
        shell.app.click(FIELD);
        shell.frames(2);
        // Start with 3 lines (settled expanded).
        type_at_caret(&mut shell.app, "Line 1\nLine 2\nLine 3");
        shell.frames(40);
        let base = inspect_composer_text(&mut shell.app);
        let base_rel = base.line_top_full - base.composer.y;
        assert_no_top_clip(base, &format!("{mode} base 3 lines"));
        let base_warnings = zgx::inspect::layout_warnings(&shell.app.ui).len();

        // Grow line by line (3 -> 4 -> 5 -> 6) via Shift+Enter / newline typing.
        for n in 4..=6 {
            type_at_caret(&mut shell.app, &format!("\nLine {n}"));
            // Check IMMEDIATELY on frame 0 (before animation advances) and every frame through settle.
            let f0 = inspect_composer_text(&mut shell.app);
            assert_no_top_clip(f0, &format!("{mode} grow to {n} lines frame 0"));
            assert!(
                ((f0.line_top_full - f0.composer.y) - base_rel).abs() <= 0.5,
                "{mode} grow to {n} frame 0: text shifted relative to composer top ({} vs {}): {f0:?}",
                f0.line_top_full - f0.composer.y,
                base_rel
            );
            for step in 1..=35 {
                shell.frames(1);
                let f = inspect_composer_text(&mut shell.app);
                assert_no_top_clip(f, &format!("{mode} grow to {n} lines frame {step}"));
                let rel = f.line_top_full - f.composer.y;
                assert!(
                    (rel - base_rel).abs() <= 0.5,
                    "{mode} grow to {n} frame {step}: text shifted relative to composer top ({rel} vs {base_rel}): {f:?}"
                );
                if !session {
                    assert!(
                        (f.line_top_full - base.line_top_full).abs() <= 0.5,
                        "new-chat grow to {n} frame {step}: text jumped vertically on screen ({} vs {}): {f:?}",
                        f.line_top_full,
                        base.line_top_full
                    );
                }
                if step == 3 || step == 35 {
                    assert!(
                        zgx::inspect::layout_warnings(&shell.app.ui).len() <= base_warnings,
                        "{mode} grow to {n} frame {step}: decoupled textarea introduced layout warnings"
                    );
                }
            }
            // Settled, the textarea's own height matches its settled text height (n * 22).
            let settled = inspect_composer_text(&mut shell.app);
            assert!(
                (settled.field.height - (n as f32) * 22.0).abs() <= 0.5,
                "{mode} grow to {n} settled: expected field.height={}, got {:?}",
                (n as f32) * 22.0,
                settled.field
            );
        }

        // Shrink from 6 lines back to 2 lines: text stays stable relative to composer top.
        let field = shell.app.find(FIELD).unwrap();
        let editor = shell.app.ui.editor_within(field).unwrap();
        let len = editor.editor.borrow().text().len();
        editor.set_selection(7, len); // keep "Line 1\n" -> replace tail with "Line 2"
        type_at_caret(&mut shell.app, "Line 2");
        for step in 0..=35 {
            if step > 0 {
                shell.frames(1);
            }
            let f = inspect_composer_text(&mut shell.app);
            assert_no_top_clip(f, &format!("{mode} shrink frame {step}"));
            let rel = f.line_top_full - f.composer.y;
            assert!(
                (rel - base_rel).abs() <= 0.5,
                "{mode} shrink frame {step}: text shifted relative to composer top ({rel} vs {base_rel}): {f:?}"
            );
        }
        // Settled back at 2 lines (below min 56px): editor height settles to 56px.
        let settled_shrink = inspect_composer_text(&mut shell.app);
        assert!(
            (settled_shrink.field.height - 56.0).abs() <= 0.5,
            "{mode} shrink settled: expected min viewport 56, got {:?}",
            settled_shrink.field
        );
    }
}

#[test]
fn composer_compact_to_expanded_transitions_never_clip_top_of_text() {
    // Test: (a) Shift+Enter from compact, (b) typing across compact wrap boundary,
    // (c) pasting 6 lines into compact, (d) pasting a long unwrapped paragraph that wraps
    // to MORE lines at compact width than at expanded width.
    let paragraph = "word ".repeat(55); // ~4 lines expanded, ~5 lines compact
    let cases: [(&str, &str); 4] = [
        ("newline", "First line\nSecond line"),
        ("wrap-boundary", &"W".repeat(65)),
        ("paste-6-lines", "L1\nL2\nL3\nL4\nL5\nL6"),
        ("paste-long-paragraph", paragraph.trim_end()),
    ];
    for (name, payload) in cases {
        let mut shell = Shell::new(false);
        shell.app.click(BUILD);
        shell.frames(60);
        shell.app.click(FIELD);
        shell.frames(2);
        type_at_caret(&mut shell.app, "a");
        shell.frames(30);
        let compact = inspect_composer_text(&mut shell.app);
        let compact_rel = compact.line_top_full - compact.composer.y;
        assert_no_top_clip(compact, &format!("{name} compact"));

        // Replace / append payload from compact mode.
        let field = shell.app.find(FIELD).unwrap();
        let editor = shell.app.ui.editor_within(field).unwrap();
        editor.set_selection(0, 1);
        type_at_caret(&mut shell.app, payload);

        let mut prev_rel = compact_rel;
        for step in 0..=35 {
            if step > 0 {
                shell.frames(1);
            }
            let f = inspect_composer_text(&mut shell.app);
            assert_no_top_clip(f, &format!("{name} frame {step}"));
            let rel = f.line_top_full - f.composer.y;
            // Compact -> expanded moves text.top monotonically from compact_rel (13.5) to expanded (17.0).
            assert!(
                rel >= compact_rel - 0.5 && rel <= 18.0,
                "{name} frame {step}: relative text top out of range ({rel}, compact={compact_rel}): {f:?}"
            );
            assert!(
                rel >= prev_rel - 0.5,
                "{name} frame {step}: relative text top moved backwards ({prev_rel} -> {rel}): {f:?}"
            );
            prev_rel = rel;
        }
    }

    // (e) Width-only resize while compact (target_geometry keeps previous width),
    // immediately followed by typing across the NEW narrower compact wrap boundary.
    let mut shell = Shell::new(false);
    shell.app.click(BUILD);
    shell.frames(60);
    shell.app.click(FIELD);
    shell.frames(2);
    type_at_caret(&mut shell.app, "short");
    shell.frames(30);
    // Narrow window while still compact on 1 line.
    shell.app.ui.scene.borrow_mut().resize(760., 800.);
    shell.app.settle();
    shell.frames(10);
    let compact_narrow = inspect_composer_text(&mut shell.app);
    assert_no_top_clip(compact_narrow, "post-narrow compact");
    // Cross the narrower compact room without crossing the old wider room.
    type_at_caret(&mut shell.app, &" m".repeat(26));
    for step in 0..=35 {
        if step > 0 {
            shell.frames(1);
        }
        let f = inspect_composer_text(&mut shell.app);
        assert_no_top_clip(f, &format!("post-narrow wrap frame {step}"));
    }
}

#[test]
fn composer_horizontal_window_resize_never_clips_top_of_text() {
    for session in [false, true] {
        let mode = if session { "docked" } else { "new-chat" };
        let mut shell = Shell::new(false);
        if session {
            shell.app.click(BUILD);
            shell.frames(60);
        }
        shell.app.click(FIELD);
        shell.frames(2);
        let text = "The quick brown fox jumps over the lazy dog and keeps running smoothly across the meadow while the window resizes.";
        type_at_caret(&mut shell.app, text);
        shell.frames(40);
        let base = inspect_composer_text(&mut shell.app);
        let base_rel = base.line_top_full - base.composer.y;
        assert_no_top_clip(base, &format!("{mode} wide"));

        // Smoothly drag window narrower (causing repeated line wraps), then wider again.
        for width in [1100., 980., 860., 760., 680., 620., 760., 960., 1200.] {
            shell.app.ui.scene.borrow_mut().resize(width, 800.);
            shell.app.settle();
            for step in 0..8 {
                if step > 0 {
                    shell.frames(1);
                }
                let f = inspect_composer_text(&mut shell.app);
                assert_no_top_clip(f, &format!("{mode} width={width} frame {step}"));
                let rel = f.line_top_full - f.composer.y;
                assert!(
                    (rel - base_rel).abs() <= 0.5,
                    "{mode} width={width} frame {step}: relative text top drifted ({rel} vs {base_rel}): {f:?}"
                );
            }
        }

        // Caret in the middle of a draft that wraps PAST the 240px cap when narrowed sharply:
        // Pass-1 pill.width must expand editor_height before caret reveal runs so Line 1 stays unclipped.
        let field = shell.app.find(FIELD).unwrap();
        let editor = shell.app.ui.editor_within(field).unwrap();
        let len = editor.editor.borrow().text().len();
        editor.set_selection(0, len);
        let mid_para = "alpha beta gamma delta epsilon zeta eta theta ".repeat(14);
        type_at_caret(&mut shell.app, mid_para.trim_end());
        shell.frames(40);
        // Place caret near line 2 (well inside 240px even when narrowed).
        let field = shell.app.find(FIELD).unwrap();
        let editor = shell.app.ui.editor_within(field).unwrap();
        editor.set_selection(40, 40);
        shell.frames(5);
        shell.app.ui.scene.borrow_mut().resize(620., 800.);
        shell.app.settle();
        for step in 0..=20 {
            if step > 0 {
                shell.frames(1);
            }
            let f = inspect_composer_text(&mut shell.app);
            assert_no_top_clip(f, &format!("{mode} mid-caret narrow past cap frame {step}"));
        }
    }
}

#[test]
fn composer_mid_animation_interruption_and_headers_never_clip_top_of_text() {
    // 1. Rapid mid-animation interruptions (grow -> paste more -> shrink -> grow).
    let mut shell = Shell::new(false);
    shell.app.click(BUILD);
    shell.frames(60);
    shell.app.click(FIELD);
    shell.frames(2);
    type_at_caret(&mut shell.app, "L1\nL2\nL3");
    shell.frames(35);
    let base = inspect_composer_text(&mut shell.app);
    let base_rel = base.line_top_full - base.composer.y;

    // Grow to 5 lines, interrupt at frame 3 with 3 more lines, interrupt at frame 3 with shrink to 4, interrupt with +2.
    type_at_caret(&mut shell.app, "\nL4\nL5");
    for step in 0..3 {
        if step > 0 { shell.frames(1); }
        let f = inspect_composer_text(&mut shell.app);
        assert_no_top_clip(f, &format!("interrupt phase 1 frame {step}"));
        assert!(((f.line_top_full - f.composer.y) - base_rel).abs() <= 0.5);
    }
    type_at_caret(&mut shell.app, "\nL6\nL7\nL8");
    for step in 0..3 {
        if step > 0 { shell.frames(1); }
        let f = inspect_composer_text(&mut shell.app);
        assert_no_top_clip(f, &format!("interrupt phase 2 frame {step}"));
        assert!(((f.line_top_full - f.composer.y) - base_rel).abs() <= 0.5);
    }
    let field = shell.app.find(FIELD).unwrap();
    let editor = shell.app.ui.editor_within(field).unwrap();
    let len = editor.editor.borrow().text().len();
    editor.set_selection(11, len); // keep "L1\nL2\nL3\nL4"
    type_at_caret(&mut shell.app, "");
    for step in 0..3 {
        if step > 0 { shell.frames(1); }
        let f = inspect_composer_text(&mut shell.app);
        assert_no_top_clip(f, &format!("interrupt phase 3 frame {step}"));
        assert!(((f.line_top_full - f.composer.y) - base_rel).abs() <= 0.5);
    }
    type_at_caret(&mut shell.app, "\nL5\nL6");
    for step in 0..=35 {
        if step > 0 { shell.frames(1); }
        let f = inspect_composer_text(&mut shell.app);
        assert_no_top_clip(f, &format!("interrupt phase 4 frame {step}"));
        assert!(((f.line_top_full - f.composer.y) - base_rel).abs() <= 0.5);
    }

    // 2. Cargo and question headers appearing/disappearing over a multi-line draft
    // never clip the top of text or let the header overlap line 1.
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui/composer_fixture.luau");
    let mut app = TestApp::new(|runtime| ScriptHost::new(runtime, path));
    app.ui.scene.borrow_mut().resize(700., 600.);
    app.frames(30);
    app.click("role:textbox");
    app.frames(2);
    type_at_caret(&mut app, "Header test line 1\nLine 2\nLine 3\nLine 4");
    app.frames(35);
    let no_header_warnings = zgx::inspect::layout_warnings(&app.ui).len();

    for (state, on, off, label) in [
        (
            "test_composer_cargo",
            r#"[{"id":"f1","kind":"file","name":"notes.md"}]"#,
            "[]",
            "cargo",
        ),
        (
            "test_composer_question",
            r#"{"text":"Choose a branch","options":[{"value":"main","label":"Main"}]}"#,
            "null",
            "question",
        ),
    ] {
        app.host.set_state(state, Value::from_json(on).unwrap()).unwrap();
        for step in 0..=30 {
            if step > 0 { app.frames(1); }
            let f = inspect_composer_text(&mut app);
            assert_no_top_clip(f, &format!("{label} open frame {step}"));
            if step == 3 || step == 30 {
                assert!(zgx::inspect::layout_warnings(&app.ui).len() <= no_header_warnings);
            }
        }
        app.host.set_state(state, Value::from_json(off).unwrap()).unwrap();
        for step in 0..=30 {
            if step > 0 { app.frames(1); }
            let f = inspect_composer_text(&mut app);
            assert_no_top_clip(f, &format!("{label} close frame {step}"));
        }
    }
}

#[test]
fn composer_writing_new_lines_never_clips_caret_at_bottom_or_text_at_top() {
    for session in [false, true] {
        let mode = if session { "docked" } else { "new-chat" };
        let mut shell = Shell::new(false);
        if session {
            shell.app.click(BUILD);
            shell.frames(60);
        }
        shell.app.click(FIELD);
        shell.frames(2);
        type_at_caret(&mut shell.app, "Line 1");
        shell.frames(30);
        let first = inspect_composer_text(&mut shell.app);
        assert_no_top_clip(first, &format!("{mode} line 1"));
        assert_caret_unclipped(first, &format!("{mode} line 1"));

        // Press Shift+Enter (newline) line by line from 1 -> 13 (crossing compact->expanded,
        // expanded min->3, 3->4..10->11 cap, and past cap 11->13).
        for line in 2..=13 {
            type_at_caret(&mut shell.app, "\n");
            let from_compact = session && line == 2;
            for step in 0..=25 {
                if step > 0 {
                    shell.frames(1);
                }
                let f = inspect_composer_text(&mut shell.app);
                // Lines 1..=10 fit under the 240px viewport cap so line 1 must stay unclipped;
                // at 11+ lines the viewport is capped and scrolls internally by design.
                if line <= 10 {
                    assert_no_top_clip(f, &format!("{mode} newline to {line} frame {step}"));
                }
                // Compact (49px) -> Expanded (120px) starts at p=0 on frame 0 before the first
                // animation tick; by frame 1 (16ms) line 2's caret must already be fully visible.
                if !from_compact || step >= 1 {
                    assert_caret_unclipped(f, &format!("{mode} newline to {line} frame {step}"));
                }
            }
            // Type text on the newly created line and verify caret stays completely unclipped.
            type_at_caret(&mut shell.app, &format!("Line {line}"));
            let typed = inspect_composer_text(&mut shell.app);
            assert_caret_unclipped(typed, &format!("{mode} typed on line {line}"));
        }

        // Also test auto-wrap growth (typing words across the right edge in expanded mode):
        // top line and bottom caret must both stay unclipped on every single character.
        let field = shell.app.find(FIELD).unwrap();
        let editor = shell.app.ui.editor_within(field).unwrap();
        let len = editor.editor.borrow().text().len();
        editor.set_selection(0, len);
        type_at_caret(&mut shell.app, "Line 1\nLine 2\n");
        shell.frames(30);
        for word in 0..22 {
            type_at_caret(&mut shell.app, "meadow ");
            let f0 = inspect_composer_text(&mut shell.app);
            assert_no_top_clip(f0, &format!("{mode} wrap word {word} frame 0"));
            assert_caret_unclipped(f0, &format!("{mode} wrap word {word} frame 0"));
            shell.frames(1);
            let f1 = inspect_composer_text(&mut shell.app);
            assert_no_top_clip(f1, &format!("{mode} wrap word {word} frame 1"));
            assert_caret_unclipped(f1, &format!("{mode} wrap word {word} frame 1"));
        }
    }
}

#[test]
fn composer_skinny_to_multiline_glides_text_and_caret_smoothly() {
    // 1. Shift+Enter from skinny (compact) -> multi-line (expanded), then Backspace back to skinny.
    let mut shell = Shell::new(false);
    shell.app.click(BUILD);
    shell.frames(60);
    shell.app.click(FIELD);
    shell.frames(2);
    type_at_caret(&mut shell.app, "Hello");
    shell.frames(30);
    let compact = inspect_composer_text(&mut shell.app);
    assert_no_top_clip(compact, "skinny base");
    assert_caret_unclipped(compact, "skinny base");

    type_at_caret(&mut shell.app, "\n");
    let mut prev = inspect_composer_text(&mut shell.app);
    assert!(
        (prev.line_left_full - compact.line_left_full).abs() <= 0.5,
        "frame 0 must start at compact text.left without jumping: prev={prev:?}, compact={compact:?}"
    );
    let mut max_dx_text = 0.0_f32;
    let mut max_dx_caret = 0.0_f32;
    for step in 1..=30 {
        shell.frames(1);
        let f = inspect_composer_text(&mut shell.app);
        assert_no_top_clip(f, &format!("skinny->multiline Shift+Enter frame {step}"));
        assert_caret_unclipped(f, &format!("skinny->multiline Shift+Enter frame {step}"));
        // Line 1 left edge and Line 2 caret (at col 0) must glide together in lockstep.
        assert!(
            (f.caret_full.x - f.line_left_full).abs() <= 0.5,
            "frame {step}: caret drifted horizontally from text ({} vs {}): {f:?}",
            f.caret_full.x,
            f.line_left_full
        );
        // Monotonic glide left (never jumps back right) with no 29.5px teleport.
        assert!(
            f.line_left_full <= prev.line_left_full + 0.5,
            "frame {step}: text moved backwards right ({} -> {}): {f:?}",
            prev.line_left_full,
            f.line_left_full
        );
        let dx_t = (f.line_left_full - prev.line_left_full).abs();
        let dx_c = (f.caret_full.x - prev.caret_full.x).abs();
        max_dx_text = max_dx_text.max(dx_t);
        max_dx_caret = max_dx_caret.max(dx_c);
        assert!(
            dx_t <= 8.0 && dx_c <= 8.0,
            "frame {step}: horizontal teleport detected (dx_text={dx_t}, dx_caret={dx_c}): prev={prev:?}, f={f:?}"
        );
        prev = f;
    }
    assert!(
        compact.line_left_full - prev.line_left_full > 25.0,
        "expected full compact->expanded inset travel (~29.5px), got {} -> {}",
        compact.line_left_full,
        prev.line_left_full
    );
    assert!(
        max_dx_text > 1.0 && max_dx_caret > 1.0,
        "expected smooth multi-frame glide, got max_dx_text={max_dx_text}, max_dx_caret={max_dx_caret}"
    );

    // Backspace back from expanded -> compact: smooth glide right, no frame-0 teleport.
    let expanded = prev;
    shell.app.key(Key::Backspace);
    let mut prev = inspect_composer_text(&mut shell.app);
    assert!(
        (prev.line_left_full - expanded.line_left_full).abs() <= 0.5,
        "contraction frame 0 must start at expanded text.left: prev={prev:?}, expanded={expanded:?}"
    );
    for step in 1..=30 {
        shell.frames(1);
        let f = inspect_composer_text(&mut shell.app);
        assert_no_top_clip(f, &format!("multiline->skinny frame {step}"));
        assert_caret_unclipped(f, &format!("multiline->skinny frame {step}"));
        assert!(
            f.line_left_full >= prev.line_left_full - 0.5,
            "contraction frame {step}: text moved backwards left ({} -> {}): {f:?}",
            prev.line_left_full,
            f.line_left_full
        );
        let dx_t = (f.line_left_full - prev.line_left_full).abs();
        let dx_c = (f.caret_full.x - prev.caret_full.x).abs();
        assert!(
            dx_t <= 14.0 && dx_c <= 14.0,
            "contraction frame {step}: horizontal teleport detected (dx_text={dx_t}, dx_caret={dx_c}): prev={prev:?}, f={f:?}"
        );
        prev = f;
    }
    assert!(
        (prev.line_left_full - compact.line_left_full).abs() <= 0.5,
        "expected settled contraction to return to compact.left"
    );

    // 2. Typing across the skinny compact width boundary: text must stay on 1 line
    // (never falsely wrap to line 2 and unwrap back to line 1) while gliding smoothly left.
    let mut shell = Shell::new(false);
    shell.app.click(BUILD);
    shell.frames(60);
    shell.app.click(FIELD);
    shell.frames(2);
    type_at_caret(&mut shell.app, "a");
    shell.frames(15);
    let compact_h = inspect_composer_text(&mut shell.app).composer.height;
    let mut steps = 0;
    let mut expanded_triggered = false;
    while inspect_composer_text(&mut shell.app).composer.height <= compact_h + 0.5 {
        assert!(steps < 120, "never crossed compact boundary");
        type_at_caret(&mut shell.app, "w");
        steps += 1;
        let before_tick = inspect_composer_text(&mut shell.app);
        shell.frames(1);
        let after_tick = inspect_composer_text(&mut shell.app);
        if after_tick.composer.height > compact_h + 0.5 {
            expanded_triggered = true;
            // Caret must stay on Line 1 across the entire expansion (no wrap-to-2 then unwrap-to-1 jump).
            assert!(
                (before_tick.caret_full.y - before_tick.line_top_full).abs() <= 2.0,
                "wrap trigger frame 0 falsely wrapped caret to line 2: {before_tick:?}"
            );
            assert!(
                (after_tick.caret_full.y - after_tick.line_top_full).abs() <= 2.0,
                "wrap trigger frame 1 falsely wrapped caret to line 2: {after_tick:?}"
            );
            let mut prev = after_tick;
            for step in 2..=30 {
                shell.frames(1);
                let f = inspect_composer_text(&mut shell.app);
                assert_no_top_clip(f, &format!("auto-wrap skinny->expanded frame {step}"));
                assert_caret_unclipped(f, &format!("auto-wrap skinny->expanded frame {step}"));
                assert!(
                    (f.caret_full.y - f.line_top_full).abs() <= 2.0,
                    "auto-wrap frame {step}: caret jumped off line 1: {f:?}"
                );
                let dx_t = (f.line_left_full - prev.line_left_full).abs();
                let dx_c = (f.caret_full.x - prev.caret_full.x).abs();
                assert!(
                    dx_t <= 8.0 && dx_c <= 8.0,
                    "auto-wrap frame {step}: horizontal teleport (dx_text={dx_t}, dx_caret={dx_c}): prev={prev:?}, f={f:?}"
                );
                prev = f;
            }
            break;
        }
    }
    assert!(expanded_triggered, "never triggered compact->expanded via typing");
}



