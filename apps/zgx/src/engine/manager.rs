//! Which engine this app works through, and keeping it running: the one
//! place that starts, restarts and refuses engines.
//!
//! **The beta** (the `beta` feature: this interface beside the Zeron app,
//! scripts/package-zgx-beta.sh) works through the Zeron app's engine: the one
//! serving `ZERON_IPC_PORT` or 27654, its data in `ZERON_DATA_DIR` or ~/.zeron
//! (crates/cli's paths). It only ever attaches: the Zeron app starts, restarts
//! and updates that engine, with its terminals and turns in view, so with
//! none answering this app waits. A staging engine (port 27871,
//! ~/.zeron-staging) or any other edge is refused.
//!
//! **The Zeron app itself** (`cutover`, src/identity.rs) works through the
//! same engine. A release build owns it, as Zeron.app does: it starts it
//! when none answers (`spawn_production`), restarts it to sync after a
//! sign-in, and restarts a stale one or on "Restart engine", whoever started
//! it ([`Config::owns_every_engine`]). A debug build only attaches, as the beta.
//!
//! **Development builds** work through a staging engine: the one at
//! `ZERON_ZGX_ENGINE_PORT` (27871) with its data in `ZERON_ZGX_ENGINE_DATA`
//! (~/.zeron-staging), scripts/staging-app.sh's. One already serving that port
//! is used as it is; otherwise this app starts `zeron-engine`
//! (`ZERON_ZGX_ENGINE_BIN`, or this checkout's `target/debug/zeron-engine`:
//! `cargo build -p zeron --bin zeron-engine` at the repository's root)
//! against the same edge and sign-in client as `src/auth.rs`'s, detached, so
//! it outlives the app as the shipping app's engine does.
//!
//!   ZERON_ZGX_ENGINE_PORT=<n>     the engine's IPC port (27871)
//!   ZERON_ZGX_ENGINE_DATA=<dir>   its data dir (~/.zeron-staging)
//!   ZERON_ZGX_ENGINE_BIN=<path>   the engine to start when none answers
//!   ZERON_ZGX_ENGINE_HOME=<dir>   a home of its own for the engine it starts
//!                                 (HOME, CLAUDE_CONFIG_DIR, CODEX_HOME), so
//!                                 the agents' logins it reads and writes are
//!                                 that dir's, not this Mac's: for trying
//!                                 sign-ins with throwaway state. It skips
//!                                 the periodic CLI update check, which
//!                                 finds no login keychain there
//!   ZERON_ZGX_ENGINE_SPAWN=0      never start one; only use one that runs
//!
//! A development build never uses the production engine by accident: ports
//! 27654 and 27655 (Zeron v3's on Windows), ~/.zeron (and the pre-0.2.0
//! ~/.comet-native) and, on Windows, %LOCALAPPDATA%\Zeron are refused unless
//! it's asked for by name:
//!
//!   ZERON_ZGX_ENGINE_PRODUCTION=1 the production engine is allowed (the real
//!                                 chats: for comparing against the shipping
//!                                 app; never sign out from it)
//!
//! A signed-out engine opens a local workspace and keeps it; signing in
//! takes a new one (the shipping app's sync switch). So once the engine's
//! session is signed in while its workspace is still local, it is stopped
//! and started again ([`needs_restart`]); a synced engine stops by itself on
//! sign-out and is started again, signed out.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The production engine's data dirs, relative to home: today's and the one
/// it was renamed from in 0.2.0.
const PRODUCTION_DATA: &[&str] = &[".zeron", ".comet-native"];

/// The ports the production engine serves: Zeron's, and the one Zeron v3's
/// Windows install serves beside it.
const PRODUCTION_PORTS: &[u16] = &[zeron_cli::DEFAULT_IPC_PORT, 27655];

/// The production engine's data dirs: [`PRODUCTION_DATA`] under home, and on
/// Windows where zeron_cli puts it there (%LOCALAPPDATA%\Zeron, which no
/// HOME moves).
fn production_dirs(var: &impl Fn(&str) -> Option<String>, home: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = PRODUCTION_DATA.iter().map(|dir| home.join(dir)).collect();
    if cfg!(windows) {
        let local = var("LOCALAPPDATA").map(PathBuf::from).or_else(|| var("USERPROFILE").map(|p| Path::new(&p).join("AppData").join("Local")));
        dirs.extend(local.map(|local| local.join("Zeron")));
    }
    dirs
}

