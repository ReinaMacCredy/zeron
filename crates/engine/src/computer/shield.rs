use std::collections::HashSet;

use super::call_context::env_flag_enabled;

pub const ZERON_CUA_MASKED_ACTIVATION: &str = "ZERON_CUA_MASKED_ACTIVATION";
pub const ZERON_CUA_MASKED_APPS: &str = "ZERON_CUA_MASKED_APPS";

pub fn cua_masked_activation_enabled(value: Option<&str>) -> bool {
    env_flag_enabled(value)
}

pub fn cua_masked_activation_opt_in(value: Option<&str>) -> HashSet<String> {
    value
        .unwrap_or_default()
        .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
        .map(|entry| entry.trim().to_lowercase())
        .filter(|entry| !entry.is_empty())
        .collect()
}

pub fn masked_activation_opted_in(opt_in: &HashSet<String>, bundle_id: Option<&str>) -> bool {
    bundle_id.is_some_and(|bundle_id| opt_in.contains(&bundle_id.trim().to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_off_by_default_and_off_for_unset_or_empty_values() {
        assert!(!cua_masked_activation_enabled(None));
        assert!(!cua_masked_activation_enabled(Some("")));
        assert!(!cua_masked_activation_enabled(Some("   ")));
    }

    #[test]
    fn accepts_the_affirmative_spellings() {
        for value in ["1", "true", "on", "yes", "TRUE", " Yes ", "ON"] {
            assert!(cua_masked_activation_enabled(Some(value)), "{value:?}");
        }
    }

    #[test]
    fn rejects_every_other_spelling_rather_than_guessing() {
        for value in [
            "0",
            "false",
            "off",
            "no",
            "enabled",
            "2",
            "tru",
            "yes please",
        ] {
            assert!(!cua_masked_activation_enabled(Some(value)), "{value:?}");
        }
    }

    #[test]
    fn opt_in_is_empty_by_default_and_for_a_blank_list() {
        assert!(cua_masked_activation_opt_in(None).is_empty());
        assert!(cua_masked_activation_opt_in(Some("")).is_empty());
        assert!(cua_masked_activation_opt_in(Some(" , ;  ")).is_empty());
    }

    #[test]
    fn opt_in_splits_on_commas_semicolons_and_whitespace_and_lowercases() {
        let mut ids: Vec<_> = cua_masked_activation_opt_in(Some(
            " com.example.Foo,COM.EXAMPLE.BAR ;org.kde.kcalc\tnet.example.Baz ",
        ))
        .into_iter()
        .collect();
        ids.sort();
        assert_eq!(
            ids,
            [
                "com.example.bar",
                "com.example.foo",
                "net.example.baz",
                "org.kde.kcalc"
            ]
        );
    }

    #[test]
    fn opt_in_deduplicates_repeated_ids() {
        assert_eq!(
            cua_masked_activation_opt_in(Some("com.example.foo,COM.EXAMPLE.FOO")).len(),
            1
        );
    }

    #[test]
    fn opted_in_matches_case_insensitively_and_ignores_surrounding_whitespace() {
        let opt_in = HashSet::from(["com.example.foo".to_owned()]);
        assert!(masked_activation_opted_in(&opt_in, Some("COM.EXAMPLE.FOO")));
        assert!(masked_activation_opted_in(
            &opt_in,
            Some(" com.example.foo ")
        ));
    }

    #[test]
    fn opted_in_does_not_match_an_absent_blank_or_different_bundle_id() {
        let opt_in = HashSet::from(["com.example.foo".to_owned()]);
        assert!(!masked_activation_opted_in(&opt_in, None));
        assert!(!masked_activation_opted_in(&opt_in, Some("")));
        assert!(!masked_activation_opted_in(
            &opt_in,
            Some("com.example.other")
        ));
        assert!(!masked_activation_opted_in(
            &HashSet::new(),
            Some("com.example.foo")
        ));
    }
}
