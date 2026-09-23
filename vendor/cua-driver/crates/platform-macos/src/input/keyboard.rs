//! Background keyboard synthesis via SLEventPostToPid (SkyLight SPI),
//! with fallback to the public CGEvent::post_to_pid.
//!
//! For keyboard events, `post_to_pid` attaches an `SLSEventAuthenticationMessage`
//! so Chromium-based apps accept synthetic keystrokes as trusted live input
//! (required on macOS 14+ for VS Code, Chrome, Electron apps).

use core_graphics::{
    event::{CGEvent, CGEventFlags},
    event_source::{CGEventSource, CGEventSourceStateID},
};
use foreign_types::ForeignType;

const SCREEN_SHARING_BUNDLE_ID: &str = "com.apple.ScreenSharing";
const SHIFT_KEY_CODE: u16 = 56;

fn is_screen_sharing_bundle_id(bundle_id: &str) -> bool {
    bundle_id == SCREEN_SHARING_BUNDLE_ID
}

/// Whether `pid` is Apple's Screen Sharing client.
///
/// Screen Sharing is an input forwarder rather than a text consumer: it relays
/// physical virtual-key transitions to the guest and ignores the Unicode string
/// attached to a synthetic keycode-0 event. Keep that special case explicit so
/// ordinary PID-routed text input retains its layout-independent Unicode path.
pub fn is_screen_sharing_pid(pid: i32) -> bool {
    crate::apps::bundle_id_for_pid(pid)
        .as_deref()
        .is_some_and(is_screen_sharing_bundle_id)
}

#[derive(Clone, Copy)]
enum KeyboardRoute {
    Pid { pid: i32, authenticated: bool },
    Hid,
}

impl KeyboardRoute {
    // Only used inside an admitted down or its already-owned release.
    fn post(self, event: &CGEvent) {
        match self {
            Self::Hid => event.post(core_graphics::event::CGEventTapLocation::HID),
            Self::Pid {
                pid,
                authenticated: true,
            } => post_keyboard_event(pid, event),
            Self::Pid {
                pid,
                authenticated: false,
            } => {
                if !super::skylight::post_to_pid(pid, event.as_ptr().cast(), false) {
                    event.post_to_pid(pid);
                }
            }
        }
    }
}

struct PreparedKey {
    down: CGEvent,
    up: CGEvent,
    route: KeyboardRoute,
    required_focus_window: Option<(u32, Option<usize>)>,
}

impl PreparedKey {
    fn new(
        source: &CGEventSource,
        code: u16,
        down_flags: CGEventFlags,
        up_flags: CGEventFlags,
        route: KeyboardRoute,
    ) -> anyhow::Result<Self> {
        let down = CGEvent::new_keyboard_event(source.clone(), code, true)
            .map_err(|_| anyhow::anyhow!("CGEvent keyboard down creation failed"))?;
        let up = CGEvent::new_keyboard_event(source.clone(), code, false)
            .map_err(|_| anyhow::anyhow!("CGEvent keyboard up creation failed"))?;
        down.set_flags(down_flags);
        up.set_flags(up_flags);
        Ok(Self {
            down,
            up,
            route,
            required_focus_window: None,
        })
    }

    fn bare(code: u16) -> anyhow::Result<Self> {
        Ok(Self {
            down: create_bare_keyboard_event(code, true)?,
            up: create_bare_keyboard_event(code, false)?,
            route: KeyboardRoute::Hid,
            required_focus_window: None,
        })
    }

    fn hold(self) -> anyhow::Result<super::cancellation::HeldInput<'static, impl FnOnce()>> {
        let Self {
            down,
            up,
            route,
            required_focus_window,
        } = self;
        if let (Some((window_id, element)), KeyboardRoute::Pid { pid, .. }) =
            (required_focus_window, route)
        {
            if !crate::ax::exact_target::keyboard_focus_matches_target(pid, window_id, element) {
                anyhow::bail!("same_pid_keyboard_ambiguity: application keyboard focus changed before key dispatch");
            }
        }
        super::cancellation::gate().press(|| route.post(&down), move || route.post(&up))
    }

    fn pulse(self) -> anyhow::Result<()> {
        let held = self.hold()?;
        key_pause(8)?;
        drop(held);
        Ok(())
    }
}

