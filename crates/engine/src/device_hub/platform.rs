use zeron_proto::device_hub::{DevicePlatform, DevicePlatformAvailability};

fn on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain([std::path::PathBuf::from("/usr/bin")])
        .any(|dir| dir.join(name).is_file())
}

pub(crate) fn ios() -> DevicePlatformAvailability {
    let reason = if !cfg!(target_os = "macos") {
        Some("iOS Simulators need macOS with Xcode.")
    } else if !on_path("xcrun") {
        Some("Xcode command line tools were not found.")
    } else {
        None
    };
    DevicePlatformAvailability {
        platform: DevicePlatform::Ios,
        available: reason.is_none(),
        reason: reason.map(str::to_string),
    }
}
