//! `zeron-engine` — the desktop app's background engine, as a process of its
//! own ("Zeron Engine" in Activity Monitor): the app starts it and attaches;
//! sessions keep running with the app closed. Without the UI linked in.
//!
//! It also answers what the engine runs its own executable for: `mcp` and
//! `hook` (the thread-tools bridge and Claude's PostToolUse hook, each
//! `--chat <id> --port <port>`) and `--noop-browser` (browser suppression for
//! agents' sign-in flows).

#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--noop-browser") => Ok(()),
        Some(bridge @ ("mcp" | "hook")) => {
            zeron_app::init_logging(false, bridge, true);
            let chat = flag(&args, "--chat")
                .ok_or_else(|| anyhow::anyhow!("{bridge} needs --chat <id>"))?;
            let port = flag(&args, "--port").map(|port| port.parse()).transpose()?;
            if bridge == "mcp" {
                zeron_app::run_mcp(chat, port)
            } else {
                zeron_app::run_hook(chat, port)
            }
        }
        Some("--version" | "-V") => {
            println!("zeron-engine {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => zeron_app::background::run(),
    }
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|ix| args.get(ix + 1).cloned())
}