const STAGING_PORT: u16 = 27871;
const STAGING_DATA: &str = ".zeron-staging";

const BETA: bool = cfg!(feature = "beta");

/// The soonest a second engine is started after the first.
const SPAWN_SPACING: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct Config {
    pub port: u16,
    pub data_dir: PathBuf,
    bin: Option<PathBuf>,
    home: Option<PathBuf>,
    spawn: bool,
    /// What the engine signs in against: `src/auth.rs`'s.
    edge: String,
    client: String,
    callback_port: u16,
    /// The Zeron app's engine (the beta, and the Zeron app itself): started,
    /// restarted and stopped from here only by a released Zeron app (`spawn`).
    managed: bool,
}

/// Why no engine can be used.
pub type Refusal = String;

/// The file in an engine's data dir holding the build of the engine this
/// app started there last (`zeron-engine --build-id`): an engine reporting
/// that build is one this app started.
const STARTED: &str = "zgx-started-build";

impl Config {
    /// The engine this app would start now, if it may start one.
    pub fn startable(&self) -> Option<&Path> {
        if self.spawn { self.bin.as_deref() } else { None }
    }

    /// Whether this is the Zeron app's engine, which follows its bundle (an
    /// update's restart stops it, so the new bundle's starts).
    pub fn follows_bundle(&self) -> bool {
        self.managed
    }

    /// Whether this app only attaches to its engine: the beta, and a debug
    /// build of the Zeron app. Restarting it (to sync, stale, or asked) is
    /// the Zeron app's, with its terminals and turns in view.
    pub fn attach_only(&self) -> bool {
        self.managed && !self.spawn
    }

    /// Whether every engine on this config's port is this app's own: the
    /// released Zeron app's, as Zeron.app's are, whoever started it last.
    pub fn owns_every_engine(&self) -> bool {
        self.managed && self.spawn
    }

    /// Whether an engine reporting `running` is stale against the binary's
    /// `on_disk` build, and whether it's this app's to restart.
    pub fn judge(&self, running: Option<&str>, on_disk: Option<&str>) -> (bool, bool) {
        let (stale, ours) = crate::engine_update::judge(running, on_disk, started_build(self).as_deref());
        (stale, ours || self.owns_every_engine())
    }

    /// Why "Restart engine" can't restart an engine reporting `running`, if it can't.
    pub fn restart_refusal(&self, running: Option<&str>) -> Option<&'static str> {
        if self.attach_only() {
            return Some("This engine is the Zeron app's: restart it from there.");
        }
        let ours = self.owns_every_engine() || (running.is_some() && running == started_build(self).as_deref());
        (self.startable().is_none() || !ours).then_some("This engine wasn't started by this app.")
    }
}

/// Which build `bin` is (`zeron-engine --build-id`), if it says; an engine
/// from before build ids doesn't.
pub fn build_of(bin: &Path) -> Option<String> {
    let out = std::process::Command::new(bin)
        .arg("--build-id")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let id = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (out.status.success() && !id.is_empty() && !id.contains(' ')).then_some(id)
}

/// The build of the engine this app last started with `config`'s data.
pub fn started_build(config: &Config) -> Option<String> {
    let id = std::fs::read_to_string(config.data_dir.join(STARTED)).ok()?;
    let id = id.trim();
    (!id.is_empty()).then(|| id.to_owned())
}

impl Config {
    /// From the environment, for the account's edge, sign-in client and
    /// callback port.
    pub fn from_env(edge: &str, client: &str, callback_port: u16) -> Result<Self, Refusal> {
        let var = |key: &str| std::env::var(key).ok().filter(|v: &String| !v.trim().is_empty());
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let exe = std::env::current_exe().ok();
        Self::from_vars(var, &home, exe.as_deref(), edge, client, callback_port)
    }

