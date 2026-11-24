//! Lightbox anti-flicker regression coverage across morph open and morph→rest handoff.

use std::path::PathBuf;

use zgx::scene::NodeKind;
use zgx_script::{ScriptHost, testing::TestApp};

fn ui_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui")
}

fn mount_fixture(name: &str) -> TestApp {
    let path = ui_dir().join(name);
    let app = TestApp::new(|runtime| ScriptHost::new(runtime, path));
    app.ui.scene.borrow_mut().resize(1200., 800.);
    app.frames(25);
    app
}

fn run_cmd(app: &TestApp, id: &str) {
    assert!(app.host.run_command(id), "command {id:?} did not run");
}

#[test]
fn lightbox_morph_to_rest_handoff_never_blanks_a_frame() {
    let mut app = mount_fixture("lightbox_fixture.luau");
    run_cmd(&app, "test.lightbox_snapshots");
    app.frames(6);

    // Click the first thumbnail to trigger a morph-in open.
    app.click("\"Open shell.png\"");

    let count_visible_lightbox_images = |app: &mut TestApp| -> (usize, usize) {
        let Some(root) = app.find("role:dialog \"Image viewer\"") else {
            return (0, 0);
        };
        app.ui.prepare_frame();
        let scene = app.ui.scene.borrow();
        let mut visible_full = 0usize;
        let mut prewarm = 0usize;
        let mut stack = vec![(root, 1.0_f32)];
        while let Some((id, parent_op)) = stack.pop() {
            let op = parent_op * scene.effects(id).opacity;
            if let NodeKind::Image { .. } = scene.kind(id) {
                if op > 0.5 {
                    visible_full += 1;
                } else if op > 0.0 && op <= 0.02 {
                    prewarm += 1;
                }
            }
            for &child in scene.children(id) {
                stack.push((child, op));
            }
        }
        (visible_full, prewarm)
    };

    let mut saw_prewarm = false;
    let mut blank_frames = 0usize;
    // Step through the entire morph (280ms ~ 17 frames) plus the 180ms handoff grace (~45 frames).
    for frame in 0..45 {
        app.frames(1);
        let (visible, prewarm) = count_visible_lightbox_images(&mut app);
        if prewarm > 0 {
            saw_prewarm = true;
        }
        if frame >= 2 && visible == 0 {
            blank_frames += 1;
        }
    }

    assert!(
        saw_prewarm,
        "morph-in should mount a target-size async pre-warm image so the rest image is decoded before handoff"
    );
    assert_eq!(
        blank_frames, 0,
        "lightbox must never show a blank frame during morph or morph->rest handoff"
    );
}

#[test]
fn lightbox_luau_suite_passes() {
    let ui = ui_dir();
    let filters = [
        "a thumbnail opens its picture fitted",
        "a composer chip opens the chips' pictures",
        "zooming a picture:",
        "a click on the picture doesn't close it",
        "a picture smaller than the window shows at its own size",
        "with motion reduced (Zeron's own setting), it opens and closes as a short crossfade",
        "with motion as it is, it grows out of the thumbnail",
        "a picture is never stretched or squeezed as it grows from its thumbnail",
        "zooming never leaves a frame without the picture",
    ];
    for filter in filters {
        let report = zgx_script::test_runner::run(&ui, Some(filter), |outcome| {
            if let Some(failure) = &outcome.failure {
                eprintln!("{}: {failure}", outcome.name);
            }
        })
        .expect("lightbox test runs");
        assert!(!report.outcomes.is_empty(), "no test matched {filter:?}");
        assert_eq!(report.failed(), 0, "{report:?}");
    }
}
