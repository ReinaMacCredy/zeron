use core_graphics::event::{CGEventFlags, CGEventType};

pub const ESCAPE_KEY_CODE: i64 = 0x35;
const HID_SYSTEM_STATE: i64 = 1;

pub const OBSERVED_EVENT_TYPES: [CGEventType; 6] = [
    CGEventType::KeyDown,
    CGEventType::FlagsChanged,
    CGEventType::LeftMouseDown,
    CGEventType::RightMouseDown,
    CGEventType::OtherMouseDown,
    CGEventType::ScrollWheel,
];

const DISQUALIFYING_FLAGS: CGEventFlags = CGEventFlags::CGEventFlagCommand
    .union(CGEventFlags::CGEventFlagAlternate)
    .union(CGEventFlags::CGEventFlagControl)
    .union(CGEventFlags::CGEventFlagShift);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventSource {
    pub process_id: i64,
    pub state_id: i64,
    pub user_data: i64,
}

impl EventSource {
    pub const HARDWARE: Self = Self {
        process_id: 0,
        state_id: HID_SYSTEM_STATE,
        user_data: 0,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classified {
    Escape,
    Physical { keyboard: bool },
    Ignored,
}

fn observed(event_type: u32) -> bool {
    OBSERVED_EVENT_TYPES
        .iter()
        .any(|observed| *observed as u32 == event_type)
}

pub fn is_physical_input(event_type: u32, source: EventSource, armed: bool) -> bool {
    armed && observed(event_type) && source == EventSource::HARDWARE
}

pub fn is_physical_escape(
    event_type: u32,
    key_code: i64,
    flags: CGEventFlags,
    source: EventSource,
    armed: bool,
) -> bool {
    is_physical_input(event_type, source, armed)
        && event_type == CGEventType::KeyDown as u32
        && key_code == ESCAPE_KEY_CODE
        && !flags.intersects(DISQUALIFYING_FLAGS)
}

pub fn classify(
    event_type: u32,
    key_code: i64,
    flags: CGEventFlags,
    source: EventSource,
    armed: bool,
) -> Classified {
    if is_physical_escape(event_type, key_code, flags, source, armed) {
        return Classified::Escape;
    }
    if !is_physical_input(event_type, source, armed) {
        return Classified::Ignored;
    }
    Classified::Physical {
        keyboard: event_type == CGEventType::KeyDown as u32
            || event_type == CGEventType::FlagsChanged as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NON_COALESCED: CGEventFlags = CGEventFlags::CGEventFlagNonCoalesced;
    const DRIVER_PID: i64 = 46_335;

    struct Escape {
        event_type: CGEventType,
        key_code: i64,
        flags: CGEventFlags,
        source: EventSource,
        armed: bool,
    }

    impl Default for Escape {
        fn default() -> Self {
            Self {
                event_type: CGEventType::KeyDown,
                key_code: ESCAPE_KEY_CODE,
                flags: NON_COALESCED,
                source: EventSource::HARDWARE,
                armed: true,
            }
        }
    }

    impl Escape {
        fn physical(self) -> bool {
            is_physical_escape(
                self.event_type as u32,
                self.key_code,
                self.flags,
                self.source,
                self.armed,
            )
        }
    }

    fn source(process_id: i64, state_id: i64, user_data: i64) -> EventSource {
        EventSource {
            process_id,
            state_id,
            user_data,
        }
    }

    #[test]
    fn a_hardware_escape_is_the_only_escape_that_stops_input() {
        assert!(Escape::default().physical(), "hardware escape");
        let driver_flags = CGEventFlags::from_bits_retain(0x2000_0100);
        let cases = [
            (
                "driver HID route (CGEventPost to HID tap)",
                Escape {
                    flags: driver_flags,
                    source: source(DRIVER_PID, HID_SYSTEM_STATE, 0),
                    ..Escape::default()
                },
            ),
            (
                "driver combined-session source",
                Escape {
                    flags: driver_flags,
                    source: source(DRIVER_PID, 0, 0),
                    ..Escape::default()
                },
            ),
            (
                "driver session-tap route",
                Escape {
                    source: source(DRIVER_PID, HID_SYSTEM_STATE, 0),
                    ..Escape::default()
                },
            ),
            (
                "poster-supplied user-data marker",
                Escape {
                    source: source(0, HID_SYSTEM_STATE, 0x5EED_BEEF),
                    ..Escape::default()
                },
            ),
            (
                "non-HID source state with a clean pid",
                Escape {
                    source: source(0, 0, 0),
                    ..Escape::default()
                },
            ),
            (
                "private source state with a clean pid",
                Escape {
                    source: source(0, -1, 0),
                    ..Escape::default()
                },
            ),
        ];
        for (label, case) in cases {
            assert!(!case.physical(), "{label}");
        }
    }

    #[test]
    fn chords_and_other_shapes_never_trip_the_kill_switch() {
        for (name, flag) in [
            ("command", CGEventFlags::CGEventFlagCommand),
            ("option", CGEventFlags::CGEventFlagAlternate),
            ("control", CGEventFlags::CGEventFlagControl),
            ("shift", CGEventFlags::CGEventFlagShift),
        ] {
            let chord = Escape {
                flags: NON_COALESCED | flag,
                ..Escape::default()
            };
            assert!(!chord.physical(), "hardware escape + {name}");
        }
        let caps_lock = Escape {
            flags: NON_COALESCED | CGEventFlags::CGEventFlagAlphaShift,
            ..Escape::default()
        };
        assert!(caps_lock.physical(), "caps lock does not change the key");
        let shapes = [
            (
                "disarmed hardware escape",
                Escape {
                    armed: false,
                    ..Escape::default()
                },
            ),
            (
                "hardware escape key up",
                Escape {
                    event_type: CGEventType::KeyUp,
                    ..Escape::default()
                },
            ),
            (
                "flags changed on escape",
                Escape {
                    event_type: CGEventType::FlagsChanged,
                    ..Escape::default()
                },
            ),
            (
                "another hardware key",
                Escape {
                    key_code: 0x24,
                    ..Escape::default()
                },
            ),
        ];
        for (label, case) in shapes {
            assert!(!case.physical(), "{label}");
        }
    }

    #[test]
    fn takeover_counts_only_physical_presses_clicks_and_scrolls() {
        for event_type in OBSERVED_EVENT_TYPES {
            let raw = event_type as u32;
            assert!(
                is_physical_input(raw, EventSource::HARDWARE, true),
                "physical takeover {raw}"
            );
            assert!(
                !is_physical_input(raw, source(99, HID_SYSTEM_STATE, 0), true),
                "synthetic takeover {raw}"
            );
            assert!(
                !is_physical_input(raw, EventSource::HARDWARE, false),
                "disarmed takeover {raw}"
            );
        }
        for event_type in [
            CGEventType::MouseMoved,
            CGEventType::LeftMouseDragged,
            CGEventType::KeyUp,
            CGEventType::LeftMouseUp,
        ] {
            let raw = event_type as u32;
            assert!(
                !is_physical_input(raw, EventSource::HARDWARE, true),
                "non-triggering input {raw}"
            );
        }
        assert!(
            !is_physical_input(CGEventType::KeyDown as u32, source(0, 0, 0), true),
            "combined session takeover"
        );
        assert!(
            !is_physical_input(
                CGEventType::LeftMouseDown as u32,
                source(0, HID_SYSTEM_STATE, 88),
                true
            ),
            "marked synthetic pointer"
        );
    }

    #[test]
    fn classification_routes_escape_keyboard_and_pointer() {
        let hardware = EventSource::HARDWARE;
        let key_down = CGEventType::KeyDown as u32;
        assert_eq!(
            classify(key_down, ESCAPE_KEY_CODE, NON_COALESCED, hardware, true),
            Classified::Escape
        );
        assert_eq!(
            classify(
                key_down,
                ESCAPE_KEY_CODE,
                NON_COALESCED | CGEventFlags::CGEventFlagCommand,
                hardware,
                true
            ),
            Classified::Physical { keyboard: true }
        );
        assert_eq!(
            classify(
                CGEventType::FlagsChanged as u32,
                0x38,
                CGEventFlags::empty(),
                hardware,
                true
            ),
            Classified::Physical { keyboard: true }
        );
        assert_eq!(
            classify(
                CGEventType::ScrollWheel as u32,
                0,
                CGEventFlags::empty(),
                hardware,
                true
            ),
            Classified::Physical { keyboard: false }
        );
        assert_eq!(
            classify(
                key_down,
                ESCAPE_KEY_CODE,
                NON_COALESCED,
                source(DRIVER_PID, HID_SYSTEM_STATE, 0),
                true
            ),
            Classified::Ignored
        );
    }
}
