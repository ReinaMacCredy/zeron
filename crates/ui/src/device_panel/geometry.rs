use gpui::{Bounds, Keystroke, Pixels, Point, point, px, size};
use zeron_proto::device_hub::{
    DeviceButton, DeviceInput, DeviceOrientation, DeviceScreenConfig, KeyPhase,
};

pub const IOS_PLACEHOLDER_ASPECT: f32 = 9.0 / 19.5;
pub const ANDROID_PLACEHOLDER_ASPECT: f32 = 9.0 / 20.0;
pub const RAIL_WIDTH: f32 = 56.0;

const SHIFT: u16 = 0xE1;
const CONTROL: u16 = 0xE0;
const ALT: u16 = 0xE2;
const COMMAND: u16 = 0xE3;
const NAMED_KEYS: [(&str, u16); 10] = [
    ("enter", 0x28),
    ("escape", 0x29),
    ("backspace", 0x2A),
    ("tab", 0x2B),
    ("space", 0x2C),
    ("delete", 0x4C),
    ("right", 0x4F),
    ("left", 0x50),
    ("down", 0x51),
    ("up", 0x52),
];
const PUNCTUATION: [(char, char, u16); 11] = [
    ('-', '_', 0x2D),
    ('=', '+', 0x2E),
    ('[', '{', 0x2F),
    (']', '}', 0x30),
    ('\\', '|', 0x31),
    (';', ':', 0x33),
    ('\'', '"', 0x34),
    ('`', '~', 0x35),
    (',', '<', 0x36),
    ('.', '>', 0x37),
    ('/', '?', 0x38),
];
const SHIFTED_DIGITS: [char; 10] = ['!', '@', '#', '$', '%', '^', '&', '*', '(', ')'];

const ANDROID_KEYCODES: [(&str, u16); 12] = [
    ("up", 19),
    ("down", 20),
    ("left", 21),
    ("right", 22),
    ("tab", 61),
    ("enter", 66),
    ("backspace", 67),
    ("delete", 112),
    ("home", 122),
    ("end", 123),
    ("pageup", 92),
    ("pagedown", 93),
];

const META_SHIFT: u32 = 0x1 | 0x40;
const META_ALT: u32 = 0x2 | 0x10;
const META_CTRL: u32 = 0x1000 | 0x2000;

fn android_character(key: &str) -> Option<u16> {
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c @ 'a'..='z'), None) => Some(29 + (c as u16 - 'a' as u16)),
        (Some(c @ '0'..='9'), None) => Some(7 + (c as u16 - '0' as u16)),
        _ => None,
    }
}

pub fn android_input(keystroke: &Keystroke) -> Option<DeviceInput> {
    if keystroke.key == "escape" {
        return Some(DeviceInput::Button {
            button: DeviceButton::Back,
        });
    }
    let held = &keystroke.modifiers;
    let keycode = ANDROID_KEYCODES
        .iter()
        .find(|(name, _)| *name == keystroke.key)
        .map(|(_, keycode)| *keycode)
        .or_else(|| {
            held.control
                .then(|| android_character(&keystroke.key))
                .flatten()
        });
    if let Some(keycode) = keycode {
        let meta_state = [
            (held.shift, META_SHIFT),
            (held.alt, META_ALT),
            (held.control, META_CTRL),
        ]
        .into_iter()
        .filter_map(|(on, bits)| on.then_some(bits))
        .fold(0, |state, bits| state | bits);
        return Some(DeviceInput::Keycode {
            keycode,
            meta_state,
        });
    }
    if held.control || held.platform {
        return None;
    }
    let text = keystroke.key_char.as_deref().unwrap_or(&keystroke.key);
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if !c.is_control() => Some(DeviceInput::Text {
            text: c.to_string(),
        }),
        _ => None,
    }
}

