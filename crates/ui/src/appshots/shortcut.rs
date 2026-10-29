//! Shared user preference and live registration updates for desktop hotkeys.
use std::sync::{Mutex, OnceLock};

use futures::channel::mpsc;
use gpui::Keystroke;

/// The macOS hotkey is one of three double-modifier chords (both keys of one
/// modifier pressed together), watched by a passive event tap, or none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChordModifier {
    Command,
    Option,
    Shift,
}

impl ChordModifier {
    pub const ALL: [Self; 3] = [Self::Command, Self::Option, Self::Shift];

    /// The stored keymap spelling.
    pub fn combo(self) -> &'static str {
        match self {
            Self::Command => "command-command",
            Self::Option => "option-option",
            Self::Shift => "shift-shift",
        }
    }

    pub fn from_combo(combo: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|chord| chord.combo() == combo)
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Self::Command => "⌘",
            Self::Option => "⌥",
            Self::Shift => "⇧",
        }
    }

    /// "⌘ + ⌘", the menu and trigger label.
    pub fn label(self) -> String {
        format!("{} + {}", self.glyph(), self.glyph())
    }
}

/// The stored spelling for "no hotkey": captures then come only from the UI.
pub const NO_HOTKEY: &str = "none";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Shortcut {
    pub key: String,
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
    pub platform: bool,
    pub chord: Option<ChordModifier>,
}

impl Shortcut {
    pub fn parse(combo: &str) -> Result<Self, ()> {
        if cfg!(target_os = "macos") {
            if combo == NO_HOTKEY {
                return Ok(Self::none());
            }
            if let Some(chord) = ChordModifier::from_combo(combo) {
                return Ok(Self {
                    chord: Some(chord),
                    ..Self::none()
                });
            }
        }
        let stroke = Keystroke::parse(&crate::settings::platform_combo(combo)).map_err(|_| ())?;
        let modifiers = stroke.modifiers;
        let key = stroke.key;
        let supported = (key.len() == 1 && key.as_bytes()[0].is_ascii_alphanumeric())
            || matches!(
                key.as_str(),
                "space"
                    | "tab"
                    | "enter"
                    | "backspace"
                    | "delete"
                    | "insert"
                    | "up"
                    | "down"
                    | "left"
                    | "right"
                    | "home"
                    | "end"
                    | "pageup"
                    | "pagedown"
            )
            || key
                .strip_prefix('f')
                .and_then(|n| n.parse::<u8>().ok())
                .is_some_and(|n| (1..=20).contains(&n));
        if !supported
            || modifiers.function
            || !(modifiers.control || modifiers.alt || modifiers.platform)
        {
            return Err(());
        }
        Ok(Self {
            key,
            control: modifiers.control,
            alt: modifiers.alt,
            shift: modifiers.shift,
            platform: modifiers.platform,
            chord: None,
        })
    }

    fn none() -> Self {
        Self {
            key: String::new(),
            control: false,
            alt: false,
            shift: false,
            platform: false,
            chord: None,
        }
    }

    /// "None" in the hotkey menu: nothing to register.
    pub fn is_none(&self) -> bool {
        self.key.is_empty() && self.chord.is_none()
    }

    #[cfg(any(target_os = "linux", test))]
    pub fn portal_trigger(&self) -> String {
        let mut parts = Vec::new();
        if self.control {
            parts.push("CTRL".to_owned());
        }
        if self.alt {
            parts.push("ALT".to_owned());
        }
        if self.shift {
            parts.push("SHIFT".to_owned());
        }
        if self.platform {
            parts.push("LOGO".to_owned());
        }
        parts.push(match self.key.as_str() {
            "enter" => "Return".into(),
            "backspace" => "BackSpace".into(),
            "pageup" => "Page_Up".into(),
            "pagedown" => "Page_Down".into(),
            "space" => "space".into(),
            key if key.len() == 1 => key.into(),
            key => {
                let mut chars = key.chars();
                chars.next().unwrap().to_uppercase().collect::<String>() + chars.as_str()
            }
        });
        parts.join("+")
    }
}

struct Preferences {
    shortcut: Shortcut,
    quick_capture: Option<ChordModifier>,
    recording: bool,
    subscribers: Vec<mpsc::UnboundedSender<Option<Shortcut>>>,
}

fn preferences() -> &'static Mutex<Preferences> {
    static PREFERENCES: OnceLock<Mutex<Preferences>> = OnceLock::new();
    PREFERENCES.get_or_init(|| {
        Mutex::new(Preferences {
            shortcut: Shortcut::parse(crate::settings::ShortcutId::CaptureAppshot.default_combo())
                .unwrap(),
            quick_capture: ChordModifier::from_combo(crate::settings::default_quick_capture()),
            recording: false,
            subscribers: Vec::new(),
        })
    })
}

impl Preferences {
    fn active(&self) -> Option<Shortcut> {
        (super::enabled() && !self.recording && !self.shortcut.is_none())
            .then(|| self.shortcut.clone())
    }

    fn quick_capture(&self) -> Option<ChordModifier> {
        self.quick_capture
            .filter(|chord| self.active().and_then(|active| active.chord) != Some(*chord))
    }
}

