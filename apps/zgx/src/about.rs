//! Settings → About & updates (ui/screens/settings/about.luau): which Zeron
//! this is, and its updates.
//!
//! **The Zeron app** (a released `cutover` build, src/identity.rs) updates
//! itself, as Zeron.app does (crates/update `desktop`, shared with the GPUI
//! app's `app_update.rs`): its own checker reads the fork's feed, a found
//! release is offered and downloads when asked, "Restart to update" installs
//! it and relaunches. Only with `ZERON_AUTO_UPDATE` does one download by
//! itself and, nobody restarting for it, install on quit ([`install_on_quit`]).
//! "Get beta updates" chooses the channel; leaving beta installs Stable's
//! release even when it's older. The engine follows the new bundle by
//! itself once nothing runs.
//!
//! **A development build** has no bundle to update: the page shows the
//! engine's updater (the engine on this computer, which knows its version and the
//! newest release and updates an install it manages), as before.
//!
//!   shared `about`   { app_version, app_build, engine_version?, engine_build?,
//!                      latest?, available, checked_at?, error?,
//!                      self_updating, channel?, channels, channel_note?,
//!                      held?, switch, flow, flow_version?, flow_error?,
//!                      blocker? }
//!                    flow: "idle" | "downloading" | "ready" | "failed" |
//!                    "installed"
//!   about.check      the release feed asked now: { latest, available }
//!   about.apply      the app: download, or with one ready, restart onto it;
//!                    a development build: the engine updates itself.
//!                    { version }; rejects with unmanaged, current, offline,
//!                    held or failed
//!   about.channel(beta)  get beta updates or not; rejects with why not

use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::OnceLock,
};

use serde_json::{Value as Json, json};
use zeron_update::{Channel, InstallKind, UpdateBlocker, UpdateStatus, Updater};
use zgx_desktop::{WindowContext, WindowFactory};
use zgx_script::{ScriptHost, Shared, Value};

use crate::engine::{self, rpc::Watch};

/// This app's version: a beta build's own (`0.2.136-beta.1`), else Zeron's.
pub const APP_VERSION: &str = env!("ZERON_APP_VERSION");

#[derive(Default)]
struct About {
    /// The engine's `UpdateStatus`, as last heard.
    status: RefCell<Json>,
    /// The engine's build (`EngineInfo.build`).
    build: RefCell<Option<String>>,
    /// The feed's newest version and when it was asked here (epoch ms).
    checked: RefCell<Option<(String, i64)>>,
    shared: RefCell<Vec<Shared>>,
    watch: RefCell<Option<Watch>>,
    /// The app's own updates, in an app install.
    desktop: RefCell<Option<Rc<Desktop>>>,
    /// To quit for "Restart to update".
    windows: RefCell<Option<WindowFactory>>,
}

/// How long a restart to update waits on the engine going with it.
const ENGINE_STOP_PATIENCE: std::time::Duration = std::time::Duration::from_secs(8);

thread_local! {
    static ABOUT: Rc<About> = Rc::default();
}

/// The runtime crates/update's async work runs on (its checker, downloads):
/// one worker, apart from the interface's thread.
fn tokio() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("zeron-update")
            .enable_all()
            .build()
            .expect("the update runtime")
    })
}

/// `work` on the update runtime, its answer awaited on this thread (None
/// only if it panicked).
async fn on_tokio<T: Send + 'static>(work: impl std::future::Future<Output = T> + Send + 'static) -> Option<T> {
    let (tx, mut rx) = zgx::task::latest();
    tokio().spawn(async move {
        tx.send(work.await);
    });
    rx.recv().await
}

/// The app's download and install, as Zeron.app's (crates/ui app_update.rs).
#[derive(Debug, Clone, PartialEq)]
enum Flow {
    Idle,
    Downloading { version: String },
    Ready { version: String, staged: PathBuf },
    Failed { version: String, message: String },
    Installed,
}

