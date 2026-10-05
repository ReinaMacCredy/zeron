use std::path::{Path, PathBuf};

use zeron_proto::device_hub::{DevicePlatform, DevicePlatformAvailability};

fn on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain([PathBuf::from("/usr/bin")])
        .any(|dir| dir.join(name).is_file())
}

fn availability(platform: DevicePlatform, reason: Option<String>) -> DevicePlatformAvailability {
    DevicePlatformAvailability {
        platform,
        available: reason.is_none(),
        reason,
    }
}

pub(crate) fn ios() -> DevicePlatformAvailability {
    let reason = if !cfg!(target_os = "macos") {
        Some("iOS Simulators need macOS with Xcode.")
    } else {
        ios_reason(on_path("xcrun"), developer_dir(), || {
            std::process::Command::new("xcrun")
                .args(["--find", "simctl"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        })
    };
    availability(DevicePlatform::Ios, reason.map(str::to_string))
}

fn developer_dir() -> Option<PathBuf> {
    let out = std::process::Command::new("/usr/bin/xcode-select")
        .arg("-p")
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
}

fn ios_reason(
    xcrun: bool,
    developer_dir: Option<PathBuf>,
    simctl: impl FnOnce() -> bool,
) -> Option<&'static str> {
    if !xcrun || !developer_dir.is_some_and(|dir| dir.is_dir()) {
        Some("Xcode command line tools were not found.")
    } else if !simctl() {
        Some("iOS Simulators need macOS with Xcode.")
    } else {
        None
    }
}

pub(crate) fn android() -> DevicePlatformAvailability {
    let env = |key: &str| std::env::var(key).ok();
    let reason = android_reason(&android_sdk(&env, &|path| std::fs::canonicalize(path).ok()));
    availability(DevicePlatform::Android, reason)
}

pub(crate) fn detect() -> Vec<DevicePlatformAvailability> {
    vec![ios(), android()]
}

#[derive(Debug, Default, PartialEq)]
struct AndroidSdk {
    root: Option<PathBuf>,
    adb: bool,
    emulator: bool,
    avdmanager: bool,
    legacy_avdmanager: bool,
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn script(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.bat")
    } else {
        name.to_string()
    }
}

fn android_sdk(
    env: &dyn Fn(&str) -> Option<String>,
    resolve: &dyn Fn(&Path) -> Option<PathBuf>,
) -> AndroidSdk {
    let set = |key: &str| {
        env(key)
            .map(|value| value.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let home = PathBuf::from(
        env("HOME")
            .or_else(|| env("USERPROFILE"))
            .unwrap_or_default(),
    );
    let explicit = set("ANDROID_HOME").or_else(|| set("ANDROID_SDK_ROOT"));
    let mut candidates: Vec<PathBuf> = match &explicit {
        Some(root) => vec![PathBuf::from(root)],
        None => vec![
            home.join("Library").join("Android").join("sdk"),
            home.join("Android").join("Sdk"),
            env("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join("AppData").join("Local"))
                .join("Android")
                .join("Sdk"),
        ],
    };
    if explicit.is_none() {
        for dir in std::env::split_paths(&env("PATH").unwrap_or_default()) {
            if let Some(adb) = resolve(&dir.join(exe("adb")))
                && let Some(root) = adb.parent().and_then(Path::parent)
            {
                candidates.push(root.to_path_buf());
            }
        }
    }
    for root in candidates {
        let adb = root.join("platform-tools").join(exe("adb")).exists();
        let emulator = root.join("emulator").join(exe("emulator")).exists();
        if explicit.is_some() || adb || emulator {
            let avdmanager = root
                .join("cmdline-tools")
                .join("latest")
                .join("bin")
                .join(script("avdmanager"))
                .exists();
            let legacy_avdmanager = !avdmanager
                && root
                    .join("tools")
                    .join("bin")
                    .join(script("avdmanager"))
                    .exists();
            return AndroidSdk {
                root: Some(root),
                adb,
                emulator,
                avdmanager,
                legacy_avdmanager,
            };
        }
    }
    AndroidSdk::default()
}

fn android_reason(sdk: &AndroidSdk) -> Option<String> {
    let Some(root) = &sdk.root else {
        return Some(
            "Android SDK was not found. Install it with Android Studio or set ANDROID_HOME to your SDK directory."
                .into(),
        );
    };
    let root = root.display();
    if !sdk.adb {
        return Some(format!(
            "Android SDK Platform-Tools are missing from {root}. Install them in Android Studio's SDK Manager."
        ));
    }
    if !sdk.emulator {
        return Some(format!(
            "Android Emulator is missing from {root}. Install it in Android Studio's SDK Manager."
        ));
    }
    if sdk.legacy_avdmanager {
        return Some(format!(
            "The Android SDK command-line tools in {root} appear to be an older, unsupported version. Install Android SDK Command-line Tools (latest) in Android Studio's SDK Manager under SDK Tools."
        ));
    }
    (!sdk.avdmanager).then(|| {
        format!(
            "Android SDK Command-line Tools (latest) are missing from {root}. Install them in Android Studio's SDK Manager."
        )
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }

    fn reason(env: &[(&str, &Path)]) -> Option<String> {
        let env: Vec<(String, String)> = env
            .iter()
            .map(|(key, value)| (key.to_string(), value.display().to_string()))
            .collect();
        let lookup = move |key: &str| {
            env.iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
        };
        android_reason(&android_sdk(&lookup, &|path| {
            std::fs::canonicalize(path).ok()
        }))
    }

    #[test]
    fn simctl_is_probed_only_once_a_developer_dir_exists() {
        let dir = tempfile::tempdir().unwrap();
        let missing = || -> bool { panic!("xcrun must not run without a developer dir") };
        let not_found = Some("Xcode command line tools were not found.");
        assert_eq!(ios_reason(true, None, missing), not_found);
        assert_eq!(
            ios_reason(true, Some(dir.path().join("gone")), missing),
            not_found
        );
        assert_eq!(
            ios_reason(true, Some(dir.path().into()), || false),
            Some("iOS Simulators need macOS with Xcode.")
        );
        assert_eq!(ios_reason(true, Some(dir.path().into()), || true), None);
    }

    #[test]
    fn android_reasons_follow_the_sdk_root_rules() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let empty = dir.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(
            reason(&[("HOME", &home), ("PATH", &empty)])
                .unwrap()
                .starts_with("Android SDK was not found.")
        );

        let sdk = home.join("Library/Android/sdk");
        touch(&sdk.join("emulator/emulator"));
        let missing = |what: &str| {
            format!(
                "{what} {}. Install them in Android Studio's SDK Manager.",
                sdk.display()
            )
        };
        assert_eq!(
            reason(&[("HOME", &home), ("PATH", &empty)]),
            Some(missing("Android SDK Platform-Tools are missing from"))
        );
        touch(&sdk.join("platform-tools/adb"));
        touch(&sdk.join("tools/bin/avdmanager"));
        assert!(
            reason(&[("HOME", &home), ("PATH", &empty)])
                .unwrap()
                .contains("appear to be an older, unsupported version")
        );
        std::fs::remove_file(sdk.join("tools/bin/avdmanager")).unwrap();
        assert_eq!(
            reason(&[("HOME", &home), ("PATH", &empty)]),
            Some(missing(
                "Android SDK Command-line Tools (latest) are missing from"
            ))
        );
        touch(&sdk.join("cmdline-tools/latest/bin/avdmanager"));
        assert_eq!(reason(&[("HOME", &home), ("PATH", &empty)]), None);

        let explicit = dir.path().join("explicit");
        std::fs::create_dir_all(&explicit).unwrap();
        assert!(
            reason(&[("HOME", &home), ("ANDROID_HOME", &explicit)])
                .unwrap()
                .contains(&format!(
                    "Platform-Tools are missing from {}",
                    explicit.display()
                ))
        );

        let other_home = dir.path().join("nobody");
        let on_path = dir.path().join("path-sdk");
        touch(&on_path.join("platform-tools/adb"));
        let emulator_missing = reason(&[
            ("HOME", &other_home),
            ("PATH", &on_path.join("platform-tools")),
        ])
        .unwrap();
        assert!(
            emulator_missing.starts_with("Android Emulator is missing from")
                && emulator_missing.contains("path-sdk"),
            "{emulator_missing}"
        );
    }
}