    fn from_vars(
        var: impl Fn(&str) -> Option<String>,
        home: &Path,
        exe: Option<&Path>,
        edge: &str,
        client: &str,
        callback_port: u16,
    ) -> Result<Self, Refusal> {
        if BETA {
            let starts = matches!(crate::identity::CURRENT.engine, crate::identity::Engine::Production { starts: true });
            Self::zeron_app(var, home, exe, edge, client, callback_port, starts)
        } else {
            Self::staging(var, home, edge, client, callback_port)
        }
    }

    /// The beta's and the Zeron app's: Zeron.app's engine, resolved as
    /// Zeron.app resolves it; started (and owned) only if `starts`.
    fn zeron_app(
        var: impl Fn(&str) -> Option<String>,
        home: &Path,
        exe: Option<&Path>,
        edge: &str,
        client: &str,
        callback_port: u16,
        starts: bool,
    ) -> Result<Self, Refusal> {
        let production = crate::auth::production();
        if (edge, client, callback_port) != (production.edge.as_str(), production.client.as_str(), production.port) {
            return Err(format!("{edge} isn't production's edge and sign-in"));
        }
        let port = zeron_cli::ipc_port_from(var("ZERON_IPC_PORT").as_deref());
        let data_dir = zeron_cli::paths::resolve_data_dir(|key| match key {
            "HOME" => Some(home.as_os_str().to_owned()),
            key => var(key).map(Into::into),
        });
        if port == STAGING_PORT {
            return Err(format!("port {STAGING_PORT} is the staging engine's"));
        }
        if same_dir(&data_dir, &home.join(STAGING_DATA)) {
            return Err(format!("~/{STAGING_DATA} is the staging engine's"));
        }
        Ok(Self {
            port,
            data_dir,
            bin: exe.and_then(zeron_launch::engine_executable),
            home: None,
            spawn: starts,
            edge: edge.into(),
            client: client.into(),
            callback_port,
            managed: true,
        })
    }

    /// A development build's: a staging engine, never production's.
    fn staging(
        var: impl Fn(&str) -> Option<String>,
        home: &Path,
        edge: &str,
        client: &str,
        callback_port: u16,
    ) -> Result<Self, Refusal> {
        let port = match var("ZERON_ZGX_ENGINE_PORT") {
            Some(raw) => raw
                .trim()
                .parse()
                .map_err(|_| format!("ZERON_ZGX_ENGINE_PORT `{raw}` isn't a port"))?,
            None => STAGING_PORT,
        };
        let data_dir = var("ZERON_ZGX_ENGINE_DATA")
            .map(|dir| expand(&dir, home))
            .unwrap_or_else(|| home.join(STAGING_DATA));
        if var("ZERON_ZGX_ENGINE_PRODUCTION").as_deref() != Some("1") {
            if PRODUCTION_PORTS.contains(&port) {
                return Err(format!("port {port} is the production engine's (ZERON_ZGX_ENGINE_PRODUCTION=1 to use it anyway)"));
            }
            if let Some(dir) = production_dirs(&var, home).into_iter().find(|dir| same_dir(&data_dir, dir)) {
                return Err(format!("{} is the production engine's (ZERON_ZGX_ENGINE_PRODUCTION=1 to use it anyway)", dir.display()));
            }
        }
        let bin = var("ZERON_ZGX_ENGINE_BIN").map(|p| expand(&p, home)).or_else(|| {
            // This checkout's: apps/zgx is two below the repository's root.
            let built = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/zeron-engine");
            built.exists().then_some(built)
        });
        Ok(Self {
            port,
            data_dir,
            bin,
            home: var("ZERON_ZGX_ENGINE_HOME").map(|dir| expand(&dir, home)),
            spawn: var("ZERON_ZGX_ENGINE_SPAWN").as_deref() != Some("0"),
            edge: edge.into(),
            client: client.into(),
            callback_port,
            managed: false,
        })
    }

    pub fn describe(&self) -> String {
        format!("port {}, data {}", self.port, self.data_dir.display())
    }
}

fn expand(path: &str, home: &Path) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(path),
    }
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canon(a) == canon(b)
}