struct Desktop {
    install: InstallKind,
    blocker: Option<UpdateBlocker>,
    /// Background download and install on quit: the Zeron app's alone, as `automatic` decides.
    automatic: bool,
    edge: String,
    data_dir: PathBuf,
    checker: Updater,
    status: RefCell<Option<UpdateStatus>>,
    flow: RefCell<Flow>,
    /// Why "Get beta updates" couldn't turn on, after a try.
    note: RefCell<Option<String>>,
}

impl Desktop {
    /// The app's own updater, in an install that updates itself (a macOS
    /// app bundle); none in a development build.
    fn start() -> Option<Rc<Self>> {
        // The beta: the fork's feed carries the other interface, which would install
        // over this bundle, and a beta's channel claim would land in the shared data dir.
        let Some(feed) = app_feed(crate::identity::CURRENT) else {
            return None;
        };
        let install = zeron_update::detect_install();
        if !install.supports_desktop_update() {
            return None;
        }
        let data_dir = engine::data_dir()?;
        zeron_update::set_app_version(APP_VERSION);
        zeron_update::channel::set_data_dir(&data_dir);
        // A beta's first launch puts this computer on the beta channel (once per
        // build: a later choice of Stable in it sticks).
        if zeron_update::Channel::of_version(APP_VERSION) == Channel::Beta {
            match zeron_update::channel::claim_for_beta_build(&data_dir, APP_VERSION) {
                Ok(true) => eprintln!("zeron-zgx: updates: this computer now gets beta updates"),
                Ok(false) => {}
                Err(error) => eprintln!("zeron-zgx: updates: couldn't claim the beta channel: {error}"),
            }
        }
        let edge = feed.unwrap_or_else(|| crate::auth::endpoint().edge);
        let checker = {
            let _runtime = tokio().enter();
            Updater::spawn_desktop(edge.clone())
        };
        let blocker = install.desktop_update_blocker();
        Some(Rc::new(Self {
            install,
            blocker,
            automatic: automatic(crate::identity::CURRENT, zeron_update::desktop_auto_update_override()),
            edge,
            data_dir,
            checker,
            status: RefCell::default(),
            flow: RefCell::new(Flow::Idle),
            note: RefCell::default(),
        }))
    }

    fn self_updating(&self) -> bool {
        self.blocker.is_none()
    }

    /// The release the last check offers, if it installs here.
    fn available(&self) -> Option<String> {
        let status = self.status.borrow();
        let status = status.as_ref()?;
        let latest = status.update_available.then(|| status.latest_version.clone()).flatten()?;
        // Another device updated this Mac under this window: what's on offer
        // is on disk already, and only wants reopening on (`moved_on_to`).
        if let InstallKind::MacApp { bundle } = &self.install
            && let Some(on_disk) = zeron_update::bundle_app_version(bundle)
            && already_there(&on_disk, &latest, APP_VERSION)
        {
            return None;
        }
        Some(latest)
    }

    fn status_heard(self: &Rc<Self>, status: UpdateStatus) {
        let succeeded = status.error.is_none();
        *self.status.borrow_mut() = Some(status);
        if succeeded && self.automatic {
            self.download_if_needed();
        }
        publish();
    }

    /// Start the background download unless the offer is downloading or
    /// staged already.
    fn download_if_needed(self: &Rc<Self>) {
        if !self.self_updating() {
            return;
        }
        let Some(latest) = self.available() else { return };
        match &*self.flow.borrow() {
            Flow::Downloading { .. } | Flow::Installed => return,
            Flow::Ready { version, .. } if *version == latest => return,
            _ => {}
        }
        self.start_download();
    }

