//! `zeron`'s command line, for every binary that answers to it: the Zeron
//! app (`apps/zeron`), its engine helper (`zeron-engine`) and the zgx
//! interface (`apps/zgx`). One parser, one help text, one set of exit codes,
//! with no engine and no interface linked in.
//!
//! [`run`] parses this process's arguments and runs what needs neither
//! (`update`, `daemon`, `--noop-browser`, help and version). What's left is
//! the caller's:
//!   - no subcommand: open the window ([`Launch::Window`]);
//!   - a subcommand that runs the engine's code here ([`EngineCommand`]:
//!     `headless`, `engine`, `login`, `logout`, `status`, `sync`, `quiesce`,
//!     `mcp`, `plugin`, `template`). A binary that links the engine runs it
//!     itself ([`Engines::InProcess`], `zeron_app::run_engine_command`); an
//!     interface that doesn't hands the whole command line to the engine
//!     helper beside it ([`Engines::Helper`]: `Contents/MacOS/Zeron Engine`
//!     in a macOS bundle), which replaces this process and runs it there.

pub mod daemon;
mod env;
pub mod logging;
pub mod paths;
pub mod update_cli;

use clap::{CommandFactory as _, FromArgMatches as _, Parser, Subcommand};

pub use env::{
    DEFAULT_EDGE_URL, DEFAULT_IPC_PORT, DEFAULT_WORKOS_CLIENT_ID, edge_url_from_env, ipc_port_from,
    ipc_port_from_env, workos_client_id_from_env,
};
pub use logging::{RotatingLog, init_logging, long_running_setup, open_log_file};

#[derive(Parser)]
#[command(
    name = "zeron",
    version,
    about = "Multi-device controller for coding agents"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// Open a Zeron conversation URL.
    #[arg(value_name = "URL")]
    open_url: Option<String>,
    #[cfg(windows)]
    #[arg(long, hide = true)]
    wait_for_exit: Option<u32>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the engine without a UI (local-only unless a saved session enables sync).
    Headless,
    /// The desktop app's background engine (`zeron-engine` where it ships
    /// separately): started by the app, it keeps sessions running with the
    /// app closed.
    #[command(hide = true)]
    Engine,
    /// Sign in and enable sync on the next engine start.
    Login,
    /// Remove the saved session and return to local-only on the next start.
    Logout,
    /// Show workspace mode, optional auth, and engine status.
    Status,
    /// Live sync introspection from the running engine: per-room connection
    /// state, last pushed-frame/ack ages, rejoin/probe/resync counters.
    Sync,
    /// Cloud sleep handshake: ask the running engine to start no new work.
    /// Exits 0 when quiesced, 75 when busy (a turn in flight, a message
    /// waiting) — the edge then keeps the machine awake.
    #[command(hide = true)]
    Quiesce,
    #[cfg(target_os = "linux")]
    /// Trigger an Appshot in the running headed instance (desktop shortcut fallback).
    Appshot,
    /// Serve one chat's thread tools over MCP stdio. The engine spawns this
    /// next to each agent process; not meant to be run by hand.
    #[command(hide = true)]
    Mcp {
        /// The chat whose agent is on the other end of stdio.
        #[arg(long)]
        chat: String,
        /// The IPC port of the engine that spawned this server.
        #[arg(long)]
        port: Option<u16>,
    },
    /// Claude Code PostToolUse hook: prints the compaction reminder the
    /// engine has for this chat's running turn, if any.
    #[command(hide = true)]
    Hook {
        #[arg(long)]
        chat: String,
        #[arg(long)]
        port: Option<u16>,
    },
    /// Inspect and drive a running Zeron app through its private API:
    /// `zeron app list`, `zeron app <action> ['<json args>'] [--target <t>]`,
    /// `zeron app scenario <file.json> [--target <t>] [--out <dir>]`,
    /// `zeron app launch [--n N] [--mock] [--zgx] [--name <app>] [-- <app args>]`, `zeron app quit --target <t>`,
    /// or `zeron app record start|stop [--path <file.json>] --target <t>`.
    #[command(name = "app", alias = "ui", hide = true)]
    Ui(UiCommand),
    /// Make, sign, check and list plugins (through the running Zeron).
    Plugin {
        #[command(subcommand)]
        command: PluginCommand,
    },
    /// List, make, use and delete templates: whole replacements of Zeron's
    /// interface (through the running Zeron). `zeron template use zeron`
    /// puts Zeron's own back.
    Template {
        #[command(subcommand)]
        command: TemplateCommand,
    },
    /// Manage `zeron headless` as a background service (launchd / systemd --user).
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },
    /// Check for a newer release and apply it (download → verify → swap →
    /// service restart). `--check` only reports (exits 1 when one is available).
    Update {
        #[arg(long)]
        check: bool,
    },
    /// Debug a Zeron problem with your own agent: writes the triage playbook
    /// and this machine's facts, then starts Claude Code or Codex on them.
    /// Use it when the app will not start.
    Triage {
        /// Write the files and print the command instead of starting an agent.
        #[arg(long)]
        dry_run: bool,
    },
    /// Watches a new version while it proves itself after an update: started by
    /// the old version just before it hands over.
    #[command(name = "update-watchdog", hide = true)]
    UpdateWatchdog,
}