/// Starts engines for `config`, no more often than [`SPAWN_SPACING`].
pub struct Spawner {
    last: Option<Instant>,
    said_missing: bool,
    said_held: bool,
}

impl Spawner {
    pub fn new() -> Self {
        Self { last: None, said_missing: false, said_held: false }
    }

    /// Whether the last look found another engine holding the data dir.
    pub fn held(&self) -> bool {
        self.said_held
    }

    /// Whether an engine this app started is likely still coming up: started
    /// less than [`SPAWN_SPACING`] ago, so it's dialled again soon rather
    /// than less and less often.
    pub fn starting(&self) -> bool {
        self.last.is_some_and(|at| at.elapsed() < SPAWN_SPACING)
    }

    /// Nothing answers on the port: start an engine, if this app may and
    /// it's been long enough since the last. Whether one was started. An
    /// engine that holds the data dir without serving yet is on its way in or
    /// out: none is started beside it, and one follows once it has let go.
    pub fn start_if_due(&mut self, config: &Config) -> bool {
        if !config.spawn || self.last.is_some_and(|at| at.elapsed() < SPAWN_SPACING) {
            return false;
        }
        let Some(bin) = &config.bin else {
            if !self.said_missing {
                self.said_missing = true;
                eprintln!(
                    "zeron-zgx: engine: nothing answers on {} and there's no engine to start: {}",
                    config.describe(),
                    "build one (`cargo build -p zeron --bin zeron-engine` at the repository's root) \
                     or set ZERON_ZGX_ENGINE_BIN"
                );
            }
            return false;
        };
        if zeron_launch::data_dir_held(&config.data_dir) {
            if !self.said_held {
                self.said_held = true;
                eprintln!("zeron-zgx: engine: another holds {} and isn't serving; waiting for it", config.data_dir.display());
            }
            return false;
        }
        self.said_held = false;
        self.last = Some(Instant::now());
        // Started on a thread of its own: creating the process waits for the
        // system to map (and Windows' antivirus to scan) the engine's
        // executable, a few hundred milliseconds the first time a new
        // version runs, which the window's first frame shouldn't wait for.
        let (config, bin) = (config.clone(), bin.clone());
        let started = std::thread::Builder::new().name("engine-start".into()).spawn(move || {
            let started = spawn(&config, &bin);
            match started {
                Ok(()) => eprintln!("zeron-zgx: engine: started {} ({})", bin.display(), config.describe()),
                Err(error) => eprintln!("zeron-zgx: engine: {} didn't start: {error}", bin.display()),
            }
        });
        if let Err(error) = started {
            eprintln!("zeron-zgx: engine: couldn't start a thread to start it: {error}");
            self.last = None;
            return false;
        }
        true
    }
}