    fn start_download(self: &Rc<Self>) {
        if !self.self_updating() || matches!(*self.flow.borrow(), Flow::Downloading { .. }) {
            return;
        }
        let Some(version) = self.available() else { return };
        *self.flow.borrow_mut() = Flow::Downloading { version: version.clone() };
        publish();
        let (edge, data_dir, install) = (self.edge.clone(), self.data_dir.clone(), self.install.clone());
        let me = self.clone();
        engine::get().spawn(async move {
            let staged = on_tokio(async move {
                zeron_update::desktop::stage_offer(&edge, &data_dir, &install)
                    .await
                    .map_err(|err| format!("{err:#}"))
            })
            .await
            .unwrap_or_else(|| Err("the download stopped".into()));
            *me.flow.borrow_mut() = match staged {
                Ok((version, staged)) => {
                    eprintln!("zeron-zgx: updates: {version} staged at {}", staged.display());
                    Flow::Ready { version, staged }
                }
                Err(message) => {
                    eprintln!("zeron-zgx: updates: the download failed: {message}");
                    Flow::Failed { version, message }
                }
            };
            publish();
        });
    }

    /// Install what's staged now and relaunch onto it; the caller quits.
    fn install_for_restart(&self) -> Result<String, String> {
        let Flow::Ready { version, staged } = self.flow.borrow().clone() else {
            return Err("nothing is ready to install".into());
        };
        match self.install.apply_desktop(&staged, true) {
            Ok(()) => {
                *self.flow.borrow_mut() = Flow::Installed;
                Ok(version)
            }
            Err(error) => {
                let message = format!("{error:#}");
                *self.flow.borrow_mut() = Flow::Failed { version, message: message.clone() };
                Err(message)
            }
        }
    }

    fn install_on_quit(&self) {
        if !self.automatic {
            return;
        }
        let Flow::Ready { version, staged } = self.flow.borrow().clone() else { return };
        match self.install.apply_desktop(&staged, false) {
            Ok(()) => {
                eprintln!("zeron-zgx: updates: installed {version} on quit");
                *self.flow.borrow_mut() = Flow::Installed;
            }
            Err(error) => eprintln!("zeron-zgx: updates: installing {version} on quit failed: {error:#}"),
        }
    }

