pub const COMPUTER_NAMED_KEYS: &[&str] = &[
    "escape",
    "enter",
    "tab",
    "space",
    "backspace",
    "delete",
    "insert",
    "home",
    "end",
    "pageup",
    "pagedown",
    "arrowup",
    "arrowdown",
    "arrowleft",
    "arrowright",
    "f1",
    "f2",
    "f3",
    "f4",
    "f5",
    "f6",
    "f7",
    "f8",
    "f9",
    "f10",
    "f11",
    "f12",
];

pub fn is_computer_named_key(name: &str) -> bool {
    COMPUTER_NAMED_KEYS.contains(&name)
}

pub const COMPUTER_KEY_NAME_ALIASES: &[(&str, &str)] = &[
    ("esc", "escape"),
    ("return", "enter"),
    ("spacebar", "space"),
    ("del", "delete"),
    ("up", "arrowup"),
    ("down", "arrowdown"),
    ("left", "arrowleft"),
    ("right", "arrowright"),
];

pub fn computer_key_name_alias(name: &str) -> Option<&'static str> {
    COMPUTER_KEY_NAME_ALIASES
        .iter()
        .find(|(alias, _)| *alias == name)
        .map(|(_, key)| *key)
}

pub const COMPUTER_MODIFIER_KEY_NAMES: &[&str] = &[
    "shift", "ctrl", "control", "alt", "option", "meta", "super", "command", "capslock",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_alias_resolves_to_a_canonical_named_key() {
        for (alias, key) in COMPUTER_KEY_NAME_ALIASES {
            assert!(is_computer_named_key(key), "{alias} -> {key}");
            assert!(!is_computer_named_key(alias), "{alias}");
        }
        assert_eq!(computer_key_name_alias("esc"), Some("escape"));
        assert_eq!(computer_key_name_alias("escape"), None);
    }

    #[test]
    fn modifiers_stay_out_of_the_named_keys() {
        for modifier in COMPUTER_MODIFIER_KEY_NAMES {
            assert!(!is_computer_named_key(modifier), "{modifier}");
        }
    }
}
