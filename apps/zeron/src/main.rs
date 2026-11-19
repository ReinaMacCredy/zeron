//! zeron — the command line with the engine linked in: `zeron headless` runs
//! the engine alone, and the engine's other subcommands run here. It starts
//! local-only without credentials. `zeron login` and `zeron logout` select
//! the profile used by the next engine start without mutating a live runtime.
//!
//! The command line is `zeron_cli`'s (shared with the engine helper and the
//! zgx interface). The desktop app is the zgx interface (apps/zgx), which
//! ships its own `zeron`: this one opens no window.

mod triage_cli;
mod ui_scenario;

use zeron_cli::{Engines, Launch, UiCommand};

fn main() -> anyhow::Result<()> {
    match zeron_cli::run(zeron_update::current_version(), Engines::InProcess) {
        Launch::Done(result) => result,
        Launch::Engine(command) => zeron_app::run_engine_command(command),
        Launch::Ui(command) => run_ui(command),
        Launch::Triage { dry_run } => {
            triage_cli::run(&zeron_app::engine_config_from_env().data_dir, dry_run)
        }
        #[cfg(target_os = "linux")]
        Launch::Appshot => anyhow::bail!("Appshots are the desktop app's: open Zeron and use its shortcut"),
        Launch::Window { .. } => anyhow::bail!(
            "this `zeron` has no window: open the Zeron app, or run `zeron headless` for the engine alone"
        ),
    }
}

fn run_ui(command: UiCommand) -> anyhow::Result<()> {
    let UiCommand {
        action,
        args,
        target,
        out,
        n,
        mock,
        zgx,
        name,
        path,
        app_args,
    } = command;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    if action == "scenario" {
        let file = args.ok_or_else(|| {
            anyhow::anyhow!("usage: zeron app scenario <file.json> [--target <t>] [--out <dir>]")
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
    let mut flags = serde_json::Map::new();
    if let Some(n) = n {
        flags.insert("n".into(), n.into());
    }
    if mock {
        flags.insert("mock".into(), true.into());
    }
    if zgx {
        flags.insert("zgx".into(), true.into());
    }
    if let Some(name) = name {
        flags.insert("name".into(), name.into());
    }
    if let Some(path) = path {
        flags.insert("path".into(), path.to_string_lossy().into_owned().into());
    }
    if !app_args.is_empty() {
        flags.insert("appArgs".into(), app_args.into());
    }
    runtime.block_on(ui_cli(action, args, target, flags))
}

async fn ui_cli(
    action: String,
    args: Option<String>,
    target: Option<String>,
    flags: serde_json::Map<String, serde_json::Value>,
) -> anyhow::Result<()> {
    let args = match args.as_deref() {
        Some(step @ ("start" | "stop")) if action == "record" => {
            Some(serde_json::json!({ "do": step }).to_string())
        }
        _ => args,
    };
    let parsed = args.as_deref().map(serde_json::from_str).transpose();
    let mut request =
        match parsed.map_err(|e| anyhow::anyhow!("the action's arguments are not JSON: {e}"))? {
            None => serde_json::Map::new(),
            Some(serde_json::Value::Object(args)) => args,
            Some(_) => anyhow::bail!("the action's arguments must be one JSON object"),
        };
    request.extend(flags);
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
