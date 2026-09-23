use std::path::{Component, Path, PathBuf};

use core_foundation::base::TCFType;
use core_foundation::url::{CFURL, CFURLRef};
use core_graphics::access::ScreenCaptureAccess;
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSBundle, NSString};

use crate::computer::host::HostPermissions;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputerGrant {
    Accessibility,
    InputMonitoring,
    ScreenRecording,
}

pub const GRANT_SETUP_ORDER: [ComputerGrant; 3] = [
    ComputerGrant::Accessibility,
    ComputerGrant::InputMonitoring,
    ComputerGrant::ScreenRecording,
];

impl ComputerGrant {
    pub fn pane(self) -> &'static str {
        match self {
            Self::Accessibility => "accessibility",
            Self::InputMonitoring => "input-monitoring",
            Self::ScreenRecording => "screen-recording",
        }
    }

    pub fn settings_url(self) -> Option<&'static str> {
        match self {
            Self::Accessibility => crate::appshots::semantic_settings_url(),
            Self::InputMonitoring => crate::appshots::input_monitoring_settings_url(),
            Self::ScreenRecording => crate::appshots::capture_settings_url(),
        }
    }

    fn granted(self, permissions: &HostPermissions) -> bool {
        match self {
            Self::Accessibility => permissions.accessibility,
            Self::InputMonitoring => permissions.input_monitoring != Some(false),
            Self::ScreenRecording => permissions.screen_recording,
        }
    }
}

pub fn next_missing_grant(permissions: &HostPermissions) -> Option<ComputerGrant> {
    GRANT_SETUP_ORDER
        .into_iter()
        .find(|grant| !grant.granted(permissions))
}

unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn CGPreflightListenEventAccess() -> bool;
}

pub fn input_monitoring_granted() -> bool {
    unsafe { CGPreflightListenEventAccess() }
}

pub fn preflight_permissions() -> HostPermissions {
    HostPermissions {
        accessibility: unsafe { AXIsProcessTrusted() },
        screen_recording: ScreenCaptureAccess.preflight(),
        input_monitoring: Some(input_monitoring_granted()),
    }
}

const REGISTRATION_MESSAGE: &str = "macOS cannot locate this running app for permission setup. Move this app to Applications and reopen it before granting access.";
const IDENTITY_MISMATCH_MESSAGE: &str = "macOS is finding another copy of this app. Quit the other copies, then reopen the copy in Applications and try setup again.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistrationFailure {
    pub code: &'static str,
    pub message: &'static str,
}

impl std::fmt::Display for RegistrationFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message)
    }
}

fn standardized(path: &Path) -> PathBuf {
    if let Ok(resolved) = std::fs::canonicalize(path) {
        return resolved;
    }
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

pub fn verify_permission_setup_registration(
    app_path: &Path,
    bundle_id: Option<&str>,
    resolve: impl Fn(&str) -> Option<PathBuf>,
    mut register: impl FnMut(&Path) -> i32,
) -> Result<(), RegistrationFailure> {
    let Some(bundle_id) = bundle_id.filter(|id| !id.is_empty()) else {
        return Err(RegistrationFailure {
            code: "permission_setup_bundle_unavailable",
            message: REGISTRATION_MESSAGE,
        });
    };
    let exact = standardized(app_path);
    let matches = |resolved: Option<PathBuf>| resolved.is_some_and(|p| standardized(&p) == exact);
    if matches(resolve(bundle_id)) {
        return Ok(());
    }
    let _ = register(&exact);
    let Some(resolved) = resolve(bundle_id) else {
        return Err(RegistrationFailure {
            code: "permission_setup_registration_unresolved",
            message: REGISTRATION_MESSAGE,
        });
    };
    if !matches(Some(resolved)) {
        return Err(RegistrationFailure {
            code: "permission_setup_identity_mismatch",
            message: IDENTITY_MISMATCH_MESSAGE,
        });
    }
    Ok(())
}

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn LSRegisterURL(url: CFURLRef, update: u8) -> i32;
}

pub fn running_app_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.ancestors()
        .find(|dir| dir.extension().is_some_and(|ext| ext == "app"))
        .map(Path::to_path_buf)
}

pub fn running_app_bundle_identifier() -> Option<String> {
    bundle_identifier(&running_app_bundle()?)
}

fn bundle_identifier(app_path: &Path) -> Option<String> {
    let path = NSString::from_str(app_path.to_str()?);
    let bundle = NSBundle::bundleWithPath(&path)?;
    bundle.bundleIdentifier().map(|id| id.to_string())
}

fn resolve_application(bundle_id: &str) -> Option<PathBuf> {
    let workspace = NSWorkspace::sharedWorkspace();
    let url = workspace.URLForApplicationWithBundleIdentifier(&NSString::from_str(bundle_id))?;
    url.path().map(|path| PathBuf::from(path.to_string()))
}

fn register_application(app_path: &Path) -> i32 {
    let Some(url) = CFURL::from_path(app_path, true) else {
        return -50;
    };
    unsafe { LSRegisterURL(url.as_concrete_TypeRef(), 1) }
}

pub fn ensure_permission_setup_registration(app_path: &Path) -> Result<(), RegistrationFailure> {
    verify_permission_setup_registration(
        app_path,
        bundle_identifier(app_path).as_deref(),
        resolve_application,
        register_application,
    )
}

pub fn open_settings_pane(grant: ComputerGrant) -> Result<(), String> {
    let url = grant
        .settings_url()
        .ok_or_else(|| format!("No System Settings pane for {}.", grant.pane()))?;
    std::process::Command::new("/usr/bin/open")
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Could not open System Settings: {error}"))
}

