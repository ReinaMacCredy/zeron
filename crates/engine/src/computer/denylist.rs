pub const COMPUTER_DENYLIST_REFUSED_CODE: &str = "computer_denylist_refused";

const COMPUTER_DENYLIST_BUNDLE_IDS: &[&str] = &[
    "com.1password.1password",
    "com.agilebits.onepassword",
    "com.apple.keychainaccess",
    "com.apple.passwords",
    "com.apple.systempreferences",
    "com.apple.securityagent",
    "com.apple.security.authorization",
    "com.lastpass.lastpass",
    "com.bitwarden.desktop",
];

const COMPUTER_DENYLIST_BUNDLE_PREFIXES: &[&str] = &["com.agilebits.onepassword", "com.dashlane."];

const COMPUTER_DENYLIST_APP_NAMES: &[&str] = &[
    "1password",
    "keychain access",
    "passwords",
    "system settings",
    "system preferences",
    "securityagent",
    "securityagenthelper",
    "bitwarden",
    "dashlane",
    "lastpass",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputerDenylistMatch {
    pub app: String,
    pub matched: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "{app} is on the computer-control denylist ({matched}): password managers and OS security surfaces are refused, with no override in this build."
)]
pub struct ComputerDenylistError {
    pub app: String,
    pub matched: String,
}

impl ComputerDenylistError {
    pub fn code(&self) -> &'static str {
        COMPUTER_DENYLIST_REFUSED_CODE
    }
}

impl From<ComputerDenylistMatch> for ComputerDenylistError {
    fn from(found: ComputerDenylistMatch) -> Self {
        Self {
            app: found.app,
            matched: found.matched,
        }
    }
}

fn normalize_computer_app_name(raw: &str) -> String {
    let mut name = raw.trim();
    if name.contains('/') {
        name = name
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or_default();
    }
    if name.len() >= 4
        && name.is_char_boundary(name.len() - 4)
        && name[name.len() - 4..].eq_ignore_ascii_case(".app")
    {
        name = &name[..name.len() - 4];
    }
    name.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn matches_computer_denylist_string(raw: &str) -> Option<String> {
    let name = normalize_computer_app_name(raw);
    for denied in COMPUTER_DENYLIST_APP_NAMES {
        if name == *denied
            || name
                .strip_prefix(denied)
                .is_some_and(|rest| rest.starts_with(' '))
        {
            return Some(format!("app name {denied}"));
        }
    }
    let bundle_id: String = name.chars().filter(|c| !c.is_whitespace()).collect();
    if COMPUTER_DENYLIST_BUNDLE_IDS.contains(&bundle_id.as_str()) {
        return Some(format!("bundle id {bundle_id}"));
    }
    COMPUTER_DENYLIST_BUNDLE_PREFIXES
        .iter()
        .find(|prefix| bundle_id.starts_with(*prefix))
        .map(|prefix| format!("bundle id {prefix}*"))
}

pub fn computer_denylist_match(
    name: Option<&str>,
    bundle_id: Option<&str>,
) -> Option<ComputerDenylistMatch> {
    if let Some(matched) = name.and_then(matches_computer_denylist_string) {
        return Some(ComputerDenylistMatch {
            app: name.unwrap_or("the target app").to_owned(),
            matched,
        });
    }
    let matched = bundle_id.and_then(matches_computer_denylist_string)?;
    Some(ComputerDenylistMatch {
        app: name.or(bundle_id).unwrap_or("the target app").to_owned(),
        matched,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn by_name(name: &str) -> Option<String> {
        computer_denylist_match(Some(name), None).map(|found| found.matched)
    }

    fn by_bundle(bundle_id: &str) -> Option<String> {
        computer_denylist_match(None, Some(bundle_id)).map(|found| found.matched)
    }

    #[test]
    fn matches_app_names_bundle_ids_prefixes_and_executable_paths() {
        assert_eq!(by_name("1Password").as_deref(), Some("app name 1password"));
        assert_eq!(
            by_name("1Password 8").as_deref(),
            Some("app name 1password")
        );
        assert_eq!(
            by_name("Keychain Access").as_deref(),
            Some("app name keychain access")
        );
        assert_eq!(
            by_name("System Settings").as_deref(),
            Some("app name system settings")
        );
        assert_eq!(
            by_name("SecurityAgent").as_deref(),
            Some("app name securityagent")
        );
        assert_eq!(
            by_bundle("com.1password.1password").as_deref(),
            Some("bundle id com.1password.1password")
        );
        assert_eq!(
            by_bundle("com.agilebits.onepassword7").as_deref(),
            Some("bundle id com.agilebits.onepassword*")
        );
        assert_eq!(
            by_bundle("com.dashlane.dashlanephonefinal").as_deref(),
            Some("bundle id com.dashlane.*")
        );
        assert_eq!(
            by_bundle("com.bitwarden.desktop").as_deref(),
            Some("bundle id com.bitwarden.desktop")
        );
        assert_eq!(
            by_name("/Applications/1Password.app/Contents/MacOS/1Password").as_deref(),
            Some("app name 1password")
        );
    }

    #[test]
    fn does_not_match_ordinary_apps_or_near_miss_spellings() {
        assert_eq!(by_name("org.kde.konsole"), None);
        assert_eq!(by_name("Finder"), None);
        assert_eq!(by_name("passwordsafe"), None);
        assert_eq!(by_bundle("com.example.passwordsafe2"), None);
        assert_eq!(computer_denylist_match(None, None), None);
    }

    #[test]
    fn refuses_anything_else_that_names_itself_a_password_manager_too() {
        assert_eq!(
            by_name("Passwords Manager Pro").as_deref(),
            Some("app name passwords")
        );
        assert_eq!(
            by_name("Bitwarden Lite").as_deref(),
            Some("app name bitwarden")
        );
    }

    #[test]
    fn names_the_refused_surface_and_code() {
        let found = computer_denylist_match(Some("Notes"), Some("com.bitwarden.desktop")).unwrap();
        assert_eq!(found.app, "Notes");
        let error = ComputerDenylistError::from(found);
        assert_eq!(error.code(), "computer_denylist_refused");
        assert!(error.to_string().starts_with(
            "Notes is on the computer-control denylist (bundle id com.bitwarden.desktop)"
        ));
        let bundle_only = computer_denylist_match(None, Some("com.bitwarden.desktop")).unwrap();
        assert_eq!(bundle_only.app, "com.bitwarden.desktop");
    }
}
