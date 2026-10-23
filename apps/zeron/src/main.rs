//! zeron — headed by default; `zeron headless` runs the engine alone. Both start
//! local-only without credentials. `zeron login` and `zeron logout` select the
//! profile used by the next engine start without mutating a live runtime.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod auth_cli;
mod daemon;
mod ui_scenario;
mod update_cli;

use clap::{Parser, Subcommand};
use zeron_app::{edge_url_from_env, engine_config_from_env, paths, workos_client_id_from_env};

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
    /// Inspect and drive a running Zeron app through its UI driver:
    /// `zeron ui list`, `zeron ui <action> ['<json args>'] [--target <t>]`,
    /// or `zeron ui scenario <file.json> [--target <t>] [--out <dir>]`.
    Ui {
        /// `list`, `scenario`, or a driver action such as state, tree, snapshot, wait, click or key.
        action: String,
        /// The action's arguments as one JSON object, such as '{"keys":"cmd-k"}'; for `scenario`, the scenario file.
        args: Option<String>,
        /// A pid or label from `zeron ui list` (exact, or a unique substring), or a socket path.
        #[arg(long)]
        target: Option<String>,
        /// For `scenario`: the report directory (default ~/.zeron/run/scenarios/<name>-<time>).
        #[arg(long)]
        out: Option<std::path::PathBuf>,
    },
    /// Cloud sleep handshake: ask the running engine to start no new work.
    /// Exits 0 when quiesced, 75 when busy (a turn in flight, a message
    /// waiting) — the edge then keeps the machine awake.
    #[command(hide = true)]
    Quiesce,
    #[cfg(target_os = "linux")]
    /// Trigger an Appshot in the running headed instance (desktop shortcut fallback).
    Appshot,
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