/// A development engine, detached (its own process group, no inherited
/// pipes), so this app closing leaves it running; it logs to its data dir.
fn spawn(config: &Config, bin: &Path) -> std::io::Result<()> {
    if config.managed {
        return spawn_production(config);
    }
    std::fs::create_dir_all(&config.data_dir)?;
    // Which build it is, kept (an engine that reports it is this app's), as
    // its reaper asks below: off this thread, since asking runs the binary.
    // Until then none is kept, so a check meanwhile reads it as not ours.
    let started = config.data_dir.join(STARTED);
    let _ = std::fs::remove_file(&started);
    let mut command = std::process::Command::new(bin);
    command
        .env("ZERON_DATA_DIR", &config.data_dir)
        .env("ZERON_IPC_PORT", config.port.to_string())
        .env("ZERON_EDGE_URL", &config.edge)
        .env("ZERON_WORKOS_CLIENT_ID", &config.client)
        .env("ZERON_CALLBACK_PORT", config.callback_port.to_string())
        // Not the shipping app's: no app to open from a banner.
        .env_remove("ZERON_APP")
        // Nor a chat's (this app run from a Zeron terminal): an engine that
        // carries a chat's origin marker lends it to everything it runs, so
        // the shipping engine's preview scan would probe its agents'
        // sign-in callbacks (Gemini CLI's OAuth ends on the first stray
        // request).
        .env_remove("ZERON_ORIGIN")
        .env_remove("ZERON_EDGE_TOKEN")
        .env_remove("ZERON_IPC_TOKEN")
        .env_remove("ZERON_IPC_TOKEN_FILE")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Some(home) = &config.home {
        let (claude, codex) = (home.join(".claude"), home.join(".codex"));
        std::fs::create_dir_all(&claude)?;
        std::fs::create_dir_all(&codex)?;
        command
            .env("HOME", home)
            .env("CLAUDE_CONFIG_DIR", claude)
            .env("CODEX_HOME", codex)
            .env("ZERON_NO_HARNESS_UPDATE_CHECKS", "1");
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let child = command.spawn()?;
    // Its build kept, then it reaped when it exits, whenever that is.
    let bin = bin.to_path_buf();
    std::thread::spawn(move || {
        if let Some(id) = build_of(&bin) {
            let _ = std::fs::write(&started, id);
        }
        let mut child = child;
        let _ = child.wait();
    });
    Ok(())
}

/// The Zeron app's own engine (`cutover`, released), as Zeron.app starts it
/// (crates/launch, the bundle's `Zeron Engine`): production's edge and
/// sign-in, its port and data.
fn spawn_production(config: &Config) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let launch = zeron_launch::LaunchConfig {
        data_dir: config.data_dir.clone(),
        ipc_port: config.port,
        edge_url: config.edge.clone(),
        workos_client_id: Some(config.client.clone()),
        edge_token: None,
        org_id: None,
    };
    zeron_launch::reap(zeron_launch::launch(&launch, &exe)?);
    Ok(())
}