    fn fields(&self, fields: &mut Vec<(&'static str, Value)>) {
        let status = self.status.borrow();
        let status = status.as_ref();
        let channel = zeron_update::channel::read(&self.data_dir).channel;
        fields.push(("self_updating", Value::Bool(self.self_updating())));
        fields.push(("channels", Value::Bool(matches!(self.install, InstallKind::MacApp { .. }))));
        fields.push(("channel", Value::from(channel.as_str())));
        fields.push(("beta_minimum", Value::from(zeron_update::channel::BETA_MINIMUM_MACOS)));
        if let Some(note) = self.note.borrow().as_deref() {
            fields.push(("channel_note", Value::from(note)));
        }
        if let Some(blocker) = &self.blocker {
            fields.push(("blocker", Value::from(blocker.to_string().as_str())));
        }
        if let Some(held) = status.and_then(|s| s.held.as_deref()) {
            fields.push(("held", Value::from(held)));
        }
        fields.push(("switch", Value::Bool(status.is_some_and(|s| s.switch && s.update_available))));
        let (flow, version, error) = match &*self.flow.borrow() {
            Flow::Idle => ("idle", None, None),
            Flow::Downloading { version } => ("downloading", Some(version.clone()), None),
            Flow::Ready { version, .. } => ("ready", Some(version.clone()), None),
            Flow::Failed { version, message } => ("failed", Some(version.clone()), Some(message.clone())),
            Flow::Installed => ("installed", None, None),
        };
        fields.push(("flow", Value::from(flow)));
        if let Some(v) = version {
            fields.push(("flow_version", Value::from(v.as_str())));
        }
        if let Some(e) = error {
            fields.push(("flow_error", Value::from(e.as_str())));
        }
    }
}

/// The quit hook (src/main.rs, once the app's run ends): a staged update
/// nobody restarted for installs now, so the next launch is current.
pub fn install_on_quit() {
    if let Some(desktop) = ABOUT.with(|a| a.desktop.borrow().clone()) {
        desktop.install_on_quit();
    }
}

fn publish() {
    ABOUT.with(|a| a.publish());
}

pub fn install(cx: &mut WindowContext, host: &ScriptHost) {
    engine::install(cx);
    let about = ABOUT.with(Rc::clone);
    *about.windows.borrow_mut() = Some(cx.windows.clone());
    // (The app's updater first: the window's first `about` says whether the
    // app updates itself.)
    if about.shared.borrow().is_empty() {
        start(&about);
    }
    about.shared.borrow_mut().push(host.shared("about", about.value()));

    let a = about.clone();
    host.async_action("about.check", move |_| {
        let a = a.clone();
        async move {
            let desktop = a.desktop.borrow().clone();
            if let Some(desktop) = desktop {
                let checker = desktop.checker.clone();
                let status = on_tokio(async move { checker.check().await.map_err(|err| engine::outcome(&format!("{err:#}"))) })
                    .await
                    .ok_or("failed")??;
                let latest = status.latest_version.clone().unwrap_or_default();
                let available = status.update_available;
                desktop.status_heard(status);
                return Ok(Value::record([("latest", Value::from(latest.as_str())), ("available", Value::Bool(available))]));
            }
            // A development build: the public stable feed, beside the
            // engine's own report.
            let manifest = on_tokio(async { zeron_update::fetch_latest_on(zeron_cli::DEFAULT_EDGE_URL, Channel::Stable).await })
                .await
                .ok_or("failed")?
                .map_err(|err| feed_outcome(&format!("{err:#}")))?;
            *a.checked.borrow_mut() = Some((manifest.version.clone(), now_ms()));
            a.publish();
            let available = a.available();
            Ok(Value::record([("latest", Value::from(manifest.version.as_str())), ("available", Value::Bool(available))]))
        }
    });
    let a = about.clone();
    host.async_action("about.apply", move |_| {
        let a = a.clone();
        async move {
            let desktop = a.desktop.borrow().clone();
            if let Some(desktop) = desktop {
                if let Some(why) = desktop.status.borrow().as_ref().and_then(|s| s.held.clone()) {
                    eprintln!("zeron-zgx: updates: held: {why}");
                    return Err("held".to_string());
                }
                // Its engine already updated this Mac, from another device: the
                // new bundle is on disk, so this window only has to reopen on it.
                // (The engine is new already, and stays up.)
                let moved_on = moved_on_to(&a.status.borrow(), APP_VERSION);
                if let (Some(version), InstallKind::MacApp { bundle }) = (moved_on, &desktop.install) {
                    zeron_update::relaunch_after_exit(std::path::Path::new("/usr/bin/open"), bundle);
                    if let Some(windows) = a.windows.borrow().as_ref() {
                        windows.quit();
                    }
                    return Ok(Value::record([("version", Value::from(version.as_str()))]));
                }
                if matches!(*desktop.flow.borrow(), Flow::Ready { .. }) {
                    let version = desktop.install_for_restart().map_err(|message| {
                        eprintln!("zeron-zgx: updates: install failed: {message}");
                        publish();
                        "failed".to_string()
                    })?;
                    publish();
                    // (The engine goes with it, so it comes back on the new
                    // bundle's rather than running on from the old: what's
                    // running stops cleanly, and carries on after.)
                    // (Not for long: the update's in, and a stuck engine mustn't
                    // keep the window from going.)
                    let engine = engine::get();
                    if engine.config().is_some_and(|c| c.follows_bundle()) {
                        let stopped = std::rc::Rc::new(std::cell::Cell::new(false));
                        let done = stopped.clone();
                        engine.spawn(async move {
                            if let Err(error) = engine::get().stop().await {
                                eprintln!("zeron-zgx: updates: the engine didn't stop: {error}");
                            }
                            done.set(true);
                        });
                        let started = std::time::Instant::now();
                        while !stopped.get() && started.elapsed() < ENGINE_STOP_PATIENCE {
                            zgx::timer::sleep(std::time::Duration::from_millis(50)).await;
                        }
                    }
                    if let Some(windows) = a.windows.borrow().as_ref() {
                        windows.quit();
                    }
                    return Ok(Value::record([("version", Value::from(version.as_str()))]));
                }
                if desktop.available().is_none() {
                    return Err("current".to_string());
                }
                desktop.start_download();
                let version = desktop.available().unwrap_or_default();
                return Ok(Value::record([("version", Value::from(version.as_str()))]));
            }
            match engine::get().call("ApplyUpdate", json!({})).await {
                Ok(answer) => Ok(Value::record([("version", Value::from(answer["version"].as_str().unwrap_or("")))])),
                Err(error) => Err(apply_outcome(&error).to_string()),
            }
        }
    });
    let a = about.clone();
    host.action("about.channel", move |beta| {
        let Some(desktop) = a.desktop.borrow().clone() else { return };
        *desktop.note.borrow_mut() = None;
        match zeron_update::desktop::choose_channel(&desktop.data_dir, beta.truthy()) {
            Ok(_) => {
                // What's staged was the other channel's offer.
                if matches!(*desktop.flow.borrow(), Flow::Ready { .. } | Flow::Failed { .. }) {
                    *desktop.flow.borrow_mut() = Flow::Idle;
                }
                *desktop.status.borrow_mut() = None;
                desktop.checker.check_now();
            }
            Err(why) => *desktop.note.borrow_mut() = Some(why),
        }
        a.publish();
    });
}

/// The first window's: the app's own updater (an app install), and the
/// engine's report either way.
fn start(about: &Rc<About>) {
    if let Some(desktop) = Desktop::start() {
        let mut statuses = desktop.checker.watch();
        let (tx, mut heard) = zgx::task::latest::<UpdateStatus>();
        tokio().spawn(async move {
            while statuses.changed().await.is_ok() {
                let status = statuses.borrow_and_update().clone();
                if !tx.send(status) {
                    break;
                }
            }
        });
        let me = desktop.clone();
        engine::get().spawn(async move {
            while let Some(status) = heard.recv().await {
                me.status_heard(status);
            }
        });
        *about.desktop.borrow_mut() = Some(desktop);
    }
    let weak = Rc::downgrade(about);
    engine::get().on_connect(move |rpc| {
        let Some(about) = weak.upgrade() else { return };
        let heard = Rc::downgrade(&about);
        let watch = rpc.watch("UpdateStatus", json!({}), move |item| {
            if let Some(about) = heard.upgrade() {
                *about.status.borrow_mut() = item;
                about.publish();
            }
        });
        *about.watch.borrow_mut() = Some(watch);
        let (me, rpc) = (about.clone(), rpc.clone());
        engine::get().spawn(async move {
            if let Ok(info) = rpc.call("EngineInfo", json!({})).await {
                *me.build.borrow_mut() = info["build"].as_str().map(Into::into);
                me.publish();
            }
        });
    });
}

/// Where the app's own updates come from: none for the beta (the Zeron app
/// beside it updates) or a debug build of the Zeron app (never one to
/// install over the real one); the fork's releases for a released Zeron app,
/// as the other interface's (crates/ui lib.rs `RELEASES_FEED`); a development
/// build's edge otherwise (never a bundle, so never asked).
fn app_feed(identity: crate::identity::Identity) -> Option<Option<String>> {
    match identity.build {
        crate::identity::Build::Beta => None,
        crate::identity::Build::Cutover if identity.updates_itself => Some(Some(zeron_update::RELEASES_FEED.to_owned())),
        crate::identity::Build::Cutover => None,
        crate::identity::Build::Development => Some(None),
    }
}

/// Whether a found release downloads by itself and installs on quit: only
/// when `ZERON_AUTO_UPDATE` says so, in the Zeron app. Not the other
/// interface's `autoUpdate`: until the fork's feed tells this interface's
/// releases from that one's, a release found may be the other app (U27-M14).
fn automatic(identity: crate::identity::Identity, forced: Option<bool>) -> bool {
    identity.updates_itself && forced == Some(true)
}

/// What a release feed's manifest (its `manifest.json`, as published) offers
/// this app, as its checker decides it (crates/update `desktop::current_offer`:
/// `fetch_latest` reads the fork's one feed as Stable's, whatever this Mac chose).
#[cfg(test)]
fn offer_of(manifest: &str, running: &str, engine: Option<&str>, macos: Option<&str>) -> zeron_update::Offer {
    let mut manifest: zeron_update::Manifest = serde_json::from_str(manifest).expect("a manifest");
    manifest.channel = Channel::Stable;
    zeron_update::offer(&manifest, running, manifest.channel, engine, macos)
}

/// Whether the bundle on disk has moved on from the app that's open, to what's
/// on offer or past it. (Not when it's the app that's open: going back to
/// stable from a beta offers an older version on purpose.)
fn already_there(on_disk: &str, offered: &str, open: &str) -> bool {
    on_disk != open && !zeron_update::version_newer(offered, on_disk)
}

/// The version this device's engine updated it to, past the app that's open:
/// another device updated it. (Only a finished update says so: a beta's engine
/// can be ahead of its app without one, and reopening wouldn't change that.)
fn moved_on_to(status: &Json, app_version: &str) -> Option<String> {
    let update = &status["update"];
    let target = update["targetVersion"].as_str()?;
    (update["state"].as_str() == Some("updated") && zeron_update::version_newer(target, app_version))
        .then(|| target.to_owned())
}

impl About {
    fn engine_version(&self) -> Option<String> {
        self.status.borrow()["currentVersion"].as_str().map(Into::into)
    }

