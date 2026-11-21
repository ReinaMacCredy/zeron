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