/// `zeron app`: run by the `zeron` binary, which links the UI driver client.
#[derive(clap::Args)]
pub struct UiCommand {
    /// `list`, `scenario`, `launch`, `quit`, `record`, or a driver action such as state, tree, snapshot, wait, click or key.
    pub action: String,
    /// The action's arguments as one JSON object, such as '{"keys":"cmd-k"}'; for `scenario`, the scenario file; for `record`, `start` or `stop`.
    pub args: Option<String>,
    /// A pid or label from `zeron app list` (exact, or a unique substring), or a socket path.
    #[arg(long)]
    pub target: Option<String>,
    /// For `scenario`: the report directory (default ~/.zeron/run/scenarios/<name>-<time>).
    #[arg(long)]
    pub out: Option<std::path::PathBuf>,
    /// For `launch`: the Dev number 60-99 (bundle target/macos-dev<N>, port 49800+N); defaults to the lowest free one.
    #[arg(long)]
    pub n: Option<u16>,
    /// For `launch`: run the app with the mock harness.
    #[arg(long)]
    pub mock: bool,
    /// For `launch`: start apps/zgx instead of a gpui Dev bundle.
    #[arg(long)]
    pub zgx: bool,
    /// For `launch`: the app name (default `Zeron Dev <N>`, or `Zeron Beta Dev <N>` with `--zgx`).
    #[arg(long)]
    pub name: Option<String>,
    /// For `snapshot` and `record stop`: the absolute file to write; for `compare`: the mockup .html file in the current folder.
    #[arg(long)]
    pub path: Option<std::path::PathBuf>,
    /// For `launch`: arguments passed to the app.
    #[arg(last = true)]
    pub app_args: Vec<String>,
}

#[derive(Subcommand)]
enum DaemonCommand {
    /// Install, enable, and start the service (captures ZERON_* env).
    Install,
    /// Stop and remove the service.
    Uninstall,
    /// Start the installed service.
    Start,
    /// Stop the service.
    Stop,
    /// Restart the service.
    Restart,
    /// Show the service manager's view of the daemon.
    Status,
}

#[derive(Subcommand)]
pub enum PluginCommand {
    /// Make a new plugin in Zeron's plugins folder, signed as you.
    New {
        /// Its id: lowercase letters, digits and dashes.
        id: String,
        /// Its name (defaults to the id).
        #[arg(long)]
        name: Option<String>,
        /// A permission it asks for (repeat for more).
        #[arg(long = "permission", value_name = "P")]
        permissions: Vec<String>,
    },
    /// Sign a plugin as you: an id in the plugins folder, or a folder's path.
    Sign { target: String },
    /// Check a plugin folder's signature and who made it.
    Verify { path: String },
    /// List the plugins Zeron knows.
    List,
}