pub fn chords_conflict(quick_capture: &str, appshot: &str) -> bool {
    ChordModifier::from_combo(quick_capture).is_some() && quick_capture == appshot
}

pub(crate) fn current() -> Option<Shortcut> {
    preferences().lock().unwrap().active()
}

#[cfg(target_os = "linux")]
pub(crate) fn subscribe() -> mpsc::UnboundedReceiver<Option<Shortcut>> {
    let (tx, rx) = mpsc::unbounded();
    let mut preferences = preferences().lock().unwrap();
    let _ = tx.unbounded_send(preferences.active());
    preferences.subscribers.push(tx);
    rx
}

pub(crate) fn current_quick_capture() -> Option<ChordModifier> {
    preferences().lock().unwrap().quick_capture()
}

fn update(mutate: impl FnOnce(&mut Preferences)) {
    let (active, quick_capture) = {
        let mut preferences = preferences().lock().unwrap();
        mutate(&mut preferences);
        let active = preferences.active();
        preferences
            .subscribers
            .retain(|tx| tx.unbounded_send(active.clone()).is_ok());
        (active, preferences.quick_capture())
    };
    #[cfg(target_os = "macos")]
    super::macos::refresh_global_shortcut(active, quick_capture);
    #[cfg(not(target_os = "macos"))]
    let _ = (active, quick_capture);
}

pub fn set_quick_capture(combo: &str) {
    let chord = ChordModifier::from_combo(combo).filter(|_| cfg!(target_os = "macos"));
    if preferences().lock().unwrap().quick_capture == chord {
        return;
    }
    update(|preferences| preferences.quick_capture = chord);
}

pub(super) fn enabled_changed() {
    update(|_| {});
}

/// The hotkey `combo` registers. Stored combos are US keys and validate as
/// such; the key then moves to the one the layout prints for it (AZERTY's ⌘1
/// key prints &), since that is how its keycode is looked up. An unusable
/// combo registers the default.
fn registered(combo: &str, equivalents: &crate::settings::KeyEquivalents) -> Shortcut {
    let mut shortcut = Shortcut::parse(combo).unwrap_or_else(|_| {
        Shortcut::parse(crate::settings::ShortcutId::CaptureAppshot.default_combo()).unwrap()
    });
    shortcut.key = crate::settings::localize_key(&shortcut.key, equivalents).into_owned();
    shortcut
}

pub fn set_shortcut(combo: &str, equivalents: &crate::settings::KeyEquivalents) {
    if !super::is_desktop() {
        return;
    }
    let shortcut = registered(combo, equivalents);
    if preferences().lock().unwrap().shortcut == shortcut {
        return;
    }
    update(|preferences| preferences.shortcut = shortcut);
}

pub fn set_recording(recording: bool) {
    if !super::is_desktop() || preferences().lock().unwrap().recording == recording {
        return;
    }
    update(|preferences| preferences.recording = recording);
}

pub fn validate_shortcut(combo: &str) -> Result<(), ()> {
    Shortcut::parse(combo).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn global_shortcuts_require_a_supported_modified_key() {
        for combo in ["a", "shift-a", "fn-a", "ctrl-mystery", "ctrl-f25"] {
            assert!(Shortcut::parse(combo).is_err(), "{combo}");
        }
        for combo in ["ctrl-alt-space", "mod-shift-k", "alt-f12", "ctrl-pageup"] {
            assert!(Shortcut::parse(combo).is_ok(), "{combo}");
        }
    }
    #[test]
    fn macos_hotkey_is_a_double_modifier_chord_or_none() {
        if cfg!(target_os = "macos") {
            for chord in ChordModifier::ALL {
                let parsed = Shortcut::parse(chord.combo()).unwrap();
                assert_eq!(parsed.chord, Some(chord));
                assert!(!parsed.is_none());
            }
            assert!(Shortcut::parse(NO_HOTKEY).unwrap().is_none());
            assert_eq!(ChordModifier::Command.label(), "⌘ + ⌘");
        } else {
            assert!(Shortcut::parse(NO_HOTKEY).is_err());
            assert!(Shortcut::parse("command-command").is_err());
        }
    }

    #[test]
    fn hotkeys_validate_us_keys_and_register_printed_ones() {
        let azerty = crate::settings::azerty_key_equivalents();
        // AZERTY prints its ⌘1 key as &, which validation alone refuses.
        assert!(Shortcut::parse("ctrl-alt-&").is_err());
        assert_eq!(registered("ctrl-alt-1", &azerty).key, "&");
        assert_eq!(registered("ctrl-alt-a", &azerty).key, "a");
        assert_eq!(registered("ctrl-alt-space", &azerty).key, "space");
        assert_eq!(registered("ctrl-alt-1", &Default::default()).key, "1");
    }
    #[test]
    fn portal_uses_xkb_key_names_and_selected_modifiers() {
        assert_eq!(
            Shortcut::parse("ctrl-alt-space").unwrap().portal_trigger(),
            "CTRL+ALT+space"
        );
        assert_eq!(
            Shortcut::parse("ctrl-shift-pageup")
                .unwrap()
                .portal_trigger(),
            "CTRL+SHIFT+Page_Up"
        );
    }
}

pub(crate) fn capture_allowed() -> bool {
    current().is_some()
}