/// Whether the engine has to start again to follow its session: signed in
/// (or choosing an organization) while its workspace is still the local
/// one it opened signed out, unless this device keeps its chats local; or
/// synced while it now keeps them local.
pub fn needs_restart(scope: Option<&str>, signed_in: bool, keep_local: bool) -> bool {
    signed_in
        && match scope {
            Some("local") => !keep_local,
            Some("synced") => keep_local,
            _ => false,
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_of(vars: &[(&str, &str)], home: &Path, exe: Option<&Path>, edge: &str) -> Result<Config, Refusal> {
        let vars: Vec<(String, String)> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let prod = crate::auth::production();
        Config::from_vars(
            move |key| vars.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()),
            home,
            exe,
            edge,
            &prod.client,
            prod.port,
        )
    }

    #[test]
    fn a_signed_in_session_on_a_local_workspace_restarts() {
        assert!(needs_restart(Some("local"), true, false));
        assert!(!needs_restart(Some("local"), false, false));
        assert!(!needs_restart(Some("synced"), true, false));
        assert!(!needs_restart(Some("development"), true, false));
        assert!(!needs_restart(None, true, false));
    }

    #[test]
    fn keeping_chats_local_holds_a_signed_in_engine_local() {
        // Signed in and kept local: the local workspace stays.
        assert!(!needs_restart(Some("local"), true, true));
        // Kept local after it synced: back to the local workspace.
        assert!(needs_restart(Some("synced"), true, true));
        // Signed out, it changes nothing.
        assert!(!needs_restart(Some("local"), false, true));
        assert!(!needs_restart(Some("development"), true, true));
        assert!(!needs_restart(None, true, true));
    }

    /// A development build never touches the production engine: its port,
    /// today's data dir or the one it was renamed from.
    #[cfg(not(feature = "beta"))]
    #[test]
    fn a_development_build_refuses_the_production_engine() {
        let home = Path::new("/Users/someone");
        let edge = "https://staging.zeron.sh";
        assert!(config_of(&[("ZERON_ZGX_ENGINE_PORT", "27654")], home, None, edge).is_err());
        assert!(config_of(&[("ZERON_ZGX_ENGINE_DATA", "~/.zeron")], home, None, edge).is_err());
        assert!(config_of(&[("ZERON_ZGX_ENGINE_DATA", "~/.comet-native")], home, None, edge).is_err());
        assert!(config_of(&[("ZERON_ZGX_ENGINE_PORT", "27655")], home, None, edge).is_err());
        // Asked for by name, it's allowed.
        let production = config_of(&[("ZERON_ZGX_ENGINE_PORT", "27655"), ("ZERON_ZGX_ENGINE_DATA", "~/.zeron"), ("ZERON_ZGX_ENGINE_PRODUCTION", "1")], home, None, edge).unwrap();
        assert_eq!((production.port, production.data_dir.clone()), (27655, home.join(".zeron")));
        // Windows' own place for it, which no HOME moves.
        if cfg!(windows) {
            let local = [("LOCALAPPDATA", r"C:\Users\someone\AppData\Local"), ("ZERON_ZGX_ENGINE_DATA", r"C:\Users\someone\AppData\Local\Zeron")];
            assert!(config_of(&local, home, None, edge).is_err());
            let profile = [("USERPROFILE", r"C:\Users\someone"), ("ZERON_ZGX_ENGINE_DATA", r"C:\Users\someone\AppData\Local\Zeron")];
            assert!(config_of(&profile, home, None, edge).is_err());
        }
        let staging = config_of(&[], home, None, edge).unwrap();
        assert_eq!((staging.port, staging.data_dir.clone()), (STAGING_PORT, home.join(STAGING_DATA)));
        assert!(!staging.follows_bundle());
    }

    /// The beta manages Zeron.app's own engine: its port and data, the
    /// bundle's engine, production's edge and sign-in; never staging's.
    #[cfg(feature = "beta")]
    #[test]
    fn a_beta_build_attaches_to_the_zeron_app_s_engine() {
        let dir = std::env::temp_dir().join(format!("zgx-beta-engine-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let macos = dir.join("Zeron.app/Contents/MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        let exe = macos.join("zeron");
        std::fs::write(macos.join("Zeron Engine"), "").unwrap();
        let home = Path::new("/Users/someone");
        let prod = crate::auth::production();
        let beta = config_of(&[], home, Some(&exe), &prod.edge).unwrap();
        assert_eq!((beta.port, beta.data_dir.clone()), (27654, home.join(".zeron")));
        // (A debug build of the Zeron app itself never starts it either: src/identity.rs.)
        assert_eq!(beta.startable(), None, "never starts one: a restart or an update is the Zeron app's");
        assert_eq!((beta.edge.as_str(), beta.client.as_str(), beta.callback_port), (prod.edge.as_str(), prod.client.as_str(), 27641));
        assert!(beta.follows_bundle());
        // As Zeron.app: ZERON_DATA_DIR and ZERON_IPC_PORT move it; the
        // development variables don't.
        let moved = config_of(&[("ZERON_DATA_DIR", "/tmp/z"), ("ZERON_IPC_PORT", "27700"), ("ZERON_ZGX_ENGINE_PORT", "1")], home, Some(&exe), &prod.edge).unwrap();
        assert_eq!((moved.port, moved.data_dir), (27700, PathBuf::from("/tmp/z")));
        // Never staging's engine, nor another edge.
        assert!(config_of(&[("ZERON_IPC_PORT", "27871")], home, Some(&exe), &prod.edge).is_err());
        assert!(config_of(&[("ZERON_DATA_DIR", "/Users/someone/.zeron-staging")], home, Some(&exe), &prod.edge).is_err());
        assert!(config_of(&[], home, Some(&exe), "https://staging.zeron.sh").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The released Zeron app owns its engine as Zeron.app does: it restarts
    /// it to sync, when stale and when asked, whoever started it. The beta
    /// and a debug Zeron app only attach.
    #[test]
    fn the_released_zeron_app_owns_its_engine_and_the_rest_attach() {
        let dir = tempfile::tempdir().unwrap();
        let macos = dir.path().join("Zeron.app/Contents/MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        let exe = macos.join("zeron");
        let engine = macos.join(zeron_launch::ENGINE_EXECUTABLES[0]);
        std::fs::write(&engine, "").unwrap();
        let home = dir.path().join("home");
        let prod = crate::auth::production();
        let vars = |key: &str| (key == "ZERON_DATA_DIR").then(|| dir.path().join("data").display().to_string());
        let of = |starts: bool| Config::zeron_app(vars, &home, Some(&exe), &prod.edge, &prod.client, prod.port, starts).unwrap();
        let owner = of(true);
        assert!(owner.owns_every_engine() && !owner.attach_only() && owner.follows_bundle());
        assert_eq!(owner.startable(), Some(engine.as_path()));
        // A stale engine is its own to restart, whoever started it, and so is one on "Restart engine".
        assert_eq!(owner.judge(Some("old"), Some("new")), (true, true));
        assert_eq!(owner.judge(None, Some("new")), (true, true));
        assert_eq!(owner.judge(Some("new"), Some("new")), (false, true));
        assert_eq!(owner.restart_refusal(Some("someone-else-s")), None);
        // The beta's and a debug Zeron app's: attached to, never judged, restarted or started here.
        let attached = of(false);
        assert!(attached.attach_only() && !attached.owns_every_engine());
        assert_eq!(attached.startable(), None);
        assert_eq!(attached.judge(Some("old"), Some("new")), (true, false));
        assert!(attached.restart_refusal(Some("old")).unwrap().contains("the Zeron app's"));
        // A development build's: only the one it started.
        let staging = config_of(&[("ZERON_ZGX_ENGINE_DATA", &dir.path().join("staging").display().to_string()), ("ZERON_ZGX_ENGINE_BIN", "/bin/true")], &home, None, "https://staging.zeron.sh");
        if let Ok(staging) = staging {
            assert!(!staging.attach_only() && !staging.owns_every_engine());
            std::fs::create_dir_all(&staging.data_dir).unwrap();
            std::fs::write(staging.data_dir.join(STARTED), "mine").unwrap();
            assert_eq!(staging.restart_refusal(Some("mine")), None);
            assert!(staging.restart_refusal(Some("theirs")).is_some());
            assert_eq!(staging.judge(Some("theirs"), Some("new")), (true, false));
        }
    }

    /// An engine holding the data dir is on its way in or out: no second one
    /// is started beside it (it would only queue for the lock), and one is
    /// started as soon as it lets go.
    #[cfg(unix)]
    #[test]
    fn no_engine_starts_while_one_holds_the_data_dir() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("engine");
        std::fs::write(&bin, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let config = Config {
            port: 1,
            data_dir: dir.path().join("data"),
            bin: Some(bin),
            home: None,
            spawn: true,
            edge: String::new(),
            client: String::new(),
            callback_port: 0,
            managed: false,
        };
        std::fs::create_dir_all(&config.data_dir).unwrap();
        let lock = std::fs::File::create(config.data_dir.join("engine.lock")).unwrap();
        lock.try_lock().unwrap();
        let mut spawner = Spawner::new();
        assert!(!spawner.start_if_due(&config), "held: nothing started");
        lock.unlock().unwrap();
        assert!(spawner.start_if_due(&config), "let go: started at once");
    }

    /// A start returns at once (the process is made on a thread of its own),
    /// and the engine it started counts as coming up until a second could
    /// be started: one start, not two, however soon it's asked again.
    #[test]
    fn a_start_returns_at_once_and_is_coming_up_until_the_next_is_due() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            port: 1,
            data_dir: dir.path().join("data"),
            bin: Some(dir.path().join("no-such-engine")),
            home: None,
            spawn: true,
            edge: String::new(),
            client: String::new(),
            callback_port: 0,
            managed: false,
        };
        let mut spawner = Spawner::new();
        assert!(!spawner.starting());
        let asked = Instant::now();
        assert!(spawner.start_if_due(&config));
        assert!(asked.elapsed() < Duration::from_secs(1), "not waiting for the process");
        assert!(spawner.starting());
        assert!(!spawner.start_if_due(&config), "one start per SPAWN_SPACING");
        let never = Config { spawn: false, ..config };
        assert!(!Spawner::new().start_if_due(&never));
    }

    #[test]
    fn home_relative_paths_expand() {
        let home = Path::new("/Users/someone");
        assert_eq!(expand("~/.zeron-staging", home), home.join(".zeron-staging"));
        assert_eq!(expand("/tmp/x", home), PathBuf::from("/tmp/x"));
    }
}