    /// The newest version known: the app's own checker's (an app install),
    /// else the engine's or this window's check, whichever is newer.
    fn latest(&self) -> Option<String> {
        if let Some(desktop) = self.desktop.borrow().as_ref() {
            return desktop.status.borrow().as_ref().and_then(|s| s.latest_version.clone());
        }
        let theirs = self.status.borrow()["latestVersion"].as_str().map(String::from);
        let ours = self.checked.borrow().as_ref().map(|(v, _)| v.clone());
        match (theirs, ours) {
            (Some(a), Some(b)) => Some(if zeron_update::version_newer(&b, &a) { b } else { a }),
            (a, b) => a.or(b),
        }
    }

    fn available(&self) -> bool {
        if let Some(desktop) = self.desktop.borrow().as_ref() {
            return desktop.available().is_some();
        }
        match (self.latest(), self.engine_version()) {
            (Some(latest), Some(current)) => zeron_update::version_newer(&latest, &current),
            _ => self.status.borrow()["updateAvailable"].as_bool().unwrap_or(false),
        }
    }

    fn value(&self) -> Value {
        let status = self.status.borrow();
        let desktop = self.desktop.borrow().clone();
        let (theirs, ours, error) = match &desktop {
            Some(desktop) => {
                let s = desktop.status.borrow();
                (s.as_ref().and_then(|s| s.checked_at), None, s.as_ref().and_then(|s| s.error.clone()))
            }
            None => {
                let theirs = status["checkedAt"].as_i64();
                let ours = self.checked.borrow().as_ref().map(|(_, at)| *at);
                // The engine's last error, unless this window has checked since.
                let error = status["error"]
                    .as_str()
                    .filter(|_| ours.is_none_or(|o| theirs.is_some_and(|t| t > o)))
                    .map(String::from);
                (theirs, ours, error)
            }
        };
        let checked_at = theirs.max(ours);
        let mut fields = vec![
            ("app_version", Value::from(APP_VERSION)),
            ("app_build", Value::from(app_build().as_str())),
            ("available", Value::Bool(self.available())),
            ("self_updating", Value::Bool(false)),
            ("channels", Value::Bool(false)),
            ("switch", Value::Bool(false)),
            ("flow", Value::from("idle")),
        ];
        if crate::identity::CURRENT.build == crate::identity::Build::Cutover {
            fields.push(("app_name", Value::from(crate::identity::CURRENT.name)));
            fields.push(("releases", Value::from(zeron_update::RELEASES_PAGE)));
        }
        if let Some(desktop) = &desktop {
            fields.retain(|(key, _)| !matches!(*key, "self_updating" | "channels" | "switch" | "flow"));
            desktop.fields(&mut fields);
            if matches!(desktop.install, InstallKind::MacApp { .. })
                && let Some(version) = moved_on_to(&status, APP_VERSION)
            {
                fields.push(("moved_on_to", Value::from(version.as_str())));
            }
        }
        if let Some(v) = self.engine_version() {
            fields.push(("engine_version", Value::from(v.as_str())));
        }
        if let Some(b) = self.build.borrow().as_deref() {
            fields.push(("engine_build", Value::from(b)));
        }
        if let Some(l) = self.latest() {
            fields.push(("latest", Value::from(l.as_str())));
        }
        if let Some(at) = checked_at {
            fields.push(("checked_at", Value::from(at as f64)));
        }
        if let Some(e) = error {
            fields.push(("error", Value::from(e.as_str())));
        }
        Value::record(fields)
    }