// Vec normally drops in forward order. Chords must unwind in reverse order.
struct HeldKeys<T>(Vec<T>);
impl<T> HeldKeys<T> {
    fn release(&mut self) -> anyhow::Result<()> {
        while let Some(key) = self.0.pop() {
            drop(key);
            key_pause(8)?;
        }
        Ok(())
    }
}
impl<T> Drop for HeldKeys<T> {
    fn drop(&mut self) {
        while let Some(key) = self.0.pop() {
            drop(key);
        }
    }
}

fn key_pause(ms: u64) -> anyhow::Result<()> {
    super::cancellation::gate().pause(std::time::Duration::from_millis(ms))
}

fn source() -> anyhow::Result<CGEventSource> {
    CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| anyhow::anyhow!("CGEventSource::new failed"))
}

fn press_key_routed(
    pid: i32,
    key: &str,
    modifiers: &[&str],
    authenticated: bool,
) -> anyhow::Result<()> {
    let (code, flags) = if key == "+" || key.eq_ignore_ascii_case("plus") {
        (
            key_name_to_code("=")?,
            modifier_flags(modifiers) | modifier_flags(&["shift"]),
        )
    } else {
        (key_name_to_code(key)?, modifier_flags(modifiers))
    };
    PreparedKey::new(
        &source()?,
        code,
        flags,
        flags,
        KeyboardRoute::Pid { pid, authenticated },
    )?
    .pulse()
}

/// Pair each admitted key down with an already-prepared release.
pub fn press_key(pid: i32, key: &str, modifiers: &[&str]) -> anyhow::Result<()> {
    press_key_routed(pid, key, modifiers, true)
}

/// Dispatch only while the application still names this exact window as its
/// keyboard destination. Matching releases remain unconditional if focus moves.
pub fn press_key_in_window(
    pid: i32,
    window_id: u32,
    element: Option<usize>,
    key: &str,
    modifiers: &[&str],
) -> anyhow::Result<()> {
    let (code, flags) = if key == "+" || key.eq_ignore_ascii_case("plus") {
        (
            key_name_to_code("=")?,
            modifier_flags(modifiers) | modifier_flags(&["shift"]),
        )
    } else {
        (key_name_to_code(key)?, modifier_flags(modifiers))
    };
    let mut pair = PreparedKey::new(
        &source()?,
        code,
        flags,
        flags,
        KeyboardRoute::Pid {
            pid,
            authenticated: true,
        },
    )?;
    pair.required_focus_window = Some((window_id, element));
    pair.pulse()
}

pub fn hotkey(pid: i32, key: &str, modifiers: &[&str]) -> anyhow::Result<()> {
    press_key(pid, key, modifiers)
}

pub fn hotkey_no_auth(pid: i32, key: &str, modifiers: &[&str]) -> anyhow::Result<()> {
    press_key_routed(pid, key, modifiers, false)
}

pub fn press_key_no_auth(pid: i32, key: &str, modifiers: &[&str]) -> anyhow::Result<()> {
    press_key_routed(pid, key, modifiers, false)
}

fn type_unicode(text: &str, inter_char_delay_ms: u64, route: KeyboardRoute) -> anyhow::Result<()> {
    let source = source()?;
    for ch in text.chars() {
        super::cancellation::gate().ensure_active()?;
        let pair = PreparedKey::new(
            &source,
            0,
            CGEventFlags::CGEventFlagNull,
            CGEventFlags::CGEventFlagNull,
            route,
        )?;
        let value = ch.to_string();
        pair.down.set_string(&value);
        pair.up.set_string(&value);
        pair.pulse()?;
        key_pause(inter_char_delay_ms)?;
    }
    Ok(())
}

pub fn type_text(pid: i32, text: &str) -> anyhow::Result<()> {
    type_text_with_delay(pid, text, 0)
}

pub fn type_text_with_delay(pid: i32, text: &str, inter_char_delay_ms: u64) -> anyhow::Result<()> {
    type_unicode(
        text,
        if inter_char_delay_ms == 0 {
            8
        } else {
            inter_char_delay_ms
        },
        KeyboardRoute::Pid {
            pid,
            authenticated: true,
        },
    )
}