#[derive(Subcommand)]
pub enum TemplateCommand {
    /// List the templates on this device, Zeron's own first.
    List,
    /// Run Zeron's window with a template. `zeron template use zeron` puts
    /// Zeron's own back, even with Zeron closed.
    Use { id: String },
    /// Make a new template: a full copy of Zeron's interface to rewrite.
    Create {
        /// Its id: lowercase letters, digits and dashes.
        id: String,
        /// Its name (defaults to the id).
        #[arg(long)]
        name: Option<String>,
        /// A template to start from (defaults to Zeron's own).
        #[arg(long)]
        from: Option<String>,
    },
    /// Show a template's errors, out-of-date files and what it inherits.
    Status { id: String },
    /// Delete a template (it's set aside in the templates folder's .removed).
    Delete { id: String },
}

/// A subcommand that runs the engine's own code in this process: what the
/// engine helper (or a binary linking the engine) does with it.
pub enum EngineCommand {
    Headless,
    Engine,
    Login,
    Logout,
    Status,
    Sync,
    Quiesce,
    Mcp { chat: String, port: Option<u16> },
    Hook { chat: String, port: Option<u16> },
    Plugin(PluginCommand),
    Template(TemplateCommand),
}

/// Where this binary runs [`EngineCommand`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engines {
    /// Here: the binary links the engine (`zeron`, `zeron-engine`).
    InProcess,
    /// In the engine helper beside this binary, which replaces this process
    /// with the same arguments (an interface that doesn't link the engine).
    Helper,
}

/// What's left for the binary once [`run`] has parsed its arguments.
pub enum Launch {
    /// A command ran here; exit with its result.
    Done(anyhow::Result<()>),
    /// Run this in-process ([`Engines::InProcess`] only); logging is set up.
    Engine(EngineCommand),
    /// No subcommand: open the window, on a conversation URL if one was given.
    Window { open_url: Option<String> },
    /// `zeron app`, for a binary that links the UI driver client.
    Ui(UiCommand),
    /// `zeron triage`, for a binary that links the engine's triage playbook.
    Triage { dry_run: bool },
    /// Ask the running headed instance for an Appshot (the interface's own).
    #[cfg(target_os = "linux")]
    Appshot,
}

/// The parser, as `binary`'s `--version` names it: `version` is the build's
/// (the workspace's, or a beta interface's own, `0.2.136-beta.1`).
pub fn command(version: &'static str) -> clap::Command {
    Cli::command().version(version)
}

