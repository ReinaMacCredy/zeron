//! What the `zeron` app/CLI and the `zeron-engine` background engine share
//! that needs the engine: its env-resolved configuration, the allocator, the
//! MCP bridge agents run, the background engine's entry point, and the
//! subcommands that run the engine's code here (`zeron_cli::EngineCommand`).
//! The command line itself, paths, logging and the rest are `zeron_cli`'s.

#[cfg(target_os = "macos")]
pub mod allocator;
pub mod auth_cli;
pub mod background;
pub mod engine_commands;
pub mod ipc_cli;
pub mod plugin_cli;

pub use engine_commands::run_engine_command;
pub use zeron_cli::{
    DEFAULT_EDGE_URL, DEFAULT_WORKOS_CLIENT_ID, edge_url_from_env, init_logging,
    long_running_setup, open_log_file, paths, workos_client_id_from_env,
};

/// mimalloc, macOS only: libmalloc never returns the streaming churn's
/// high-water pages, so transient allocation became permanent RSS
/// (docs/memory-plan.md §1). Pinned to mimalloc v2 in the workspace manifest —
/// the crate's default v3 has the same pathology (churn retained as permanent
/// RSS, ~6x glibc's growth on identical workloads, no idle recovery). Linux
/// measured flat on glibc, so it keeps the system allocator. Its regions are
/// tagged as heap, not GPU memory ([`allocator`]).
#[cfg(target_os = "macos")]
#[global_allocator]
static ALLOC: allocator::Allocator = allocator::Allocator;

/// The env-resolved engine configuration shared by `headless`, `login`,
/// `logout`, and `status` — one resolution so the CLI auth commands always
/// operate on the exact session the daemon will load.
pub fn engine_config_from_env() -> zeron_engine::EngineConfig {
    // (Every way an engine starts here resolves its configuration first.)
    #[cfg(target_os = "macos")]
    zeron_engine::heap::set_trim(allocator::trim);
    // Dev-mode bearer (no WorkOS): an explicit token enables sync.
    let edge_token = std::env::var("ZERON_EDGE_TOKEN").ok();
    zeron_engine::EngineConfig {
        data_dir: paths::data_dir(),
        edge_url: edge_url_from_env(),
        ipc_port: zeron_cli::ipc_port_from_env(),
        default_harness: harness_from_env(),
        // WorkOS mode: the signed-in session's org wins; ZERON_ORG_ID (dev
        // default "dev-org") scopes the workspace room otherwise.
        org_id: std::env::var("ZERON_ORG_ID").ok(),
        // Real auth against production by default; see
        // `workos_client_id_from_env` for the dev-mode escape hatches.
        workos_client_id: workos_client_id_from_env(&edge_token),
        edge_token,
    }
}

/// `ZERON_HARNESS` (kebab-case id) picks the default harness for chats without a
/// config row — `mock` powers the e2e smoke; default `claude-code`.
pub fn harness_from_env() -> zeron_engine::HarnessId {
    match std::env::var("ZERON_HARNESS").as_deref().map(str::trim) {
        Ok("mock") => zeron_engine::HarnessId::Mock,
        Ok("codex") => zeron_engine::HarnessId::Codex,
        Ok("cursor") => zeron_engine::HarnessId::Cursor,
        Ok("devin") => zeron_engine::HarnessId::Devin,
        Ok("grok") => zeron_engine::HarnessId::Grok,
        Ok("hermes") => zeron_engine::HarnessId::Hermes,
        Ok("pi") => zeron_engine::HarnessId::Pi,
        Ok("antigravity") => zeron_engine::HarnessId::Antigravity,
        Ok("muse") => zeron_engine::HarnessId::Muse,
        _ => zeron_engine::HarnessId::ClaudeCode,
    }
}

/// `mcp --chat <id> --port <port>`: one bridge per agent session, and it
/// only relays stdio requests out to the engine over IPC and replies back.
/// Two workers keep a burst of parallel tool calls close to per-core speed
/// without a worker per core in every agent's bridge.
pub fn run_mcp(chat: String, port: Option<u16>) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let port = port.unwrap_or_else(|| engine_config_from_env().ipc_port);
    runtime.block_on(mcp_cli(chat, port))
}

/// `hook --chat <id> --port <port>`: Claude Code's PostToolUse hook.
pub fn run_hook(chat: String, port: Option<u16>) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let port = port.unwrap_or_else(|| engine_config_from_env().ipc_port);
    // A failing hook shows as an error on the agent's tool call: say nothing instead.
    if let Err(error) = runtime.block_on(hook_cli(chat, port)) {
        tracing::debug!(%error, "context hook skipped");
    }
    Ok(())
}

/// stdout is the MCP wire, so nothing else may print there: tracing already
/// goes to stderr, and every failure below is returned, not printed.
async fn mcp_cli(chat: String, ipc_port: u16) -> anyhow::Result<()> {
    // A run the headless engine revives at boot can start before the engine binds its port.
    let mut attempts = 0;
    let client = loop {
        match zeron_rpc::connect_ipc_as_agent(ipc_port, Some(&paths::data_dir())).await {
            Ok(client) => break client,
            Err(e) if attempts >= 40 => {
                anyhow::bail!("no engine listening on 127.0.0.1:{ipc_port} ({e})")
            }
            Err(_) => {
                attempts += 1;
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
        }
    };
    if let Ok(secret) = std::env::var(zeron_rpc::COMPUTER_RUN_SECRET_ENV)
        && let Err(error) = client
            .call(
                zeron_rpc::methods::BIND_RUN,
                serde_json::json!({ "chatId": chat, "runSecret": secret }),
            )
            .await
    {
        tracing::warn!(%error, chat = %chat, "the engine did not bind this server to its run");
    }
    let tools = zeron_mcp::ThreadTools::new(client, chat).with_ipc_port(ipc_port);
    zeron_mcp::serve(
        tokio::io::BufReader::new(tokio::io::stdin()),
        tokio::io::stdout(),
        &tools,
    )
    .await?;
    Ok(())
}

async fn hook_cli(chat: String, ipc_port: u16) -> anyhow::Result<()> {
    let mut input = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)?;
    let event: serde_json::Value = serde_json::from_str(&input)?;
    // A subagent's tool call: it cannot compact the chat it runs under.
    if event
        .get("agent_id")
        .and_then(|id| id.as_str())
        .is_some_and(|id| !id.is_empty())
    {
        return Ok(());
    }
    let client = zeron_rpc::connect_ipc_as_agent(ipc_port, Some(&paths::data_dir()))
        .await
        .map_err(|e| anyhow::anyhow!("no engine listening on 127.0.0.1:{ipc_port} ({e})"))?;
    let reply = client
        .call(
            zeron_rpc::methods::MID_TURN_NUDGE,
            serde_json::json!({ "chatId": chat }),
        )
        .await
        .map_err(|e| anyhow::anyhow!("MidTurnNudge failed: {e}"))?;
    if let Some(text) = reply.get("text").and_then(|text| text.as_str()) {
        let output = serde_json::json!({
            "hookSpecificOutput": { "hookEventName": "PostToolUse", "additionalContext": text }
        });
        println!("{output}");
    }
    Ok(())
}