    fn publish(&self) {
        let value = self.value();
        for shared in self.shared.borrow().iter() {
            shared.set(value.clone());
        }
    }
}

/// The engine's refusal, as what happened.
fn apply_outcome(error: &str) -> &'static str {
    let lower = error.to_ascii_lowercase();
    if lower.contains("not update-managed") {
        "unmanaged"
    } else if lower.contains("already up to date") {
        "current"
    } else {
        engine::outcome(error)
    }
}

/// A feed that couldn't be read, as what happened: `offline` when the
/// network didn't answer, else `failed`.
fn feed_outcome(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("error sending request") || lower.contains("dns") || lower.contains("connect") || lower.contains("timed out") {
        "offline".into()
    } else {
        "failed".into()
    }
}

/// This app's build: when its executable was built (seconds since the
/// epoch), as the engine says its own.
fn app_build() -> String {
    std::env::current_exe()
        .and_then(std::fs::metadata)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or_else(String::new, |d| d.as_secs().to_string())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_engine_s_refusals_say_what_happened() {
        assert_eq!(
            apply_outcome("this install is not update-managed — the desktop app updates from its UI"),
            "unmanaged"
        );
        assert_eq!(apply_outcome("already up to date (0.2.131)"), "current");
    }

    #[test]
    fn the_newest_known_version_is_the_engine_s_or_a_later_check() {
        let about = About::default();
        *about.status.borrow_mut() = json!({ "currentVersion": "0.2.130", "latestVersion": "0.2.130", "updateAvailable": false, "checkedAt": 5 });
        assert!(!about.available());
        *about.checked.borrow_mut() = Some(("0.2.131".into(), 10));
        assert_eq!(about.latest().as_deref(), Some("0.2.131"));
        assert!(about.available());
    }

    /// A development build has no app updater: the page shows the engine's,
    /// and says the app doesn't update itself.
    #[test]
    fn a_development_build_reports_without_updating_itself() {
        let about = About::default();
        let value = about.value();
        assert!(!value.field("self_updating").is_some_and(Value::truthy));
        assert!(!value.field("channels").is_some_and(Value::truthy));
        assert_eq!(value.field("app_version").map(|v| v.to_json()).as_deref(), Some(format!("\"{APP_VERSION}\"").as_str()));
        // Its name and its own release notes are the Zeron app's alone (About's copy and link stay as they were otherwise).
        assert_eq!(value.field("releases").is_some(), cfg!(feature = "cutover"));
        assert_eq!(value.field("app_name").is_some(), cfg!(feature = "cutover"));
    }

    #[test]
    fn what_another_device_already_installed_isn_t_offered_again() {
        // Updated under this window: nothing to download.
        assert!(already_there("0.3.0", "0.3.0", "0.2.0"));
        // Something newer still.
        assert!(!already_there("0.3.0", "0.4.0", "0.2.0"));
        // Nothing has moved: an update, or leaving a beta for an older stable.
        assert!(!already_there("0.2.0", "0.3.0", "0.2.0"));
        assert!(!already_there("0.3.0-beta.2", "0.2.9", "0.3.0-beta.2"));
    }

    #[test]
    fn an_app_is_behind_only_when_its_engine_finished_updating_past_it() {
        let status = |state: &str, target: &str| json!({ "currentVersion": target, "update": { "state": state, "targetVersion": target } });
        assert_eq!(moved_on_to(&status("updated", "0.3.0"), "0.2.0").as_deref(), Some("0.3.0"));
        // Reopened on it: nothing more to do.
        assert_eq!(moved_on_to(&status("updated", "0.3.0"), "0.3.0"), None);
        // Still on trial, or gone back: the bundle hasn't changed.
        assert_eq!(moved_on_to(&status("inProgress", "0.3.0"), "0.2.0"), None);
        assert_eq!(moved_on_to(&status("rolledBack", "0.3.0"), "0.2.0"), None);
        // A beta's engine ahead of its app, with no update: no restart offered.
        assert_eq!(moved_on_to(&json!({ "currentVersion": "0.3.0" }), "0.2.0"), None);
    }
}