// macOS reports shift + digit or punctuation as the shifted character with shift cleared.
pub fn hid_usage(key: &str) -> Option<(u16, bool)> {
    if let Some((_, usage)) = NAMED_KEYS.iter().find(|(name, _)| *name == key) {
        return Some((*usage, false));
    }
    let mut chars = key.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return None;
    };
    match c {
        'a'..='z' => Some((0x04 + (c as u16 - 'a' as u16), false)),
        '1'..='9' => Some((0x1E + (c as u16 - '1' as u16), false)),
        '0' => Some((0x27, false)),
        _ => SHIFTED_DIGITS
            .iter()
            .position(|shifted| *shifted == c)
            .map(|i| (0x1E + i as u16, true))
            .or_else(|| {
                PUNCTUATION.iter().find_map(|(plain, shifted, usage)| {
                    (c == *plain)
                        .then_some((*usage, false))
                        .or((c == *shifted).then_some((*usage, true)))
                })
            }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPress {
    pub usage: u16,
    modifiers: Vec<u16>,
}

impl KeyPress {
    pub fn from_keystroke(keystroke: &Keystroke) -> Option<Self> {
        let (usage, shifted) = hid_usage(&keystroke.key)?;
        let held = &keystroke.modifiers;
        let modifiers = [
            (held.control, CONTROL),
            (held.alt, ALT),
            (held.shift || shifted, SHIFT),
            (held.platform, COMMAND),
        ]
        .into_iter()
        .filter_map(|(on, usage)| on.then_some(usage))
        .collect();
        Some(Self { usage, modifiers })
    }

    // Only the bare key stays held, so a modifier never leaks into the next key of a rollover.
    pub fn down(&self) -> Vec<DeviceInput> {
        let downs = self.modifiers.iter().chain([&self.usage]);
        let ups = self.modifiers.iter().rev();
        downs
            .map(|&usage| key(KeyPhase::Down, usage))
            .chain(ups.map(|&usage| key(KeyPhase::Up, usage)))
            .collect()
    }
}

fn key(phase: KeyPhase, usage: u16) -> DeviceInput {
    DeviceInput::Key { phase, usage }
}

pub fn key_up(usage: u16) -> DeviceInput {
    key(KeyPhase::Up, usage)
}

pub fn command_r() -> Vec<DeviceInput> {
    vec![
        key(KeyPhase::Down, COMMAND),
        key(KeyPhase::Down, 0x15),
        key(KeyPhase::Up, 0x15),
        key(KeyPhase::Up, COMMAND),
    ]
}

pub fn next_orientation(current: DeviceOrientation) -> DeviceOrientation {
    match current {
        DeviceOrientation::Portrait => DeviceOrientation::LandscapeLeft,
        DeviceOrientation::LandscapeLeft => DeviceOrientation::PortraitUpsideDown,
        DeviceOrientation::PortraitUpsideDown => DeviceOrientation::LandscapeRight,
        DeviceOrientation::LandscapeRight => DeviceOrientation::Portrait,
    }
}

pub fn display_rotation(screen: Option<&DeviceScreenConfig>) -> DeviceOrientation {
    screen
        .filter(|screen| screen.width <= screen.height)
        .map_or(DeviceOrientation::Portrait, |screen| screen.orientation)
}

pub fn rotate_for_display(
    image: image::RgbaImage,
    rotation: DeviceOrientation,
) -> image::RgbaImage {
    match rotation {
        DeviceOrientation::Portrait => image,
        DeviceOrientation::LandscapeLeft => image::imageops::rotate90(&image),
        DeviceOrientation::PortraitUpsideDown => image::imageops::rotate180(&image),
        DeviceOrientation::LandscapeRight => image::imageops::rotate270(&image),
    }
}

pub fn fit_device_frame(aspect: f32, width: f32, height: f32, right_inset: f32) -> (f32, f32) {
    let available = (width - right_inset).max(0.0);
    if available == 0.0 || height == 0.0 {
        return (0.0, 0.0);
    }
    let by_height = (height * aspect, height);
    if by_height.0 <= available {
        by_height
    } else {
        (available, available / aspect)
    }
}

pub fn centered_frame(bounds: Bounds<Pixels>, aspect: f32, right_inset: f32) -> Bounds<Pixels> {
    let (width, height) = fit_device_frame(
        aspect,
        f32::from(bounds.size.width),
        f32::from(bounds.size.height),
        right_inset,
    );
    let origin = point(
        bounds.origin.x + (bounds.size.width - px(right_inset) - px(width)) / 2.0,
        bounds.origin.y + (bounds.size.height - px(height)) / 2.0,
    );
    Bounds::new(origin, size(px(width), px(height)))
}

pub fn normalized(frame: Bounds<Pixels>, position: Point<Pixels>) -> (f32, f32) {
    let ratio = |offset: Pixels, extent: Pixels| {
        if extent <= px(0.0) {
            0.0
        } else {
            (f32::from(offset) / f32::from(extent)).clamp(0.0, 1.0)
        }
    };
    (
        ratio(position.x - frame.origin.x, frame.size.width),
        ratio(position.y - frame.origin.y, frame.size.height),
    )
}

pub fn position_at(frame: Bounds<Pixels>, x: f32, y: f32) -> Point<Pixels> {
    point(
        frame.origin.x + frame.size.width * x.clamp(0.0, 1.0),
        frame.origin.y + frame.size.height * y.clamp(0.0, 1.0),
    )
}

pub fn raw_point(rotation: DeviceOrientation, x: f32, y: f32) -> (f32, f32) {
    match rotation {
        DeviceOrientation::LandscapeLeft => (y, 1.0 - x),
        DeviceOrientation::LandscapeRight => (1.0 - y, x),
        DeviceOrientation::PortraitUpsideDown => (1.0 - x, 1.0 - y),
        DeviceOrientation::Portrait => (x, y),
    }
}

pub fn display_point(rotation: DeviceOrientation, x: f32, y: f32) -> (f32, f32) {
    match rotation {
        DeviceOrientation::LandscapeLeft => (1.0 - y, x),
        DeviceOrientation::LandscapeRight => (y, 1.0 - x),
        DeviceOrientation::PortraitUpsideDown => (1.0 - x, 1.0 - y),
        DeviceOrientation::Portrait => (x, y),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn fits_a_flat_device_beside_the_fixed_controls_rail() {
        let (width, height) = fit_device_frame(IOS_PLACEHOLDER_ASPECT, 480.0, 1000.0, 56.0);
        assert!(width + 56.0 <= 480.0);
        assert!(height <= 1000.0);
        assert!(close(width / height, IOS_PLACEHOLDER_ASPECT));
    }

    #[test]
    fn height_limited_first_then_width_limited_and_empty_when_no_room() {
        assert_eq!(fit_device_frame(0.5, 1000.0, 400.0, 0.0), (200.0, 400.0));
        assert_eq!(fit_device_frame(0.5, 100.0, 400.0, 0.0), (100.0, 200.0));
        assert_eq!(fit_device_frame(0.5, 56.0, 400.0, 56.0), (0.0, 0.0));
        assert_eq!(fit_device_frame(0.5, 100.0, 0.0, 0.0), (0.0, 0.0));
    }

    #[test]
    fn the_frame_is_centred_and_points_normalize_with_clamping() {
        let bounds = Bounds::new(point(px(10.0), px(20.0)), size(px(400.0), px(400.0)));
        let frame = centered_frame(bounds, 0.5, 0.0);
        assert_eq!(frame.origin, point(px(110.0), px(20.0)));
        assert_eq!(frame.size, size(px(200.0), px(400.0)));
        assert_eq!(normalized(frame, point(px(160.0), px(220.0))), (0.25, 0.5));
        assert_eq!(normalized(frame, point(px(0.0), px(900.0))), (0.0, 1.0));
        assert_eq!(position_at(frame, 0.25, 0.5), point(px(160.0), px(220.0)));
    }

    #[test]
    fn the_rail_inset_keeps_the_frame_left_of_the_rail() {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(456.0), px(100.0)));
        let frame = centered_frame(bounds, 2.0, RAIL_WIDTH);
        assert_eq!(frame.size, size(px(200.0), px(100.0)));
        assert_eq!(frame.origin, point(px(100.0), px(0.0)));
    }

    #[test]
    fn every_table_key_round_trips_through_keystroke_parse() {
        let mut names: Vec<String> = NAMED_KEYS
            .iter()
            .map(|(name, _)| name.to_string())
            .collect();
        names.extend(('a'..='z').chain('0'..='9').map(String::from));
        names.extend(PUNCTUATION.iter().map(|(plain, ..)| plain.to_string()));
        for name in &names {
            let keystroke = Keystroke::parse(name).unwrap_or_else(|_| panic!("{name}"));
            assert_eq!(&keystroke.key, name);
            let (usage, shifted) = hid_usage(&keystroke.key).unwrap_or_else(|| panic!("{name}"));
            assert!(!shifted, "{name}");
            assert!((0x04..=0x52).contains(&usage), "{name}");
        }
        let shifted = SHIFTED_DIGITS
            .iter()
            .chain(PUNCTUATION.iter().map(|(_, shifted, _)| shifted));
        for name in shifted.map(char::to_string) {
            let keystroke = Keystroke::parse(&name).unwrap_or_else(|_| panic!("{name}"));
            assert_eq!(keystroke.key, name);
            assert!(
                matches!(hid_usage(&keystroke.key), Some((_, true))),
                "{name}"
            );
        }
        assert_eq!(hid_usage("a"), Some((0x04, false)));
        assert_eq!(hid_usage("z"), Some((0x1D, false)));
        assert_eq!(hid_usage("1"), Some((0x1E, false)));
        assert_eq!(hid_usage("0"), Some((0x27, false)));
        assert_eq!(hid_usage("/"), Some((0x38, false)));
        assert_eq!(hid_usage("up"), Some((0x52, false)));
        assert_eq!(hid_usage("f1"), None);
        assert_eq!(hid_usage("ä"), None);
    }

    #[test]
    fn shift_wraps_the_key_for_letters_and_shifted_characters() {
        let press = |keystroke: Keystroke| {
            let press = KeyPress::from_keystroke(&keystroke).unwrap();
            [press.down(), vec![key_up(press.usage)]].concat()
        };
        let wrapped = |usage| {
            vec![
                key(KeyPhase::Down, SHIFT),
                key(KeyPhase::Down, usage),
                key(KeyPhase::Up, SHIFT),
                key(KeyPhase::Up, usage),
            ]
        };
        assert_eq!(press(Keystroke::parse("shift-a").unwrap()), wrapped(0x04));
        let typed = |key: &str| Keystroke {
            key: key.into(),
            ..Default::default()
        };
        for (shifted, usage) in [
            ("!", 0x1E),
            (")", 0x27),
            (":", 0x33),
            ("\"", 0x34),
            ("?", 0x38),
            ("_", 0x2D),
            ("~", 0x35),
        ] {
            assert_eq!(press(typed(shifted)), wrapped(usage), "{shifted}");
        }
        assert_eq!(
            press(typed("a")),
            vec![key(KeyPhase::Down, 0x04), key(KeyPhase::Up, 0x04)]
        );
    }

    #[test]
    fn a_shifted_key_held_during_rollover_does_not_shift_the_next_key() {
        let down = |text: &str| {
            KeyPress::from_keystroke(&Keystroke::parse(text).unwrap())
                .unwrap()
                .down()
        };
        assert_eq!(
            [down("shift-t"), down("h")].concat(),
            vec![
                key(KeyPhase::Down, SHIFT),
                key(KeyPhase::Down, 0x17),
                key(KeyPhase::Up, SHIFT),
                key(KeyPhase::Down, 0x0B),
            ]
        );
        assert_eq!(
            [down("!"), down("a")].concat(),
            vec![
                key(KeyPhase::Down, SHIFT),
                key(KeyPhase::Down, 0x1E),
                key(KeyPhase::Up, SHIFT),
                key(KeyPhase::Down, 0x04),
            ]
        );
    }

    #[test]
    fn android_keys_become_keycodes_back_or_typed_text() {
        let input = |text: &str| android_input(&Keystroke::parse(text).unwrap());
        let keycode = |keycode| {
            Some(DeviceInput::Keycode {
                keycode,
                meta_state: 0,
            })
        };
        let chord = |keycode, meta_state| {
            Some(DeviceInput::Keycode {
                keycode,
                meta_state,
            })
        };
        let typed = |text: &str| Some(DeviceInput::Text { text: text.into() });
        assert_eq!(
            input("escape"),
            Some(DeviceInput::Button {
                button: DeviceButton::Back
            })
        );
        for (key, code) in ANDROID_KEYCODES {
            assert_eq!(input(key), keycode(code), "{key}");
        }
        assert_eq!(input("a"), typed("a"));
        assert_eq!(input("7"), typed("7"));
        assert_eq!(input("/"), typed("/"));
        assert_eq!(
            android_input(&Keystroke {
                key: "a".into(),
                key_char: Some("A".into()),
                modifiers: gpui::Modifiers::shift(),
                ..Default::default()
            }),
            typed("A"),
            "the typed character wins, so no shift juggling"
        );
        assert_eq!(
            android_input(&Keystroke {
                key: "space".into(),
                key_char: Some(" ".into()),
                ..Default::default()
            }),
            typed(" ")
        );
        assert_eq!(input("shift-tab"), chord(61, 0x41));
        assert_eq!(input("shift-left"), chord(21, 0x41));
        assert_eq!(input("alt-backspace"), chord(67, 0x12));
        assert_eq!(input("ctrl-a"), chord(29, 0x3000));
        assert_eq!(input("ctrl-shift-z"), chord(54, 0x3041));
        assert_eq!(input("ctrl-0"), chord(7, 0x3000));
        assert_eq!(input("ctrl-/"), None);
        assert_eq!(input("cmd-v"), None);
        assert_eq!(input("f1"), None);
    }

    #[test]
    fn rotate_cycles_portrait_left_upside_down_right() {
        let mut orientation = DeviceOrientation::Portrait;
        let mut seen = Vec::new();
        for _ in 0..4 {
            orientation = next_orientation(orientation);
            seen.push(orientation);
        }
        assert_eq!(
            seen,
            vec![
                DeviceOrientation::LandscapeLeft,
                DeviceOrientation::PortraitUpsideDown,
                DeviceOrientation::LandscapeRight,
                DeviceOrientation::Portrait,
            ]
        );
    }

    #[test]
    fn a_rotated_frame_and_the_touch_remap_agree_on_every_pixel() {
        let (width, height) = (4u32, 8u32);
        let raw = (1u32, 2u32);
        let mut image = image::RgbaImage::new(width, height);
        image.put_pixel(raw.0, raw.1, image::Rgba([255, 0, 0, 255]));
        for orientation in [
            DeviceOrientation::Portrait,
            DeviceOrientation::LandscapeLeft,
            DeviceOrientation::PortraitUpsideDown,
            DeviceOrientation::LandscapeRight,
        ] {
            let screen = DeviceScreenConfig {
                width,
                height,
                orientation,
                ..Default::default()
            };
            let shown = rotate_for_display(image.clone(), display_rotation(Some(&screen)));
            let (sx, sy) = shown
                .enumerate_pixels()
                .find(|(.., pixel)| pixel.0[0] == 255)
                .map(|(x, y, _)| (x, y))
                .unwrap();
            let centre = |at: u32, extent: u32| (at as f32 + 0.5) / extent as f32;
            let (x, y) = raw_point(
                display_rotation(Some(&screen)),
                centre(sx, shown.width()),
                centre(sy, shown.height()),
            );
            assert!(
                (x - centre(raw.0, width)).abs() < 1e-4 && (y - centre(raw.1, height)).abs() < 1e-4,
                "{orientation:?}: ({x}, {y})"
            );
        }
    }

    #[test]
    fn rotations_remap_into_the_raw_portrait_framebuffer() {
        let screen = |orientation, width, height| DeviceScreenConfig {
            width,
            height,
            orientation,
            ..Default::default()
        };
        let left = screen(DeviceOrientation::LandscapeLeft, 588, 1280);
        let right = screen(DeviceOrientation::LandscapeRight, 588, 1280);
        let down = screen(DeviceOrientation::PortraitUpsideDown, 588, 1280);
        let up = screen(DeviceOrientation::Portrait, 588, 1280);
        let raw =
            |screen: Option<&DeviceScreenConfig>| raw_point(display_rotation(screen), 0.25, 0.75);
        assert_eq!(raw(Some(&left)), (0.75, 0.75));
        assert_eq!(raw(Some(&right)), (0.25, 0.25));
        assert_eq!(raw(Some(&down)), (0.75, 0.25));
        assert_eq!(raw(Some(&up)), (0.25, 0.75));
        assert_eq!(raw(None), (0.25, 0.75));
        let already_landscape = screen(DeviceOrientation::LandscapeLeft, 1280, 588);
        assert_eq!(raw(Some(&already_landscape)), (0.25, 0.75));
    }

    #[test]
    fn display_point_inverts_raw_point_in_every_orientation() {
        for (rotation, shown) in [
            (DeviceOrientation::Portrait, (0.1, 0.2)),
            (DeviceOrientation::LandscapeLeft, (0.8, 0.1)),
            (DeviceOrientation::LandscapeRight, (0.2, 0.9)),
            (DeviceOrientation::PortraitUpsideDown, (0.9, 0.8)),
        ] {
            let (x, y) = display_point(rotation, 0.1, 0.2);
            assert!(
                close(x, shown.0) && close(y, shown.1),
                "{rotation:?}: ({x}, {y})"
            );
            let (rx, ry) = raw_point(rotation, x, y);
            assert!(
                close(rx, 0.1) && close(ry, 0.2),
                "{rotation:?}: ({rx}, {ry})"
            );
        }
    }
}