fn with_modifier_keys<T>(
    modifiers: &[&str],
    route: KeyboardRoute,
    gesture: impl FnOnce(CGEventFlags) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let source = source()?;
    let mut flags = CGEventFlags::CGEventFlagNull;
    let mut codes = Vec::new();
    let mut held = HeldKeys(Vec::new());
    for modifier in modifiers {
        let Some((code, flag)) = modifier_key_code_and_flag(modifier) else {
            continue;
        };
        if codes.contains(&code) {
            continue;
        }
        let previous = flags;
        flags |= flag;
        held.0
            .push(PreparedKey::new(&source, code, flags, previous, route)?.hold()?);
        codes.push(code);
        key_pause(8)?;
    }
    // A modifier-free gesture must still obey cancellation admission.
    super::cancellation::gate().ensure_active()?;
    let result = gesture(flags);
    held.release()?;
    result
}

pub fn press_key_global(key: &str, modifiers: &[&str]) -> anyhow::Result<()> {
    let code = key_name_to_code(key)?;
    with_global_modifier_keys(modifiers, |flags| {
        PreparedKey::new(&source()?, code, flags, flags, KeyboardRoute::Hid)?.pulse()
    })
}

pub(super) fn with_global_modifier_keys<T>(
    modifiers: &[&str],
    gesture: impl FnOnce(CGEventFlags) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    with_modifier_keys(modifiers, KeyboardRoute::Hid, gesture)
}

fn modifier_key_code_and_flag(modifier: &str) -> Option<(u16, CGEventFlags)> {
    match modifier.to_lowercase().as_str() {
        "cmd" | "command" => Some((55, CGEventFlags::CGEventFlagCommand)),
        "shift" => Some((56, CGEventFlags::CGEventFlagShift)),
        "option" | "alt" => Some((58, CGEventFlags::CGEventFlagAlternate)),
        "ctrl" | "control" => Some((59, CGEventFlags::CGEventFlagControl)),
        "fn" => Some((63, CGEventFlags::CGEventFlagSecondaryFn)),
        _ => None,
    }
}

/// Hold PID-routed modifier keys around one pointer gesture and release them in
/// reverse order even when the gesture fails.
///
/// Mouse-event flag bits alone are not a sufficient physical-modifier model for
/// every AppKit host (Finder's collection views are a notable example). Pairing
/// the flagged mouse events with real modifier down/up transitions gives the
/// target the same ordered state it receives for a keyboard chord without
/// putting those transitions on the global HID queue.

pub(super) fn with_pid_modifier_keys<T>(
    pid: i32,
    modifiers: &[&str],
    gesture: impl FnOnce() -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    with_modifier_keys(
        modifiers,
        KeyboardRoute::Pid {
            pid,
            authenticated: true,
        },
        |_| gesture(),
    )
}

pub fn type_text_global(text: &str, inter_char_delay_ms: u64) -> anyhow::Result<()> {
    type_unicode(text, inter_char_delay_ms.max(8), KeyboardRoute::Hid)
}

fn press_bare_chord(code: u16, modifiers: &[u16]) -> anyhow::Result<()> {
    let pairs = bare_chord_transitions(code, modifiers)
        .into_iter()
        .filter(|(_, down)| *down)
        .map(|(code, _)| PreparedKey::bare(code))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let mut held = HeldKeys(Vec::new());
    for pair in pairs {
        held.0.push(pair.hold()?);
        key_pause(8)?;
    }
    held.release()
}

pub fn type_text_physical_global(text: &str, inter_char_delay_ms: u64) -> anyhow::Result<()> {
    // Validate the whole payload before sending the first physical chord.
    let groups = text
        .chars()
        .map(physical_text_events)
        .collect::<anyhow::Result<Vec<_>>>()?;
    for events in groups {
        let code = events
            .iter()
            .find(|event| event.event_type == PhysicalEventType::KeyDown)
            .ok_or_else(|| anyhow::anyhow!("physical text has no base key"))?
            .key_code;
        let modifiers = events
            .iter()
            .filter(|event| event.event_type == PhysicalEventType::FlagsChanged && event.key_down)
            .map(|event| event.key_code)
            .collect::<Vec<_>>();
        press_bare_chord(code, &modifiers)?;
        if inter_char_delay_ms > 8 {
            key_pause(inter_char_delay_ms - 8)?;
        }
    }
    Ok(())
}