#[cfg(test)]
mod update_tests {
    use super::*;
    use crate::identity::{Build, of};
    use zeron_update::Offer;

    fn manifest(name: &str) -> String {
        std::fs::read_to_string(format!("{}/tests/fixtures/updates/{name}.json", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    /// Only a released Zeron app updates itself, from the fork's releases; the
    /// beta leaves it to the Zeron app beside it, and a debug build of the
    /// Zeron app never installs over the real one.
    #[test]
    fn the_zeron_app_updates_itself_from_the_fork_s_releases() {
        assert_eq!(app_feed(of(Build::Cutover, true)), Some(Some(zeron_update::RELEASES_FEED.to_owned())));
        assert!(zeron_update::RELEASES_FEED.contains("ReinaMacCredy/zeron"));
        assert_eq!(app_feed(of(Build::Cutover, false)), None);
        assert_eq!(app_feed(of(Build::Beta, true)), None);
        assert_eq!(app_feed(of(Build::Development, true)), Some(None));
        // It checks and offers; it downloads by itself and installs on quit only when ZERON_AUTO_UPDATE says so.
        let zeron = of(Build::Cutover, true);
        assert!(!automatic(zeron, None));
        assert!(!automatic(zeron, Some(false)));
        assert!(automatic(zeron, Some(true)));
        assert!(!automatic(of(Build::Cutover, false), Some(true)));
        assert!(!automatic(of(Build::Beta, true), Some(true)));
        assert!(!automatic(of(Build::Development, true), Some(true)));
    }

    #[test]
    fn a_release_is_offered_as_the_checker_decides() {
        assert_eq!(offer_of(&manifest("stable-newer"), "0.2.132", Some("0.2.132"), Some("15.1")), Offer::Newer);
        assert_eq!(offer_of(&manifest("stable-same"), "0.2.132", None, Some("15.1")), Offer::Current);
        assert_eq!(offer_of(&manifest("stable-older"), "0.2.132", None, Some("15.1")), Offer::Current);
        assert_eq!(offer_of(&manifest("garbage-version"), "0.2.132", None, Some("15.1")), Offer::Current);
        assert!(matches!(offer_of(&manifest("needs-newer-macos"), "0.2.132", None, Some("14.6")), Offer::Held(why) if why.contains("macOS 15.0")));
        assert_eq!(offer_of(&manifest("needs-newer-macos"), "0.2.132", None, Some("15.0")), Offer::Newer);
        // A beta build on the fork's one feed (Stable's): its release replaces the beta though older,
        // unless the beta's engine is newer than that release's.
        assert_eq!(offer_of(&manifest("stable-older"), "0.2.133-beta.1", Some("0.2.131"), Some("15.1")), Offer::Switch);
        assert!(matches!(offer_of(&manifest("stable-older"), "0.2.133-beta.1", Some("0.2.132"), Some("15.1")), Offer::Held(_)));
    }
}
