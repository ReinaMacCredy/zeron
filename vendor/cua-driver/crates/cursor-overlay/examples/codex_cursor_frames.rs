use cursor_overlay::{
    paint_cursor, CursorAction, CursorConfig, OverlayCommand, RenderStateCore, CODEX_THEME_ID,
};
use std::path::{Path, PathBuf};

const SIZE: u32 = 252;
const SCALE: f64 = 2.0;
const TIP: (f64, f64) = (62.625 - 1.22, 63.405 - 2.915);

fn codex() -> RenderStateCore {
    let mut core = RenderStateCore::new(CursorConfig {
        compact: true,
        theme_id: CODEX_THEME_ID.into(),
        ..CursorConfig::default()
    });
    core.motion.idle_hide_ms = 0.0;
    core.screen_size = Some((1512.0, 982.0));
    core
}

fn place(core: &mut RenderStateCore, x: f64, y: f64) {
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

fn settle(core: &mut RenderStateCore) {
    for _ in 0..240 {
        core.tick_compact(1.0 / 120.0);
    }
}

fn save(core: &RenderStateCore, dir: &Path, name: &str) {
    let mut pm = tiny_skia::Pixmap::new(SIZE, SIZE).unwrap();
    let (x, y) = core.compact_display_pos();
    paint_cursor(&mut pm, core, x - TIP.0, y - TIP.1, None, SCALE as f32);
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for pixel in pm.pixels() {
        let c = pixel.demultiply();
        rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    let path = dir.join(format!("{name}.png"));
    image::save_buffer(&path, &rgba, SIZE, SIZE, image::ColorType::Rgba8).unwrap();
    println!("{}", path.display());
}

fn main() {
    let dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "/tmp/codex-cu-probe/slice4".into()),
    );
    std::fs::create_dir_all(&dir).unwrap();
    let tints: [(&str, Option<[u8; 3]>); 4] = [
        ("orange", Some([138, 114, 97])),
        ("green", Some([108, 102, 88])),
        ("pink", Some([113, 97, 95])),
        ("grey", None),
    ];

    for (name, tint) in tints {
        let mut core = codex();
        core.fog_tint = tint;
        place(&mut core, 400.0, 300.0);
        settle(&mut core);
        save(&core, &dir, &format!("idle_{name}"));
    }

    let mut core = codex();
    core.fog_tint = Some([138, 114, 97]);
    place(&mut core, 400.0, 300.0);
    settle(&mut core);
    core.apply_command_base(
        OverlayCommand::ClickPulse { x: 400.0, y: 300.0 },
        true,
        true,
    );
    for _ in 0..12 {
        core.tick_compact(1.0 / 120.0);
    }
    println!("pressed scale {:.3}", core.codex.channels().scale);
    save(&core, &dir, "pressed_orange");

    for (name, from, target, tint) in [
        (
            "move_right_orange",
            (300.0, 300.0),
            (1100.0, 300.0),
            Some([138, 114, 97]),
        ),
        (
            "move_left_orange",
            (1100.0, 300.0),
            (100.0, 500.0),
            Some([138, 114, 97]),
        ),
        ("hop_left_grey", (600.0, 300.0), (440.0, 340.0), None),
    ] {
        let mut core = codex();
        core.fog_tint = tint;
        place(&mut core, from.0, from.1);
        settle(&mut core);
        place(&mut core, target.0, target.1);
        let mut best = (0.0_f64, 0usize);
        let mut probe = core.clone_for_probe();
        for i in 0..240 {
            probe.tick_compact(1.0 / 120.0);
            let rotation = probe.codex.channels().rotation.abs();
            if rotation > best.0 {
                best = (rotation, i);
            }
        }
        for _ in 0..=best.1 {
            core.tick_compact(1.0 / 120.0);
        }
        let ch = core.codex.channels();
        println!(
            "{name}: tilt {:.1} deg stretch {:.3} squash {:.3}",
            ch.rotation.to_degrees(),
            ch.stretch,
            ch.squash
        );
        save(&core, &dir, name);
    }

    let mut core = codex();
    core.fog_tint = Some([108, 102, 88]);
    place(&mut core, 400.0, 300.0);
    settle(&mut core);
    core.apply_command_base(
        OverlayCommand::BeginAction {
            action: CursorAction::Observe,
            delivery: None,
            target: None,
        },
        true,
        true,
    );
    for _ in 0..60 {
        core.tick_compact(1.0 / 120.0);
    }
    save(&core, &dir, "loading_green");

    let mut core = codex();
    core.fog_tint = Some([138, 114, 97]);
    place(&mut core, 400.0, 300.0);
    core.tick_compact(0.0);
    core.tick_compact(cursor_overlay::codex_cursor::REVEAL_SECS * 0.5);
    save(&core, &dir, "fade_in_orange");
}

trait Probe {
    fn clone_for_probe(&self) -> RenderStateCore;
}

impl Probe for RenderStateCore {
    fn clone_for_probe(&self) -> RenderStateCore {
        let mut probe = codex();
        probe.fog_tint = self.fog_tint;
        probe.pos = self.pos;
        probe.compact_motion = self.compact_motion.clone();
        probe.codex = self.codex.clone();
        probe
    }
}