pub fn press_key_bare_global(key: &str, modifiers: &[&str]) -> anyhow::Result<()> {
    let code = key_name_to_code(key)?;
    let mut codes = Vec::new();
    for modifier in modifiers {
        if let Some((code, _)) = modifier_key_code_and_flag(modifier) {
            if !codes.contains(&code) {
                codes.push(code);
            }
        }
    }
    press_bare_chord(code, &codes)
}

fn bare_chord_transitions(key_code: u16, modifier_codes: &[u16]) -> Vec<(u16, bool)> {
    let mut transitions = Vec::with_capacity(modifier_codes.len() * 2 + 2);
    transitions.extend(modifier_codes.iter().map(|&code| (code, true)));
    transitions.push((key_code, true));
    transitions.push((key_code, false));
    transitions.extend(modifier_codes.iter().rev().map(|&code| (code, false)));
    transitions
}

fn create_bare_keyboard_event(key_code: u16, key_down: bool) -> anyhow::Result<CGEvent> {
    unsafe {
        let event_ref = CGEventCreateKeyboardEvent(std::ptr::null_mut(), key_code, key_down);
        if event_ref.is_null() {
            anyhow::bail!("CGEventCreateKeyboardEvent with default source failed");
        }
        Ok(CGEvent::from_ptr(event_ref))
    }
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventCreateKeyboardEvent(
        source: core_graphics::sys::CGEventSourceRef,
        key_code: u16,
        key_down: bool,
    ) -> core_graphics::sys::CGEventRef;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PhysicalTextEvent {
    key_code: u16,
    key_down: bool,
    event_type: PhysicalEventType,
    shift: bool,
    text: Option<char>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PhysicalEventType {
    KeyDown,
    KeyUp,
    FlagsChanged,
}

fn physical_text_events(ch: char) -> anyhow::Result<Vec<PhysicalTextEvent>> {
    let (key_code, shift) = physical_key_for_char(ch).ok_or_else(|| {
        anyhow::anyhow!(
            "Screen Sharing physical text delivery does not support character U+{:04X}",
            ch as u32
        )
    })?;
    let mut events = Vec::with_capacity(if shift { 4 } else { 2 });
    if shift {
        events.push(PhysicalTextEvent {
            key_code: SHIFT_KEY_CODE,
            key_down: true,
            event_type: PhysicalEventType::FlagsChanged,
            shift: true,
            text: None,
        });
    }
    events.push(PhysicalTextEvent {
        key_code,
        key_down: true,
        event_type: PhysicalEventType::KeyDown,
        shift,
        text: Some(ch),
    });
    events.push(PhysicalTextEvent {
        key_code,
        key_down: false,
        event_type: PhysicalEventType::KeyUp,
        shift,
        text: Some(ch),
    });
    if shift {
        events.push(PhysicalTextEvent {
            key_code: SHIFT_KEY_CODE,
            key_down: false,
            event_type: PhysicalEventType::FlagsChanged,
            shift: false,
            text: None,
        });
    }
    Ok(events)
}

/// Map printable ASCII to the physical key that produces it on the standard US
/// layout. The Screen Sharing path intentionally sends only the returned
/// keycode and the required bare Shift transitions.
fn physical_key_for_char(ch: char) -> Option<(u16, bool)> {
    let lower = ch.to_ascii_lowercase();
    if ch.is_ascii_alphabetic() {
        return key_name_to_code(&lower.to_string())
            .ok()
            .map(|code| (code, ch.is_ascii_uppercase()));
    }
    if ch.is_ascii_digit() {
        return key_name_to_code(&ch.to_string())
            .ok()
            .map(|code| (code, false));
    }

    let (base, shift) = match ch {
        ' ' => ("space", false),
        '\t' => ("tab", false),
        '\n' | '\r' => ("return", false),
        '-' => ("-", false),
        '_' => ("-", true),
        '=' => ("=", false),
        '+' => ("=", true),
        '[' => ("[", false),
        '{' => ("[", true),
        ']' => ("]", false),
        '}' => ("]", true),
        '\\' => ("\\", false),
        '|' => ("\\", true),
        ';' => (";", false),
        ':' => (";", true),
        '\'' => ("'", false),
        '"' => ("'", true),
        ',' => (",", false),
        '<' => (",", true),
        '.' => (".", false),
        '>' => (".", true),
        '/' => ("/", false),
        '?' => ("/", true),
        '`' => ("`", false),
        '~' => ("`", true),
        '!' => ("1", true),
        '@' => ("2", true),
        '#' => ("3", true),
        '$' => ("4", true),
        '%' => ("5", true),
        '^' => ("6", true),
        '&' => ("7", true),
        '*' => ("8", true),
        '(' => ("9", true),
        ')' => ("0", true),
        _ => return None,
    };
    key_name_to_code(base).ok().map(|code| (code, shift))
}

/// Post a keyboard event to `pid` via SLEventPostToPid (with auth message for
/// Chromium/Electron support) or fall back to CGEvent::post_to_pid.
pub(super) fn post_keyboard_event(pid: i32, event: &CGEvent) {
    let event_ptr = event.as_ptr() as *mut std::ffi::c_void;
    // attachAuthMessage = true: required for Chromium keyboard on macOS 14+.
    if !crate::input::skylight::post_to_pid(pid as libc::pid_t, event_ptr, true) {
        event.post_to_pid(pid as libc::pid_t);
    }
}

fn modifier_flags(modifiers: &[&str]) -> CGEventFlags {
    let mut flags = CGEventFlags::CGEventFlagNull;
    for m in modifiers {
        match m.to_lowercase().as_str() {
            "cmd" | "command" => flags |= CGEventFlags::CGEventFlagCommand,
            "shift" => flags |= CGEventFlags::CGEventFlagShift,
            "option" | "alt" => flags |= CGEventFlags::CGEventFlagAlternate,
            "ctrl" | "control" => flags |= CGEventFlags::CGEventFlagControl,
            "fn" => flags |= CGEventFlags::CGEventFlagSecondaryFn,
            _ => {}
        }
    }
    flags
}

pub(super) fn key_name_to_code(key: &str) -> anyhow::Result<u16> {
    let code = match key.to_lowercase().as_str() {
        "return" | "enter" => 36,
        "tab" => 48,
        "space" => 49,
        "delete" | "backspace" => 51,
        "escape" | "esc" => 53,
        "command" | "cmd" => 55,
        "shift" => 56,
        "capslock" | "caps_lock" => 57,
        "option" | "alt" => 58,
        "control" | "ctrl" => 59,
        "fn" => 63,
        "home" => 115,
        "pageup" | "page_up" | "pgup" | "prior" => 116,
        "del" | "forward_delete" => 117,
        "end" => 119,
        "pagedown" | "page_down" | "pgdn" | "next" => 121,
        "left" | "left_arrow" => 123,
        "right" | "right_arrow" => 124,
        "down" | "down_arrow" => 125,
        "up" | "up_arrow" => 126,
        "f1" => 122,
        "f2" => 120,
        "f3" => 99,
        "f4" => 118,
        "f5" => 96,
        "f6" => 97,
        "f7" => 98,
        "f8" => 100,
        "f9" => 101,
        "f10" => 109,
        "f11" => 103,
        "f12" => 111,
        // Extended function row: real keycodes Apple reserves even though no
        // current Apple keyboard ships these keys.
        "f13" => 105,
        "f14" => 107,
        "f15" => 113,
        "f16" => 106,
        "f17" => 64,
        "f18" => 79,
        "f19" => 80,
        "f20" => 90,
        // PC editing-cluster and ANSI keypad keys (Apple kVK_* constants).
        // "menu" is the Application/Context-Menu key; "help" is what the Insert
        // key on a PC keyboard maps to — it is NOT Insert semantics, so
        // "insert" stays unmapped. Keypad navigation names (kp_home,
        // kp_insert, kp_delete, kp_begin, kp_tab, kp_space, kp_f1-f4,
        // kp_separator, kp_page_up/down, kp arrows) have no macOS keycode.
        "help" => 114,
        "menu" | "context_menu" | "application" => 110,
        "kp_decimal" | "keypad_decimal" => 65,
        "kp_multiply" | "keypad_multiply" => 67,
        "kp_add" | "kp_plus" | "keypad_plus" => 69,
        "kp_clear" | "keypad_clear" | "numlock" | "num_lock" => 71,
        "kp_divide" | "keypad_divide" => 75,
        "kp_enter" | "keypad_enter" => 76,
        "kp_subtract" | "kp_minus" | "keypad_minus" => 78,
        "kp_equals" | "kp_equal" | "keypad_equals" => 81,
        "kp_0" | "keypad_0" => 82,
        "kp_1" | "keypad_1" => 83,
        "kp_2" | "keypad_2" => 84,
        "kp_3" | "keypad_3" => 85,
        "kp_4" | "keypad_4" => 86,
        "kp_5" | "keypad_5" => 87,
        "kp_6" | "keypad_6" => 88,
        "kp_7" | "keypad_7" => 89,
        "kp_8" | "keypad_8" => 91,
        "kp_9" | "keypad_9" => 92,
        "a" => 0,
        "s" => 1,
        "d" => 2,
        "f" => 3,
        "h" => 4,
        "g" => 5,
        "z" => 6,
        "x" => 7,
        "c" => 8,
        "v" => 9,
        "b" => 11,
        "q" => 12,
        "w" => 13,
        "e" => 14,
        "r" => 15,
        "y" => 16,
        "t" => 17,
        "1" => 18,
        "2" => 19,
        "3" => 20,
        "4" => 21,
        "6" => 22,
        "5" => 23,
        "=" => 24,
        "9" => 25,
        "7" => 26,
        "-" => 27,
        "8" => 28,
        "0" => 29,
        "]" => 30,
        "o" => 31,
        "u" => 32,
        "[" => 33,
        "i" => 34,
        "p" => 35,
        "l" => 37,
        "j" => 38,
        "'" => 39,
        "k" => 40,
        ";" => 41,
        "\\" => 42,
        "," => 43,
        "/" => 44,
        "n" => 45,
        "m" => 46,
        "." => 47,
        "`" => 50,
        _ => anyhow::bail!("Unknown key name: {key}"),
    };
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_graphics::event::CGEventType;

    #[test]
    fn physical_text_uses_flags_changed_for_balanced_shift_transitions() {
        assert_eq!(
            physical_text_events('a').unwrap(),
            vec![
                PhysicalTextEvent {
                    key_code: 0,
                    key_down: true,
                    event_type: PhysicalEventType::KeyDown,
                    shift: false,
                    text: Some('a'),
                },
                PhysicalTextEvent {
                    key_code: 0,
                    key_down: false,
                    event_type: PhysicalEventType::KeyUp,
                    shift: false,
                    text: Some('a'),
                },
            ]
        );
        assert_eq!(
            physical_text_events('Z').unwrap(),
            vec![
                PhysicalTextEvent {
                    key_code: 56,
                    key_down: true,
                    event_type: PhysicalEventType::FlagsChanged,
                    shift: true,
                    text: None,
                },
                PhysicalTextEvent {
                    key_code: 6,
                    key_down: true,
                    event_type: PhysicalEventType::KeyDown,
                    shift: true,
                    text: Some('Z'),
                },
                PhysicalTextEvent {
                    key_code: 6,
                    key_down: false,
                    event_type: PhysicalEventType::KeyUp,
                    shift: true,
                    text: Some('Z'),
                },
                PhysicalTextEvent {
                    key_code: 56,
                    key_down: false,
                    event_type: PhysicalEventType::FlagsChanged,
                    shift: false,
                    text: None,
                },
            ]
        );
        assert_eq!(physical_key_for_char('1'), Some((18, false)));
        assert_eq!(physical_key_for_char('!'), Some((18, true)));
        assert_eq!(physical_key_for_char('/'), Some((44, false)));
        assert_eq!(physical_key_for_char('?'), Some((44, true)));
    }

    #[test]
    fn bare_modifier_events_derive_type_and_flags_from_default_source() {
        let shift_down = create_bare_keyboard_event(SHIFT_KEY_CODE, true).unwrap();
        assert_eq!(
            shift_down.get_type() as u32,
            CGEventType::FlagsChanged as u32
        );
        assert!(
            shift_down
                .get_flags()
                .contains(CGEventFlags::CGEventFlagShift),
            "bare Shift down must derive the active Shift flag"
        );
        assert_eq!(
            shift_down
                .get_integer_value_field(core_graphics::event::EventField::EVENT_SOURCE_STATE_ID),
            CGEventSourceStateID::CombinedSessionState as i64,
            "NULL source must resolve to the default combined-session state, not HID state"
        );

        let shift_up = create_bare_keyboard_event(SHIFT_KEY_CODE, false).unwrap();
        assert_eq!(shift_up.get_type() as u32, CGEventType::FlagsChanged as u32);
        assert!(
            !shift_up
                .get_flags()
                .contains(CGEventFlags::CGEventFlagShift),
            "bare Shift up must derive cleared Shift state"
        );

        let z_down = create_bare_keyboard_event(6, true).unwrap();
        assert_eq!(z_down.get_type() as u32, CGEventType::KeyDown as u32);
        assert_eq!(
            z_down
                .get_integer_value_field(core_graphics::event::EventField::KEYBOARD_EVENT_KEYCODE),
            6
        );
    }

    #[test]
    fn bare_command_chord_orders_modifier_base_and_reverse_release() {
        assert_eq!(
            bare_chord_transitions(9, &[55]),
            vec![(55, true), (9, true), (9, false), (55, false)]
        );
        let command_down = create_bare_keyboard_event(55, true).unwrap();
        assert_eq!(
            command_down.get_type() as u32,
            CGEventType::FlagsChanged as u32
        );
        assert!(
            command_down
                .get_flags()
                .contains(CGEventFlags::CGEventFlagCommand),
            "bare Command down must derive the active Command flag"
        );
    }

    #[test]
    fn physical_text_rejects_characters_without_a_lossless_keycode() {
        let error = physical_text_events('🙂').unwrap_err().to_string();
        assert!(error.contains("U+1F642"), "{error}");
    }

    #[test]
    fn physical_text_covers_printable_ascii_and_common_text_whitespace() {
        for byte in 0x20_u8..=0x7e {
            let ch = char::from(byte);
            assert!(
                physical_key_for_char(ch).is_some(),
                "missing physical key for ASCII {ch:?}"
            );
        }
        for ch in ['\t', '\n', '\r'] {
            assert!(
                physical_key_for_char(ch).is_some(),
                "missing physical key for whitespace {ch:?}"
            );
        }
    }

    #[test]
    fn screen_sharing_detection_is_exact_and_case_sensitive() {
        assert!(is_screen_sharing_bundle_id("com.apple.ScreenSharing"));
        assert!(!is_screen_sharing_bundle_id("com.apple.screensharing"));
        assert!(!is_screen_sharing_bundle_id("com.microsoft.rdc.macos"));
    }

    #[test]
    fn extended_keymap_names_resolve_to_macos_virtual_keycodes() {
        for (name, code) in [
            ("f13", 105_u16),
            ("f14", 107),
            ("f15", 113),
            ("f16", 106),
            ("f17", 64),
            ("f18", 79),
            ("f19", 80),
            ("f20", 90),
            ("help", 114),
            ("menu", 110),
            ("kp_decimal", 65),
            ("kp_multiply", 67),
            ("kp_add", 69),
            ("kp_clear", 71),
            ("kp_divide", 75),
            ("kp_enter", 76),
            ("kp_subtract", 78),
            ("kp_equals", 81),
            ("kp_0", 82),
            ("kp_5", 87),
            ("kp_9", 92),
            ("page_up", 116),
            ("caps_lock", 57),
        ] {
            assert_eq!(key_name_to_code(name).unwrap(), code, "{name}");
        }
    }
}
