//! Which app this build is: a development build (staging's engine, its own
//! folder), the beta beside the Zeron app (`beta`: Zeron.app's engine,
//! attached to, never started), or the Zeron app itself (`cutover`, which
//! takes `beta`'s production edge and engine and goes further: Zeron.app's
//! bundle id, name, `zeron://` and updates, the engine started when none
//! answers, and the other interface's settings and drafts carried over once,
//! src/migration.rs). scripts/package-zgx-cutover.sh packages it.
//!
//! A debug build of `cutover` never starts the production engine, never
//! updates itself and carries nothing over unless `ZERON_ZGX_MIGRATE=1`
//! asks: only a release one does each, as Zeron.app's.

/// The three builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Build {
    Development,
    Beta,
    Cutover,
}

/// The engine a build works through (src/engine/manager.rs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    /// Staging's (port 27871, ~/.zeron-staging), started when none answers.
    Staging,
    /// Production's (`ZERON_IPC_PORT` or 27654, `ZERON_DATA_DIR` or
    /// ~/.zeron), started when none answers only if `starts`.
    Production { starts: bool },
}

/// What the system and the app's own files know this build as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Identity {
    pub build: Build,
    /// The app's name (the bundle's `CFBundleName`).
    pub name: &'static str,
    /// The bundle's `CFBundleIdentifier`; none outside a bundle.
    pub bundle_id: Option<&'static str>,
    /// zgx-desktop's application id: `zeron://` delivery, and one instance
    /// at a time (Linux's D-Bus name; macOS's bundle).
    pub application_id: Option<&'static str>,
    /// `ui.persist`'s folder (zgx_script::app_data_dir).
    pub app_id: &'static str,
    /// The URL scheme the bundle claims.
    pub url_scheme: Option<&'static str>,
    pub engine: Engine,
    /// Whether the app updates itself through crates/update's channel for the
    /// app (src/about.rs), from the fork's feed.
    pub updates_itself: bool,
    /// Whether the other interface's `ui-settings.json` and drafts are
    /// carried in once (src/migration.rs). The beta carries the sidebar and
    /// the look on every launch instead (src/carried_settings.rs).
    pub migrates: bool,
}

pub const fn of(build: Build, release: bool) -> Identity {
    match build {
        Build::Development => Identity {
            build,
            name: "Zeron (zgx)",
            bundle_id: None,
            application_id: None,
            app_id: "zeron-zgx",
            url_scheme: None,
            engine: Engine::Staging,
            updates_itself: false,
            migrates: false,
        },
        Build::Beta => Identity {
            build,
            name: "Zeron Beta",
            bundle_id: Some("sh.zeron.app.zgx"),
            application_id: Some("sh.zeron.app.zgx"),
            app_id: "zeron-beta",
            url_scheme: None,
            engine: Engine::Production { starts: false },
            updates_itself: false,
            migrates: false,
        },
        Build::Cutover => Identity {
            build,
            name: "Zeron",
            bundle_id: Some("sh.zeron.app"),
            application_id: Some("sh.zeron.app"),
            app_id: "zeron",
            url_scheme: Some("zeron"),
            engine: Engine::Production { starts: release },
            updates_itself: release,
            migrates: release,
        },
    }
}

pub const BUILD: Build = if cfg!(feature = "cutover") {
    Build::Cutover
} else if cfg!(feature = "beta") {
    Build::Beta
} else {
    Build::Development
};

/// This build's.
pub const CURRENT: Identity = of(BUILD, !cfg!(debug_assertions));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_build_is_the_app_it_says() {
        let dev = of(Build::Development, true);
        assert_eq!((dev.app_id, dev.application_id, dev.url_scheme, dev.engine), ("zeron-zgx", None, None, Engine::Staging));
        assert!(!dev.updates_itself && !dev.migrates);
        // The beta, as scripts/package-zgx-beta.sh and src/main.rs had it.
        let beta = of(Build::Beta, true);
        assert_eq!((beta.app_id, beta.application_id, beta.bundle_id), ("zeron-beta", Some("sh.zeron.app.zgx"), Some("sh.zeron.app.zgx")));
        assert_eq!((beta.url_scheme, beta.engine), (None, Engine::Production { starts: false }));
        assert!(!beta.updates_itself && !beta.migrates);
        // The Zeron app: dist/macos/Info.plist's id and scheme.
        let zeron = of(Build::Cutover, true);
        assert_eq!((zeron.name, zeron.bundle_id, zeron.application_id), ("Zeron", Some("sh.zeron.app"), Some("sh.zeron.app")));
        assert_eq!((zeron.url_scheme, zeron.engine), (Some("zeron"), Engine::Production { starts: true }));
        assert!(zeron.updates_itself && zeron.migrates);
        // A debug build of it never starts production's engine, updates itself or migrates by itself.
        let debug = of(Build::Cutover, false);
        assert_eq!(debug.engine, Engine::Production { starts: false });
        assert!(!debug.updates_itself && !debug.migrates);
        assert_eq!((debug.bundle_id, debug.app_id), (zeron.bundle_id, zeron.app_id));
    }

    #[test]
    fn this_build_is_the_one_its_features_name() {
        let want = if cfg!(feature = "cutover") {
            Build::Cutover
        } else if cfg!(feature = "beta") {
            Build::Beta
        } else {
            Build::Development
        };
        assert_eq!(CURRENT.build, want);
        assert_eq!(CURRENT.engine, of(want, !cfg!(debug_assertions)).engine);
        // The bundle's own files say the same (the package scripts write them).
        let script = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/package-zgx-cutover.sh")).unwrap();
        let zeron = of(Build::Cutover, true);
        assert!(script.contains(&format!("BUNDLE_ID={}", zeron.bundle_id.unwrap())), "the script's bundle id");
        assert!(script.contains(&format!("APP_NAME=\"{}\"", zeron.name)), "the script's name");
        assert!(script.contains("--features cutover"));
        // Built apart from the Dev's release binary (scripts/run-zgx-dev.sh runs target/release/zeron-zgx).
        assert!(script.contains("ZGX_TARGET_DIR:-$ZGX/target/package-zgx-cutover}"), "the script's own target dir");
    }
}
