use super::*;

fn codex() -> RenderStateCore {
    let mut core = RenderStateCore::new(CursorConfig {
        compact: true,
        theme_id: crate::CODEX_THEME_ID.into(),
        ..CursorConfig::default()
    });
    core.motion.idle_hide_ms = 0.0;
    core
}

fn snap(core: &mut RenderStateCore, x: f64, y: f64) {
    core.apply_command_base(
        OverlayCommand::SnapTo {
            x,
            y,
            heading_radians: None,
        },
        true,
        true,
    );
}

fn run(core: &mut RenderStateCore, secs: f64) {
    for _ in 0..(secs * 120.0) as usize {
        core.tick_compact(1.0 / 120.0);
    }
}

fn ink(core: &RenderStateCore) -> usize {
    let mut pm = tiny_skia::Pixmap::new(200, 200).unwrap();
    paint_cursor(&mut pm, core, 0.0, 0.0, None, 1.0);
    pm.pixels().iter().filter(|p| p.alpha() > 0).count()
}

#[test]
fn theme_selection_switches_between_compact_looks() {
    let mut core = codex();
    assert!(core.codex_look());
    assert_eq!(core.active_theme_metadata().0, crate::CODEX_THEME_ID);
    core.apply_command_base(
        OverlayCommand::SetTheme {
            theme_id: crate::COMPACT_THEME_ID.into(),
            reduced_motion: ReducedMotion::Auto,
        },
        true,
        true,
    );
    assert!(!core.codex_look());
    assert_eq!(core.active_theme_metadata().0, crate::COMPACT_THEME_ID);
    core.apply_command_base(
        OverlayCommand::SetTheme {
            theme_id: crate::CODEX_THEME_ID.into(),
            reduced_motion: ReducedMotion::Auto,
        },
        true,
        true,
    );
    assert!(core.codex_look());
}

#[test]
fn first_placement_fades_in_then_goes_quiet() {
    let mut core = codex();
    snap(&mut core, 100.0, 100.0);
    core.tick_compact(0.0);
    assert!(core.compact_animating());
    assert_eq!(ink(&core), 0);
    core.tick_compact(crate::codex_cursor::REVEAL_SECS / 2.0);
    assert!(ink(&core) > 0);
    run(&mut core, 1.0);
    assert_eq!(core.codex.channels().alpha, 1.0);
    assert!(!core.compact_animating());
}

#[test]
fn click_presses_and_hide_fades_out() {
    let mut core = codex();
    snap(&mut core, 100.0, 100.0);
    run(&mut core, 1.0);
    core.apply_command_base(
        OverlayCommand::ClickPulse { x: 100.0, y: 100.0 },
        true,
        true,
    );
    assert!(core.compact_animating());
    run(&mut core, 0.1);
    assert!(core.codex.channels().scale < 0.9);
    run(&mut core, 1.0);
    assert!((core.codex.channels().scale - 1.0).abs() < 1.0e-2);

    core.apply_command_base(OverlayCommand::SetEnabled(false), true, true);
    core.tick_compact(crate::codex_cursor::REVEAL_SECS / 2.0);
    assert!(ink(&core) > 0, "hiding fades out instead of popping");
    run(&mut core, 1.0);
    assert_eq!(ink(&core), 0);
    assert!(!core.compact_animating());
}

#[test]
fn observing_spins_until_the_action_ends() {
    let mut core = codex();
    snap(&mut core, 100.0, 100.0);
    run(&mut core, 1.0);
    core.apply_command_base(
        OverlayCommand::BeginAction {
            action: CursorAction::Observe,
            delivery: None,
            target: None,
        },
        true,
        true,
    );
    run(&mut core, 0.5);
    assert!(core.codex.channels().rotation > 1.0);
    assert!(core.compact_animating());
    core.apply_command_base(OverlayCommand::EndAction(CursorAction::Observe), true, true);
    run(&mut core, 3.0);
    assert_eq!(core.codex.channels().rotation, 0.0);
    assert!(!core.compact_animating());
}

#[test]
fn fog_uses_the_sampled_tint_and_the_style_overrides_it() {
    let mut core = codex();
    snap(&mut core, 100.0, 100.0);
    run(&mut core, 1.0);
    let fog_at = |core: &RenderStateCore| {
        let mut pm = tiny_skia::Pixmap::new(200, 200).unwrap();
        paint_cursor(&mut pm, core, 0.0, 0.0, None, 1.0);
        pm.pixel(85, 103).unwrap().demultiply()
    };
    let grey = fog_at(&core);
    for (channel, expected) in [grey.red(), grey.green(), grey.blue()]
        .into_iter()
        .zip(crate::codex_cursor::FOG_FALLBACK)
    {
        assert!(channel.abs_diff(expected) <= 3, "{grey:?}");
    }
    core.fog_tint = Some([40, 120, 60]);
    let tinted = fog_at(&core);
    assert!(tinted.green() > tinted.red() + 40);
    core.style.shadow = Some([200, 30, 30]);
    let styled = fog_at(&core);
    assert!(styled.red() > styled.green() + 100);
}

#[test]
fn an_unmatched_observe_stops_spinning_at_its_nominal_duration() {
    let mut core = codex();
    snap(&mut core, 100.0, 100.0);
    run(&mut core, 1.0);
    core.apply_command_base(
        OverlayCommand::BeginAction {
            action: CursorAction::Observe,
            delivery: None,
            target: None,
        },
        true,
        true,
    );
    run(&mut core, 4.0);
    assert_eq!(core.codex.channels().rotation, 0.0);
    assert!(!core.compact_animating());
}

#[test]
fn travel_on_a_display_above_the_primary_stays_on_that_display() {
    let mut core = codex();
    core.screen_origin = (-44.0, -1080.0);
    core.screen_size = Some((1920.0, 1080.0));
    snap(&mut core, 1700.0, -1000.0);
    assert!(core.is_placed());
    run(&mut core, 1.0);
    snap(&mut core, 0.0, -60.0);
    assert!(core.compact_motion.is_some());
    for _ in 0..600 {
        core.tick_compact(1.0 / 120.0);
        let (x, y) = core.compact_display_pos();
        assert!(
            (-44.0..=1876.0).contains(&x) && (-1080.0..=0.0).contains(&y),
            "({x}, {y}) left the display"
        );
    }
    assert_eq!(core.compact_display_pos(), (0.0, -60.0));
}