/// Parse this process's arguments and run what needs neither the engine nor
/// the interface. Help, `--version` and argument errors print and exit here,
/// as clap does.
pub fn run(version: &'static str, engines: Engines) -> Launch {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--noop-browser")) {
        return Launch::Done(Ok(()));
    }
    #[cfg(windows)]
    attach_parent_console();
    let cli = match Cli::from_arg_matches(&command(version).get_matches()) {
        Ok(cli) => cli,
        Err(error) => error.exit(),
    };
    if engines == Engines::Helper && cli.command.as_ref().is_some_and(Command::runs_engine) {
        // The helper parses the same arguments and does all of the below
        // itself (logging, Windows' image cleanup, the command).
        let exe = match std::env::current_exe() {
            Ok(exe) => exe,
            Err(error) => return Launch::Done(Err(error.into())),
        };
        let error = zeron_launch::exec_helper(&exe, std::env::args_os().skip(1));
        return Launch::Done(Err(anyhow::anyhow!("couldn't start the engine: {error}")));
    }
    #[cfg(windows)]
    if let Some(pid) = cli.wait_for_exit {
        if let Err(error) = zeron_update::windows::wait_for_exit(pid) {
            return Launch::Done(Err(error));
        }
    } else if matches!(
        &cli.command,
        None | Some(Command::Headless) | Some(Command::Engine)
    ) {
        zeron_update::windows::cleanup_previous_image();
    }
    let long_running = matches!(&cli.command, None | Some(Command::Headless));
    let mode = if cli.command.is_some() {
        "headless"
    } else {
        "headed"
    };
    // `zeron mcp` owns stdout for the protocol: a single log line on it
    // would corrupt the JSON-RPC stream, so its diagnostics go to stderr.
    // The background engine logs on its own (`zeron_app::background`).
    if !matches!(&cli.command, Some(Command::Engine)) {
        init_logging(
            long_running,
            mode,
            matches!(
                &cli.command,
                Some(
                    Command::Mcp { .. }
                        | Command::Hook { .. }
                        | Command::Ui(_)
                        | Command::Plugin { .. }
                        | Command::Template { .. }
                )
            ),
        );
    }
    if long_running {
        long_running_setup();
    }

    let engine = |command| match engines {
        Engines::InProcess => Launch::Engine(command),
        // (Handed to the helper above.)
        Engines::Helper => unreachable!("engine commands run in the helper"),
    };
    match cli.command {
        Some(Command::Headless) => engine(EngineCommand::Headless),
        Some(Command::Engine) => engine(EngineCommand::Engine),
        Some(Command::Login) => engine(EngineCommand::Login),
        Some(Command::Logout) => engine(EngineCommand::Logout),
        Some(Command::Status) => engine(EngineCommand::Status),
        Some(Command::Sync) => engine(EngineCommand::Sync),
        Some(Command::Quiesce) => engine(EngineCommand::Quiesce),
        Some(Command::Mcp { chat, port }) => engine(EngineCommand::Mcp { chat, port }),
        Some(Command::Hook { chat, port }) => engine(EngineCommand::Hook { chat, port }),
        Some(Command::Ui(command)) => Launch::Ui(command),
        Some(Command::Triage { dry_run }) => Launch::Triage { dry_run },
        Some(Command::Plugin { command }) => engine(EngineCommand::Plugin(command)),
        Some(Command::Template { command }) => engine(EngineCommand::Template(command)),
        #[cfg(target_os = "linux")]
        Some(Command::Appshot) => Launch::Appshot,
        Some(Command::Update { check }) => Launch::Done(
            tokio::runtime::Runtime::new()
                .map_err(anyhow::Error::from)
                .and_then(|runtime| runtime.block_on(update_cli::update(check))),
        ),
        Some(Command::UpdateWatchdog) => Launch::Done(zeron_update::transaction::run_watchdog()),
        Some(Command::Daemon { command }) => Launch::Done(match command {
            DaemonCommand::Install => daemon::install(&paths::data_dir()),
            DaemonCommand::Uninstall => daemon::uninstall(),
            DaemonCommand::Start => daemon::start(),
            DaemonCommand::Stop => daemon::stop(),
            DaemonCommand::Restart => daemon::restart(),
            DaemonCommand::Status => daemon::status(),
        }),
        None => Launch::Window {
            open_url: cli.open_url,
        },
    }
}

impl Command {
    /// Whether it runs the engine's code ([`EngineCommand`]).
    fn runs_engine(&self) -> bool {
        match self {
            Self::Headless
            | Self::Engine
            | Self::Login
            | Self::Logout
            | Self::Status
            | Self::Sync
            | Self::Quiesce
            | Self::Mcp { .. }
            | Self::Hook { .. }
            | Self::Plugin { .. }
            | Self::Template { .. } => true,
            #[cfg(target_os = "linux")]
            Self::Appshot => false,
            Self::Update { .. }
            | Self::UpdateWatchdog
            | Self::Daemon { .. }
            | Self::Ui(_)
            | Self::Triage { .. } => false,
        }
    }
}

#[cfg(windows)]
fn attach_parent_console() {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE,
        STD_OUTPUT_HANDLE, SetStdHandle,
    };

    // The GUI subsystem prevents Explorer from creating a console at startup.
    // Reuse an existing parent's console for CLI output and cargo run, without
    // allocating one. Attach before Clap so help and argument errors work too.
    // Preserve redirected pipes/files: attaching may replace standard handles.
    unsafe {
        let saved = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE]
            .map(|id| (id, GetStdHandle(id)));
        if AttachConsole(ATTACH_PARENT_PROCESS) != 0 {
            for (id, handle) in saved {
                if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
                    SetStdHandle(id, handle);
                }
            }
        }
    }
}