fn main() -> anyhow::Result<()> {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--noop-browser")) {
        return Ok(());
    }
    #[cfg(windows)]
    attach_parent_console();
    let cli = Cli::parse();
    #[cfg(windows)]
    if let Some(pid) = cli.wait_for_exit {
        zeron_update::windows::wait_for_exit(pid)?;
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
        zeron_app::init_logging(
            long_running,
            mode,
            matches!(
                &cli.command,
                Some(Command::Mcp { .. } | Command::Hook { .. } | Command::Ui { .. })
            ),
        );
    }
    if long_running {
        zeron_app::long_running_setup();
    }

    match cli.command {
        Some(Command::Headless) => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(async {
                let engine = zeron_engine::Engine::new(engine_config_from_env());
                engine.run().await
            })
        }
        Some(Command::Login) => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(auth_cli::login(engine_config_from_env()))
        }
        Some(Command::Logout) => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(auth_cli::logout(engine_config_from_env()))
        }
        Some(Command::Status) => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(auth_cli::status(engine_config_from_env()))
        }
        Some(Command::Sync) => {
            let runtime = tokio::runtime::Runtime::new()?;
            let config = engine_config_from_env();
            runtime.block_on(sync_cli(config.ipc_port, &config.data_dir))
        }
        Some(Command::Mcp { chat, port }) => zeron_app::run_mcp(chat, port),
        Some(Command::Hook { chat, port }) => zeron_app::run_hook(chat, port),
        Some(Command::Ui {
            action,
            args,
            target,
            out,
        }) => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            if action == "scenario" {
                let file = args.ok_or_else(|| {
                    anyhow::anyhow!(
                        "usage: zeron ui scenario <file.json> [--target <t>] [--out <dir>]"
                    )
                })?;
                match runtime.block_on(ui_scenario::run(
                    std::path::Path::new(&file),
                    target.as_deref(),
                    out,
                )) {
                    Ok(true) => return Ok(()),
                    Ok(false) => std::process::exit(1),
                    Err(message) => {
                        eprintln!("{message}");
                        std::process::exit(1);
                    }
                }
            }
            runtime.block_on(ui_cli(action, args, target))
        }
        Some(Command::Quiesce) => {
            let runtime = tokio::runtime::Runtime::new()?;
            let config = engine_config_from_env();
            runtime.block_on(quiesce_cli(config.ipc_port, &config.data_dir))
        }
        Some(Command::Engine) => zeron_app::background::run(),
        #[cfg(target_os = "linux")]
        Some(Command::Appshot) => {
            zeron_ui::appshots::request_running_appshot(&engine_config_from_env().data_dir)
                .map_err(anyhow::Error::msg)
        }
        Some(Command::Update { check }) => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(update_cli::update(zeron_update::RELEASES_FEED, check))
        }
        Some(Command::Daemon { command }) => match command {
            DaemonCommand::Install => daemon::install(&engine_config_from_env().data_dir),
            DaemonCommand::Uninstall => daemon::uninstall(),
            DaemonCommand::Start => daemon::start(),
            DaemonCommand::Stop => daemon::stop(),
            DaemonCommand::Restart => daemon::restart(),
            DaemonCommand::Status => daemon::status(),
        },
        None => {
            let edge_token = std::env::var("ZERON_EDGE_TOKEN").ok();
            // Headed: the UI probes ZERON_IPC_PORT and connects to a running
            // daemon, or embeds the engine in-process (ARCHITECTURE §1).
            zeron_ui::run_app(zeron_ui::UiConfig {
                data_dir: paths::data_dir(),
                ipc_port: std::env::var("ZERON_IPC_PORT")
                    .ok()
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(27654),
                edge_url: edge_url_from_env(),
                workos_client_id: workos_client_id_from_env(&edge_token),
                edge_token,
                org_id: std::env::var("ZERON_ORG_ID").ok(),
                default_harness: zeron_ui::HarnessId::ClaudeCode,
                initial_url: cli.open_url,
            });
            Ok(())
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

/// `zeron quiesce`'s exit status when the engine is busy (EX_TEMPFAIL); the
/// edge's sleep workflow reads it (`QUIESCE_BUSY_EXIT`).
const QUIESCE_BUSY_EXIT: i32 = 75;

async fn quiesce_cli(ipc_port: u16, data_dir: &std::path::Path) -> anyhow::Result<()> {
    let client = zeron_rpc::connect_ipc(ipc_port, Some(data_dir))
        .await
        .map_err(|e| anyhow::anyhow!("no engine listening on 127.0.0.1:{ipc_port} ({e})"))?;
    let reply = client
        .call(zeron_rpc::methods::QUIESCE_FOR_SLEEP, serde_json::json!({}))
        .await
        .map_err(|e| anyhow::anyhow!("QuiesceForSleep failed: {e}"))?;
    if reply["quiesced"].as_bool() == Some(true) {
        println!("quiesced");
        return Ok(());
    }
    eprintln!(
        "busy: {}",
        reply["busy"].as_str().unwrap_or("the engine is busy")
    );
    std::process::exit(QUIESCE_BUSY_EXIT);
}

async fn ui_cli(
    action: String,
    args: Option<String>,
    target: Option<String>,
) -> anyhow::Result<()> {
    let parsed = args.as_deref().map(serde_json::from_str).transpose();
    let mut request =
        match parsed.map_err(|e| anyhow::anyhow!("the action's arguments are not JSON: {e}"))? {
            None => serde_json::Map::new(),
            Some(serde_json::Value::Object(args)) => args,
            Some(_) => anyhow::bail!("the action's arguments must be one JSON object"),
        };
    request.insert("action".into(), action.into());
    if let Some(target) = target {
        request.insert("target".into(), target.into());
    }
    match zeron_mcp::ui_driver::run(&serde_json::Value::Object(request), None, None).await {
        Ok(reply) => {
            println!("{}", zeron_mcp::ui_driver::text(&reply));
            Ok(())
        }
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    }
}

/// `zeron sync`: dial the running engine's IPC and print per-room sync state.
/// The introspection surface every 2026-08 incident was missing — "is this
/// device's workspace room actually receiving?" as a one-liner.
async fn sync_cli(ipc_port: u16, data_dir: &std::path::Path) -> anyhow::Result<()> {
    let client = zeron_rpc::connect_ipc(ipc_port, Some(data_dir))
        .await
        .map_err(|e| {
            anyhow::anyhow!("no engine listening on 127.0.0.1:{ipc_port} ({e}) — is zeron running?")
        })?;
    let status = client
        .call(zeron_rpc::methods::SYNC_STATUS, serde_json::json!({}))
        .await
        .map_err(|e| anyhow::anyhow!("SyncStatus failed: {e}"))?;
    let now = status.get("nowMs").and_then(|v| v.as_i64()).unwrap_or(0);
    let age = |ms: i64| -> String {
        if ms <= 0 {
            return "never".into();
        }
        let s = (now - ms).max(0) / 1000;
        if s >= 3600 {
            format!("{}h{}m ago", s / 3600, (s % 3600) / 60)
        } else if s >= 60 {
            format!("{}m{}s ago", s / 60, s % 60)
        } else {
            format!("{s}s ago")
        }
    };
    let room_line = |room: Option<&serde_json::Value>| -> String {
        let Some(room) = room else {
            return "no room (dialing or edge-less)".into();
        };
        let get = |k: &str| room.get(k).and_then(|v| v.as_i64()).unwrap_or(0);
        // REJECTED is loud and only shown when nonzero: rejected writes with
        // a fresh-looking room is exactly the latched-session wedge
        // (2026-08-04) this readout previously masked.
        let rejected = get("rejected");
        format!(
            "{} pushed {} · acked {} · rejoins {} probes {} resyncs {} drops {}{}",
            if room.get("connected").and_then(|v| v.as_bool()) == Some(true) {
                "connected ·"
            } else {
                "DISCONNECTED ·"
            },
            age(get("lastPushedMs")),
            age(get("lastAckMs")),
            get("rejoins"),
            get("probes"),
            get("fullResyncs"),
            get("disconnects"),
            if rejected > 0 {
                format!(" REJECTED {rejected}")
            } else {
                String::new()
            },
        )
    };
    println!(
        "Device:    {}",
        status
            .get("deviceId")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
    );
    println!(
        "Workspace: {}",
        room_line(status.get("workspace").filter(|v| !v.is_null()))
    );
    let chats = status
        .get("chats")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if chats.is_empty() {
        println!("Chats:     none open");
    }
    // Chat rooms speak chat2: cursor/head tell "am I caught up?", pending
    // tells "did my writes leave?", resets/rejected are the loud tells.
    let chat_line = |room: Option<&serde_json::Value>| -> String {
        let Some(room) = room else {
            return "no room (dialing or edge-less)".into();
        };
        let get = |k: &str| room.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
        let resets = get("serverResets");
        let rejected = get("rejected");
        format!(
            "{} cursor {}/{} · pending {} · rows {} ({}KB) · rejoins {} drops {}{}{}",
            if room.get("connected").and_then(|v| v.as_bool()) == Some(true) {
                "connected ·"
            } else {
                "DISCONNECTED ·"
            },
            get("cursor"),
            get("headSeq"),
            get("pendingPushes"),
            get("rowCount"),
            get("rowBytes") / 1024,
            get("rejoins"),
            get("disconnects"),
            if resets > 0 {
                format!(" RESETS {resets}")
            } else {
                String::new()
            },
            if rejected > 0 {
                format!(" REJECTED {rejected}")
            } else {
                String::new()
            },
        )
    };
    for chat in &chats {
        println!(
            "Chat {}: {}",
            chat.get("chatId")
                .and_then(|v| v.as_str())
                .map(|s| &s[..s.len().min(8)])
                .unwrap_or("?"),
            chat_line(chat.get("room").filter(|v| !v.is_null()))
        );
    }
    Ok(())
}