pub fn start_permission_setup() -> Result<Option<ComputerGrant>, String> {
    let app = running_app_bundle().ok_or(REGISTRATION_MESSAGE)?;
    ensure_permission_setup_registration(&app).map_err(|failure| failure.message.to_string())?;
    let Some(grant) = next_missing_grant(&preflight_permissions()) else {
        return Ok(None);
    };
    open_settings_pane(grant)?;
    Ok(Some(grant))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn permissions(accessibility: bool, input: Option<bool>, screen: bool) -> HostPermissions {
        HostPermissions {
            accessibility,
            input_monitoring: input,
            screen_recording: screen,
        }
    }

    #[test]
    fn setup_opens_the_next_missing_pane_in_order() {
        assert_eq!(
            next_missing_grant(&permissions(false, Some(false), false)),
            Some(ComputerGrant::Accessibility)
        );
        assert_eq!(
            next_missing_grant(&permissions(true, Some(false), false)),
            Some(ComputerGrant::InputMonitoring)
        );
        assert_eq!(
            next_missing_grant(&permissions(true, Some(true), false)),
            Some(ComputerGrant::ScreenRecording)
        );
        assert_eq!(
            next_missing_grant(&permissions(false, Some(true), true)),
            Some(ComputerGrant::Accessibility)
        );
        assert_eq!(
            next_missing_grant(&permissions(true, Some(false), true)),
            Some(ComputerGrant::InputMonitoring)
        );
        assert_eq!(
            next_missing_grant(&permissions(true, None, false)),
            Some(ComputerGrant::ScreenRecording)
        );
        assert_eq!(
            next_missing_grant(&permissions(true, Some(true), true)),
            None
        );
        assert_eq!(
            GRANT_SETUP_ORDER.map(ComputerGrant::pane),
            ["accessibility", "input-monitoring", "screen-recording"]
        );
    }

    #[test]
    fn every_grant_has_a_settings_pane() {
        for grant in GRANT_SETUP_ORDER {
            let url = grant.settings_url().unwrap();
            assert!(url.starts_with("x-apple.systempreferences:"), "{url}");
        }
    }

    const APP: &str = "/Applications/Synara Test.app";
    const OTHER: &str = "/tmp/Synara Test.app";

    struct Case {
        name: &'static str,
        identifier: Option<&'static str>,
        initial: Option<&'static str>,
        after_registration: Option<&'static str>,
        registration_status: i32,
        expected_code: Option<&'static str>,
        expected_registrations: usize,
    }

    impl Default for Case {
        fn default() -> Self {
            Self {
                name: "",
                identifier: Some("test.permission.setup"),
                initial: None,
                after_registration: None,
                registration_status: 0,
                expected_code: None,
                expected_registrations: 0,
            }
        }
    }

    fn run(case: Case) {
        let app = Path::new(APP);
        let registrations = RefCell::new(Vec::<PathBuf>::new());
        let result = verify_permission_setup_registration(
            app,
            case.identifier,
            |_| {
                let chosen = if registrations.borrow().is_empty() {
                    case.initial
                } else {
                    case.after_registration
                };
                chosen.map(PathBuf::from)
            },
            |path| {
                registrations.borrow_mut().push(path.to_path_buf());
                case.registration_status
            },
        );
        let name = case.name;
        match (result, case.expected_code) {
            (Ok(()), None) => {}
            (Ok(()), Some(code)) => panic!("{name}: unexpected success, wanted {code}"),
            (Err(failure), expected) => {
                assert_eq!(Some(failure.code), expected, "{name}: unexpected failure");
                let recovery = if expected == Some("permission_setup_identity_mismatch") {
                    "Quit the other copies, then reopen the copy in Applications"
                } else {
                    "Move this app to Applications and reopen"
                };
                assert!(
                    failure.message.contains(recovery),
                    "{name}: no actionable recovery"
                );
                assert!(
                    !failure.message.contains("/Applications/")
                        && !failure.message.contains("/tmp/"),
                    "{name}: disclosed a local path"
                );
            }
        }
        let registrations = registrations.into_inner();
        assert_eq!(
            registrations.len(),
            case.expected_registrations,
            "{name}: registration count"
        );
        assert!(
            registrations.iter().all(|path| path == &standardized(app)),
            "{name}: registered another bundle"
        );
    }

    #[test]
    fn registration_resolves_this_exact_copy_or_fails_actionably() {
        run(Case {
            name: "already resolves",
            initial: Some(APP),
            ..Case::default()
        });
        run(Case {
            name: "registration fixes lookup",
            after_registration: Some(APP),
            expected_registrations: 1,
            ..Case::default()
        });
        run(Case {
            name: "success return does not prove lookup",
            expected_code: Some("permission_setup_registration_unresolved"),
            expected_registrations: 1,
            ..Case::default()
        });
        run(Case {
            name: "other copy is not this app",
            initial: Some(OTHER),
            after_registration: Some(OTHER),
            expected_code: Some("permission_setup_identity_mismatch"),
            expected_registrations: 1,
            ..Case::default()
        });
        run(Case {
            name: "registration selects exact copy",
            initial: Some(OTHER),
            after_registration: Some(APP),
            expected_registrations: 1,
            ..Case::default()
        });
        run(Case {
            name: "missing bundle identity",
            identifier: None,
            expected_code: Some("permission_setup_bundle_unavailable"),
            ..Case::default()
        });
        run(Case {
            name: "registration error",
            registration_status: -50,
            expected_code: Some("permission_setup_registration_unresolved"),
            expected_registrations: 1,
            ..Case::default()
        });
    }

    #[test]
    fn standardizes_symlinks_and_dot_segments() {
        assert_eq!(
            standardized(Path::new("/Applications/./X.app/../Y.app")),
            PathBuf::from("/Applications/Y.app")
        );
    }
}