/// Every subcommand path of the command line (`[]`, `["plugin"]`,
/// `["plugin", "new"]`, …), hidden ones included: what the parity checks of
/// each binary that runs it walk (`apps/zeron/tests/cli.rs`,
/// `apps/zgx/tests/cli.rs`).
pub fn subcommand_paths(version: &'static str) -> Vec<Vec<String>> {
    fn walk(command: &clap::Command, path: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
        out.push(path.clone());
        for sub in command.get_subcommands() {
            if sub.get_name() == "help" {
                continue;
            }
            path.push(sub.get_name().to_owned());
            walk(sub, path, out);
            path.pop();
        }
    }
    let mut out = Vec::new();
    walk(&command(version), &mut Vec::new(), &mut out);
    out
}

/// `zeron <path…> --help` as a binary named `zeron` prints it: clap's own
/// answer to those arguments.
pub fn help_for(version: &'static str, path: &[String]) -> String {
    let argv = std::iter::once("zeron".to_owned())
        .chain(path.iter().cloned())
        .chain(std::iter::once("--help".to_owned()));
    match command(version).try_get_matches_from(argv) {
        Err(help) if help.kind() == clap::error::ErrorKind::DisplayHelp => help.render().to_string(),
        other => panic!("zeron {} --help didn't print help: {other:?}", path.join(" ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_is_well_formed() {
        command("0.0.0").debug_assert();
    }

    #[test]
    fn every_subcommand_is_reachable() {
        let paths = subcommand_paths("0.0.0");
        for expected in [
            &["headless"][..],
            &["engine"],
            &["login"],
            &["logout"],
            &["status"],
            &["sync"],
            &["quiesce"],
            &["mcp"],
            &["plugin", "new"],
            &["template", "use"],
            &["daemon", "install"],
            &["update"],
        ] {
            assert!(
                paths.iter().any(|path| path == expected),
                "{expected:?} missing from {paths:?}"
            );
        }
    }

    /// What runs where: the engine's commands go to the engine (in-process
    /// or the helper), the rest run here.
    #[test]
    fn engine_commands_are_the_ones_that_run_the_engine() {
        let parse = |args: &[&str]| {
            let argv = std::iter::once("zeron").chain(args.iter().copied());
            Cli::from_arg_matches(&command("0.0.0").try_get_matches_from(argv).unwrap())
                .unwrap()
                .command
        };
        for args in [
            &["headless"][..],
            &["engine"],
            &["login"],
            &["logout"],
            &["status"],
            &["sync"],
            &["quiesce"],
            &["mcp", "--chat", "c"],
            &["hook", "--chat", "c"],
            &["plugin", "list"],
            &["template", "list"],
        ] {
            assert!(parse(args).unwrap().runs_engine(), "{args:?}");
        }
        for args in [
            &["update", "--check"][..],
            &["daemon", "status"],
            &["ui", "list"],
            &["app", "list"],
            &["triage"],
        ] {
            assert!(!parse(args).unwrap().runs_engine(), "{args:?}");
        }
        assert!(parse(&[]).is_none());
    }

    #[test]
    fn plugin_and_template_show_in_help() {
        let help = help_for("0.0.0", &[]);
        for command in ["plugin", "template"] {
            assert!(
                help.lines()
                    .any(|line| line.trim_start().starts_with(command)),
                "{help}"
            );
        }
        assert!(
            !help
                .lines()
                .any(|line| line.trim_start().starts_with("app")),
            "{help}"
        );
    }

    #[test]
    fn the_version_is_the_build_s() {
        let version = command("0.2.136-beta.1").render_version();
        assert_eq!(version.trim(), "zeron 0.2.136-beta.1");
    }
}
