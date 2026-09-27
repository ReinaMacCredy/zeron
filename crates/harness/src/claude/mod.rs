//! Claude Code harness: spawns the installed `claude` CLI and speaks its
//! stream-json protocol directly — no adapter process in between. Resurrected
//! from the pre-ACP driver (see docs/research/harness.md) and modernized
//! against CLI 2.1.228.
//!
//! - stdout JSONL frames are normalized into [`AgentEvent`]s (init dedupe,
//!   subagent tagging, typed tool decoding, error-code mapping).
//! - PERMISSIONS ride the stdio control channel: `--permission-prompt-tool
//!   stdio` (undocumented — absent from `claude --help`, but it is the same
//!   transport the Claude Agent SDK's `query()` drives, and was re-validated
//!   live against 2.1.228: `can_use_tool` control requests arrive and
//!   allow/deny responses are honored). The alternative channel — an MCP
//!   permission tool — needs a server process and was rejected. The run's
//!   [`zeron_proto::PermissionMode`] picks the CLI mode (Ask → `default`,
//!   Auto → `acceptEdits`, Full → `bypassPermissions`) and whether a
//!   `can_use_tool` auto-allows (Full) or becomes an Allow/Deny question;
//!   `AskUserQuestion` always round-trips through
//!   [`RunControls::request_input`].
//! - DONE is the CLI's own `result` frame, eagerly: background work (a
//!   spawned subagent) never holds the turn. The CLI natively runs a second
//!   wake turn when a background task finishes — a fresh `init` (same
//!   session id, deduped) plus another `result` — and both are forwarded;
//!   the engine's parked-session resume path turns them into the
//!   done→Working→done wake.
//! - SUBAGENT frames arrive on the same stdout tagged with a top-level
//!   `parent_tool_use_id`; they are wrapped in [`AgentEvent::Subagent`] and
//!   NEVER folded into the parent feed (a background subagent interleaves
//!   with the parent's own stream — folding them in split contiguous text
//!   around phantom tool calls).
//! - Steering: queued [`SteerMessage`]s are written to stdin as user lines at
//!   any time, their attachments inlined like a first turn's; the CLI folds
//!   them into the running turn at its own step boundary.
//! - Interrupt: cancelling [`RunControls::interrupt`] sends the protocol-level
//!   interrupt control request, then escalates to SIGTERM and SIGKILL.

pub mod catalog;
mod discovery;
mod normalize;
mod wire;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use zeron_proto::{
    AgentEvent, Compaction, DoneStatus, HarnessId, Model, ReasoningLevel, RunRequest,
    SlashCommand, SlashCommandKind, SteeringMode, UserInputAnswer, UserInputQuestion,
};

use crate::process::{Child, ChildStdin, Command, Stdio};
use crate::{Harness, HarnessError, RunControls, Signal, send_signal, shutdown_child};
use catalog::{apply_ultrathink, to_effort};
use normalize::Normalizer;
use wire::{ControlRequestFrame, Frame, allow_response, control_response_line};

/// Locate the device's installed Claude Code CLI: our own PATH, then the
/// login-shell PATH snapshot (the user's shell init shapes PATH in ways a
/// GUI/service launch never sees — see [`crate::shell_env`]), then known
/// install locations as a last resort. The `CLAUDE_CODE_EXECUTABLE` override
/// is applied by [`ClaudeHarness::resolve_executable`], so availability and
/// launches agree on one resolution order. Resolved per call — cheap after
/// the snapshot is cached.
fn resolve_claude_executable() -> Option<PathBuf> {
    let mut extra = Vec::new();
    if let Some(home) = crate::executable::home_dir() {
        extra.push(home.join(".claude").join("local").join("claude"));
        extra.push(home.join(".local").join("bin").join("claude"));
    }
    extra.push(PathBuf::from("/opt/homebrew/bin/claude"));
    extra.push(PathBuf::from("/usr/local/bin/claude"));
    crate::executable::find_on_paths("claude", extra)
}

fn option_is_on(options: &serde_json::Map<String, Value>, key: &str) -> bool {
    match options.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => s == "on" || s == "true",
        _ => false,
    }
}

/// The Claude Code harness. Construct with [`ClaudeHarness::new`]; tests point
/// it at a fake CLI with [`ClaudeHarness::with_executable`].
pub struct ClaudeHarness {
    executable: Option<PathBuf>,
    /// Grace between the interrupt control request and SIGTERM.
    interrupt_grace: Duration,
    /// Grace between SIGTERM and SIGKILL.
    kill_grace: Duration,
    /// Command discovery cache: only a successful probe is cached, so a
    /// broken CLI retries on the next picker open (ACP-harness parity).
    initialize: discovery::InitializeCache,
    models_cache: crate::catalog::Catalog,
    workspace_commands: crate::skills::CommandDiscovery,
}

impl Default for ClaudeHarness {
    fn default() -> Self {
        Self {
            executable: None,
            interrupt_grace: Duration::from_secs(2),
            kill_grace: Duration::from_secs(3),
            initialize: discovery::InitializeCache::default(),
            models_cache: crate::catalog::Catalog::default(),
            workspace_commands: crate::skills::CommandDiscovery::default(),
        }
    }
}

impl ClaudeHarness {
    pub fn new() -> Self {
        Self::default()
    }

    /// Use a fixed CLI binary instead of PATH/known-location resolution.
    pub fn with_executable(mut self, path: impl Into<PathBuf>) -> Self {
        self.executable = Some(path.into());
        self
    }

    /// Tune the interrupt→SIGTERM→SIGKILL escalation timing.
    pub fn with_graces(mut self, interrupt_grace: Duration, kill_grace: Duration) -> Self {
        self.interrupt_grace = interrupt_grace;
        self.kill_grace = kill_grace;
        self
    }

    /// Skill names for the `$name` rewrite. The probe runs only when the
    /// prompt carries a `$` at all, so an ordinary first turn never waits on
    /// it; the initialize response it reads is cached.
    async fn skill_names(&self, prompt: &str) -> Vec<String> {
        if !prompt.contains('$') {
            return Vec::new();
        }
        match self.commands().await {
            Ok(commands) => commands
                .into_iter()
                .filter(|command| command.kind == SlashCommandKind::Skill)
                .map(|command| command.name)
                .collect(),
            Err(error) => {
                tracing::debug!(%error, "skill probe failed; `$` mentions sent as typed");
                Vec::new()
            }
        }
    }

    fn resolve_executable(&self) -> Result<PathBuf, HarnessError> {
        if let Some(p) = &self.executable {
            return crate::executable::validate_native_override(p);
        }
        if let Some(p) = std::env::var_os("CLAUDE_CODE_EXECUTABLE")
            && !p.is_empty()
        {
            return crate::executable::validate_native_override(&PathBuf::from(p));
        }
        resolve_claude_executable().ok_or_else(|| {
            HarnessError::NotInstalled(
                "claude (searched PATH, the login shell's PATH, ~/.claude/local, \
                 ~/.local/bin, /opt/homebrew/bin, /usr/local/bin, and \
                 fnm/nvm/volta/pnpm/bun install dirs; Windows also checks USERPROFILE \
                 and explicit NVM_SYMLINK/VOLTA_HOME/PNPM_HOME; set \
                 CLAUDE_CODE_EXECUTABLE to override)"
                    .into(),
            )
        })
    }

    fn build_command(&self, exe: &PathBuf, request: &RunRequest) -> Command {
        let mut cmd = Command::new(exe);
        crate::compose_child_path(&mut cmd, exe);
        cmd.args([
            "--print",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            // Required by the CLI alongside `-p --output-format stream-json`.
            "--verbose",
            "--include-partial-messages",
            // Newer Claude models emit no readable thinking text unless a
            // summary is asked for (raw reasoning stays provider-private).
            "--thinking-display",
            "summarized",
            // Route permission prompts to the stdio control channel so
            // `can_use_tool` (and AskUserQuestion in particular) reaches us.
            // Undocumented flag; validated live against 2.1.228.
            "--permission-prompt-tool",
            "stdio",
            // Echo each prompt back with its transcript uuid — the anchor a
            // later rewind or fork cuts at.
            "--replay-user-messages",
        ]);
        // File checkpoints are only recorded for sessions that ran with this
        // set; a chat that never had it can never restore code later.
        cmd.env("CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING", "true");
        // The 1M context window is selected via a model-id suffix
        // (`sonnet[1m]`), exactly how the CLI itself does it; fast mode and
        // always-on thinking are settings overrides. The suffix alone can't
        // express 200K: the catalog's long-context models are `native_1m` in
        // the CLI, so a bare id still runs at 1M and only the env kill switch
        // brings it down (see `catalog::context_window`).
        if let Some(model) = &request.model {
            let context_window = request
                .model_options
                .get("contextWindow")
                .and_then(Value::as_str);
            cmd.arg("--model");
            cmd.arg(if context_window == Some("1m") {
                format!("{model}[1m]")
            } else {
                model.clone()
            });
            if context_window == Some("200k") {
                cmd.env("CLAUDE_CODE_DISABLE_1M_CONTEXT", "1");
            }
        }
        if let Some(effort) = to_effort(request.reasoning, request.model.as_deref()) {
            cmd.args(["--effort", effort]);
        }
        // The CLI decides what asks under each mode (default asks for
        // everything but its own read-only safe list, acceptEdits also lets
        // file edits through; the interactive `auto` mode is silently
        // downgraded to `default` in print mode — all verified live against
        // 2.1.274); every `can_use_tool` that does reach stdio is a question
        // for the user unless the mode is Full, where the handler auto-allows
        // — Full keeps the `default` flag zeron always ran with, so the
        // unattended path is byte-for-byte the old one (`bypassPermissions`
        // would also be refused under a managed disableBypassPermissionsMode).
        let cli_mode = match request.permission_mode {
            zeron_proto::PermissionMode::Ask | zeron_proto::PermissionMode::Full => "default",
            zeron_proto::PermissionMode::Auto => "acceptEdits",
        };
        cmd.args(["--permission-mode", cli_mode]);
        if let Some(resume) = &request.resume {
            cmd.arg(format!("--resume={resume}"));
            // Naming the source's id, or no id, also skips copying file checkpoints (CLI 2.1.280).
            if let Some(forked_at) = &request.forked_at {
                let session = match &request.branch {
                    Some(_) => {
                        let minted = uuid::Uuid::new_v4().to_string();
                        cmd.args(["--session-id", &minted]);
                        minted
                    }
                    None => resume.clone(),
                };
                cmd.env(
                    "CLAUDE_CODE_RESUME_SOURCE_ALIVE",
                    format!("{session}|{forked_at}"),
                );
            }
            // A branch runs on a COPY of the resumed session cut at the kept
            // reply; `--resume-drops-turn` names the discarded prompt so the
            // CLI accepts a cut that spans its own bookkeeping frames. Both
            // are print-mode flags absent from `--help` (SDK `resumeSessionAt`
            // / `resumeDropsTurn`, 2.1.223+).
            if let Some(branch) = &request.branch {
                cmd.arg("--fork-session");
                if let Some(keep) = &branch.keep {
                    cmd.arg(format!("--resume-session-at={keep}"));
                }
                if let Some(drop) = &branch.drop {
                    cmd.arg(format!("--resume-drops-turn={drop}"));
                }
            }
        }
        let mut settings = serde_json::Map::new();
        if option_is_on(&request.model_options, "fastMode") {
            settings.insert("fastMode".into(), Value::Bool(true));
        }
        if option_is_on(&request.model_options, "thinking") {
            settings.insert("alwaysThinkingEnabled".into(), Value::Bool(true));
        }
        if request.reasoning == Some(ReasoningLevel::Ultracode) {
            settings.insert("ultracode".into(), Value::Bool(true));
        }
        if !settings.is_empty() {
            cmd.arg("--settings");
            cmd.arg(Value::Object(settings).to_string());
        }
        if !request.cwd.is_empty() {
            cmd.current_dir(&request.cwd);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        cmd
    }

    /// Share the complete initialize response between model and command discovery.
    /// No user message is written; the short-lived child is retired after initialize.
    async fn initialize(&self) -> Result<Value, HarnessError> {
        self.initialize
            .get(
                || self.model_context().map(|c| c.unwrap().key()),
                || self.probe_initialize(None),
            )
            .await
    }

    async fn discover_commands(
        &self,
        cwd: Option<&std::path::Path>,
    ) -> Result<Vec<SlashCommand>, HarnessError> {
        let response = match cwd {
            Some(cwd) => self.probe_initialize(Some(cwd)).await?,
            None => self.initialize().await?,
        };
        Ok(parse_initialize_commands(&response))
    }

    async fn probe_initialize(&self, cwd: Option<&std::path::Path>) -> Result<Value, HarnessError> {
        let exe = self.resolve_executable()?;
        let mut cmd = Command::new(&exe);
        if let Some(cwd) = cwd {
            cmd.current_dir(cwd);
        }
        crate::compose_child_path(&mut cmd, &exe);
        cmd.args([
            "--print",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            // Mandatory with --print + stream-json output; without it the
            // CLI exits immediately with a usage error.
            "--verbose",
        ]);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                HarnessError::NotInstalled(crate::executable::binary_hint(&exe))
            } else {
                HarnessError::Io(e)
            }
        })?;
        let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            shutdown_child(&mut child, self.kill_grace).await;
            return Err(HarnessError::Protocol("claude child has no stdio".into()));
        };
        const PROBE_ID: &str = "zeron-command-probe";
        let discovery = async {
            let request = serde_json::json!({
                "type": "control_request",
                "request_id": PROBE_ID,
                "request": { "subtype": "initialize" },
            });
            stdin
                .write_all(format!("{request}\n").as_bytes())
                .await
                .map_err(HarnessError::Io)?;
            stdin.flush().await.map_err(HarnessError::Io)?;
            let mut lines = BufReader::new(stdout).lines();
            while let Some(line) = lines.next_line().await.map_err(HarnessError::Io)? {
                let Ok(frame) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if frame.get("type").and_then(Value::as_str) != Some("control_response") {
                    continue;
                }
                let response = frame.get("response").cloned().unwrap_or(Value::Null);
                if response.get("request_id").and_then(Value::as_str) != Some(PROBE_ID) {
                    continue;
                }
                if response.get("subtype").and_then(Value::as_str) == Some("error") {
                    let msg = response
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("initialize control request failed");
                    return Err(HarnessError::Protocol(msg.into()));
                }
                return Ok(response);
            }
            Err(HarnessError::Protocol(
                "claude exited before answering the initialize control request".into(),
            ))
        };
        let result = tokio::time::timeout(Duration::from_secs(10), discovery).await;
        shutdown_child(&mut child, self.kill_grace).await;
        match result {
            Ok(inner) => inner,
            Err(_) => Err(HarnessError::Protocol("Claude initialize timed out".into())),
        }
    }
}

/// `commands` out of an `initialize` control_response payload
/// (`response.response.commands`: name / description / argumentHint /
/// builtin). Entries without `builtin: true` are skills and user commands.
fn parse_initialize_commands(response: &Value) -> Vec<SlashCommand> {
    response
        .get("response")
        .and_then(|r| r.get("commands"))
        .and_then(Value::as_array)
        .map(|a| a.as_slice())
        .unwrap_or_default()
        .iter()
        .filter_map(|c| {
            let name = c.get("name").and_then(Value::as_str)?.trim();
            if name.is_empty() {
                return None;
            }
            Some(SlashCommand {
                name: name.to_owned(),
                description: c
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                input_hint: c
                    .get("argumentHint")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|h| !h.is_empty())
                    .map(str::to_owned),
                kind: if c.get("builtin").and_then(Value::as_bool) == Some(true) {
                    SlashCommandKind::BuiltIn
                } else {
                    SlashCommandKind::Skill
                },
            })
        })
        .collect()
}

#[async_trait]
impl Harness for ClaudeHarness {
    fn id(&self) -> HarnessId {
        HarnessId::ClaudeCode
    }
    fn display_name(&self) -> &str {
        "Claude Code"
    }
    fn supports_steering(&self) -> bool {
        true
    }
    fn live_reasoning(&self) -> bool {
        true
    }
    fn steering_mode(&self) -> SteeringMode {
        SteeringMode::StepBoundary
    }
    fn reasoning_levels(&self) -> &[ReasoningLevel] {
        &[
            ReasoningLevel::Low,
            ReasoningLevel::Medium,
            ReasoningLevel::High,
            ReasoningLevel::XHigh,
            ReasoningLevel::Max,
        ]
    }
    fn installed(&self) -> bool {
        // The launch resolver, not bare discovery: a valid CLAUDE_CODE_EXECUTABLE
        // (or a test `executable`) must report installed, and an invalid one
        // must not — availability and launches share one resolution.
        self.resolve_executable().is_ok()
    }
    fn executable_path(&self) -> Option<PathBuf> {
        self.resolve_executable().ok()
    }
    /// Done is the CLI's own terminal frame, for wake turns too.
    fn deterministic_turn_end(&self) -> bool {
        true
    }
    /// The CLI compacts itself: `/compact` as prompt text, answered with a
    /// `system`/`compact_boundary` frame.
    fn branching(&self) -> bool {
        true
    }

    fn compaction(&self) -> Compaction {
        Compaction::SlashText("/compact".into())
    }

    /// Credential and executable identity scopes both initialize and catalog caches.
    fn model_context(&self) -> Result<Option<crate::ModelContext>, HarnessError> {
        crate::model_context::context(self.id(), &self.resolve_executable()?, &[]).map(Some)
    }
    fn fallback_models(&self) -> Vec<Model> {
        catalog::configured_models()
    }
    async fn model_catalog(&self, force: bool) -> Result<crate::ModelCatalog, HarnessError> {
        self.model_context()?.unwrap().log();
        self.models_cache
            .get_with_timeout(
                force,
                Duration::from_secs(35),
                || self.model_context().map(|c| c.unwrap().key()),
                || async {
                    let response = self.initialize().await?;
                    catalog::with_discovered_models(catalog::configured_models(), &response)
                },
            )
            .await
    }
    async fn models(&self) -> Result<Vec<Model>, HarnessError> {
        self.resolve_executable()?;
        match self.model_catalog(false).await {
            Ok(catalog) => Ok(catalog.models),
            Err(error) => {
                tracing::warn!(%error, source = "static", "Claude model discovery failed");
                Ok(self.fallback_models())
            }
        }
    }

    /// Slash commands from the CLI's `initialize` control-request handshake —
    /// the same channel the Claude Agent SDK's `query()` opens. The response
    /// carries every command with description + argument hint and involves no
    /// model turn (verified live, 2.1.228: the control_response is the first
    /// stdout line, well before any API traffic). Cached on success.
    async fn skills(
        &self,
        cwd: &std::path::Path,
    ) -> Result<Option<Vec<zeron_proto::invocation::Skill>>, HarnessError> {
        let (skills, commands) = tokio::try_join!(
            crate::skills::discover(self.id(), cwd),
            self.workspace_commands
                .get(cwd, self.discover_commands(Some(cwd)))
        )?;
        // The native advertised catalog controls availability (including plugin
        // enablement and skillOverrides). Shared Agent Skills can use file delivery.
        Ok(Some(
            skills
                .into_iter()
                .filter_map(|mut skill| {
                    if crate::skills::is_shared_skill(&skill.path) {
                        // Shared files are not Claude command definitions. A
                        // same-named built-in must not replace their identity.
                        Some(skill)
                    } else if zeron_proto::invocation::valid_skill_command_name(&skill.name)
                        && commands.iter().any(|command| command.name == skill.name)
                    {
                        skill.command = Some(zeron_proto::invocation::SkillCommand {
                            name: skill.name.clone(),
                            harness: self.id(),
                        });
                        Some(skill)
                    } else {
                        None
                    }
                })
                .collect(),
        ))
    }

    async fn commands(&self) -> Result<Vec<SlashCommand>, HarnessError> {
        self.discover_commands(None).await
    }

    async fn commands_for(&self, cwd: &std::path::Path) -> Result<Vec<SlashCommand>, HarnessError> {
        self.workspace_commands
            .get(cwd, self.discover_commands(Some(cwd)))
            .await
    }

    async fn run(
        &self,
        request: RunRequest,
        controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        self.run_with_mode(request, controls, false).await
    }

    async fn run_title(
        &self,
        mut request: RunRequest,
        controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        request.resume = None;
        request.worktree = None;
        request.attachments.clear();
        request.mcp = None;
        request.model_options.clear();
        request.auto_approve = false;
        self.run_with_mode(request, controls, true).await
    }
}

impl ClaudeHarness {
    async fn run_with_mode(
        &self,
        request: RunRequest,
        controls: RunControls,
        title_only: bool,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        let exe = self.resolve_executable()?;
        let mut cmd = self.build_command(&exe, &request);
        if title_only {
            cmd.args([
                "--system-prompt",
                crate::TITLE_INSTRUCTIONS,
                "--tools",
                "",
                "--strict-mcp-config",
                "--mcp-config",
                "{\"mcpServers\":{}}",
                "--setting-sources",
                "",
            ]);
        } else if let Some(server) = &controls.tool_server {
            // Additive on purpose: `--strict-mcp-config` here would drop the
            // user's own MCP servers. The CLI kills any MCP call after
            // MCP_TOOL_TIMEOUT (default 5 min, cap 30 min; read from 2.1.274);
            // wait_threads blocks up to 30 min, so raise it to the cap.
            cmd.args(["--mcp-config", &server.mcp_config_json("zeron")]);
            cmd.env("MCP_TOOL_TIMEOUT", "1800000");
            if let Some(guidance) = &server.guidance {
                cmd.args(["--append-system-prompt", guidance]);
            }
        }
        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                HarnessError::NotInstalled(crate::executable::binary_hint(&exe))
            } else {
                HarnessError::Io(e)
            }
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| HarnessError::Protocol("claude child has no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| HarnessError::Protocol("claude child has no stdout".into()))?;
        let stderr_tail = crate::StderrTail::default();
        if let Some(stderr) = child.stderr.take() {
            let tail = stderr_tail.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!(target: "zeron_harness::claude", "stderr: {line}");
                    tail.push(&line);
                }
            });
        }

        let (stdin_tx, stdin_rx) = mpsc::unbounded_channel::<StdinMsg>();
        tokio::spawn(stdin_writer(stdin, stdin_rx));

        // The initial prompt as the first stdin user line (streaming-input
        // mode). Ultrathink rides every user message — steers included.
        // Staged image attachments are inlined as base64 image content blocks
        // ahead of the text (verified against the real CLI); their path refs
        // also ride the prompt text, so a skipped/unreadable file degrades to
        // the old-app behavior (the agent opens the path with its Read tool).
        let images = load_image_blocks(&request.attachments).await;
        let skills = if title_only {
            Vec::new()
        } else {
            self.skill_names(&request.prompt).await
        };
        let first = wire::user_message_line_with_images(
            &apply_ultrathink(
                request.reasoning,
                &crate::skill_mentions_as_slash(&request.prompt, &skills),
            ),
            &images,
        );
        let _ = stdin_tx.send(StdinMsg::Line(first));

        let (event_tx, event_rx) = mpsc::channel::<Result<AgentEvent, HarnessError>>(256);
        tokio::spawn(run_session(Session {
            title_only,
            permission_mode: request.permission_mode,
            computer_control: request.computer_control,
            child,
            stdout_lines: BufReader::new(stdout).lines(),
            stdin_tx,
            event_tx,
            controls,
            reasoning: request.reasoning,
            model: request.model.clone(),
            skills,
            interrupt_grace: self.interrupt_grace,
            kill_grace: self.kill_grace,
            stderr_tail,
        }));

        Ok(futures::stream::unfold(event_rx, |mut rx| async move {
            rx.recv().await.map(|ev| (ev, rx))
        })
        .boxed())
    }
}

enum StdinMsg {
    Line(String),
    /// Close stdin (end of steering input): the CLI finishes the current turn
    /// and exits, which ends the run stream at stdout EOF.
    Close,
}

/// Anthropic's API caps inline images at 5MB of raw bytes; larger files stay
/// path refs only.
const MAX_INLINE_IMAGE_BYTES: u64 = 5 * 1024 * 1024;

/// Media type for an inline image block — extension first, magic bytes as the
/// fallback (pasted screenshots may carry odd names). Only the API-supported
/// inline types map; anything else (svg/bmp/tiff/…) returns `None`.
fn image_media_type(path: &std::path::Path, bytes: &[u8]) -> Option<&'static str> {
    let by_ext = match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        _ => None,
    };
    by_ext.or(match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [0xFF, 0xD8, 0xFF, ..] => Some("image/jpeg"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'E',
            b'B',
            b'P',
            ..,
        ] => Some("image/webp"),
        _ => None,
    })
}

/// Load `RunRequest::attachments` into inline image blocks, best-effort: an
/// unreadable, oversized, or unsupported file is skipped — its path ref still
/// rides the prompt text — never fatal to the run.
async fn load_image_blocks(paths: &[String]) -> Vec<wire::ImageBlock> {
    use base64::Engine as _;
    let mut blocks = Vec::new();
    for path in paths {
        let bytes = match tokio::fs::read(path).await {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::warn!(target: "zeron_harness::claude", %path, error = %err, "attachment unreadable; path ref only");
                continue;
            }
        };
        if bytes.len() as u64 > MAX_INLINE_IMAGE_BYTES {
            tracing::debug!(target: "zeron_harness::claude", %path, "attachment over inline cap; path ref only");
            continue;
        }
        let Some(media_type) = image_media_type(std::path::Path::new(path), &bytes) else {
            tracing::debug!(target: "zeron_harness::claude", %path, "attachment not an inline-supported image; path ref only");
            continue;
        };
        blocks.push(wire::ImageBlock {
            media_type: media_type.to_string(),
            data: base64::engine::general_purpose::STANDARD.encode(&bytes),
        });
    }
    blocks
}

/// Owns the child's stdin; a write failure (EPIPE after the child died) is
/// tolerated and logged.
async fn stdin_writer(mut stdin: ChildStdin, mut rx: mpsc::UnboundedReceiver<StdinMsg>) {
    while let Some(msg) = rx.recv().await {
        match msg {
            StdinMsg::Line(line) => {
                let write = async {
                    stdin.write_all(line.as_bytes()).await?;
                    stdin.write_all(b"\n").await?;
                    stdin.flush().await
                };
                if let Err(e) = write.await {
                    tracing::debug!(target: "zeron_harness::claude", "stdin write failed (tolerated): {e}");
                    return;
                }
            }
            StdinMsg::Close => {
                let _ = stdin.shutdown().await;
                return;
            }
        }
    }
}

struct Session {
    title_only: bool,
    permission_mode: zeron_proto::PermissionMode,
    computer_control: bool,
    child: Child,
    stdout_lines: tokio::io::Lines<BufReader<crate::process::ChildStdout>>,
    stdin_tx: mpsc::UnboundedSender<StdinMsg>,
    event_tx: mpsc::Sender<Result<AgentEvent, HarnessError>>,
    controls: RunControls,
    reasoning: Option<ReasoningLevel>,
    model: Option<String>,
    /// Skill names the `$name` rewrite recognizes, for steers too.
    skills: Vec<String>,
    interrupt_grace: Duration,
    kill_grace: Duration,
    /// Rolling stderr tail for the crash message on an unexpected exit.
    stderr_tail: crate::StderrTail,
}

/// The per-run event loop: one task multiplexing stdout frames, the steering
/// mailbox, the interrupt token, and consumer liveness.
async fn run_session(session: Session) {
    let Session {
        title_only,
        permission_mode,
        computer_control,
        mut child,
        mut stdout_lines,
        stdin_tx,
        event_tx,
        controls,
        mut reasoning,
        model,
        skills,
        interrupt_grace,
        kill_grace,
        stderr_tail,
    } = session;
    let RunControls {
        execution_lease: _execution_lease,
        request_input,
        mut steering,
        interrupt,
        stop_tasks,
        ..
    } = controls;
    let request_input = Arc::new(request_input);
    let mut stop_tasks = stop_tasks;
    let mut stop_seq: u64 = 0;

    let mut norm = Normalizer::new();
    let mut pending_steers = std::collections::VecDeque::new();
    // Top-level tool calls in flight: a steer must not abort them (see
    // `wire::steer_message_line`).
    let mut open_tools = std::collections::HashSet::new();
    let mut steering_open = true;
    let mut interrupted = false;
    let mut interrupt_sent = false;
    let mut any_done = false;
    // A turn end held back while steers wait for their replay. Rapid `now`
    // steers each interrupt the turn the previous one started, and the CLI
    // replays only the last (verified on 2.1.280; the earlier texts still
    // reach the model). If nothing follows the held result, the steers were
    // absorbed: release them and the turn end instead of spinning forever.
    const HELD_DONE_SETTLE: Duration = Duration::from_secs(5);
    let mut held_done: Option<(AgentEvent, tokio::time::Instant)> = None;
    let mut done_after_interrupt = false;
    let mut escalation: Option<tokio::task::JoinHandle<()>> = None;

    'main: loop {
        tokio::select! {
            line = stdout_lines.next_line() => match line {
                Ok(Some(line)) => {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    // The CLI is still producing: whatever it is doing is not
                    // the quiet end the held turn end waits for.
                    if let Some((_, deadline)) = held_done.as_mut() {
                        *deadline = tokio::time::Instant::now() + HELD_DONE_SETTLE;
                    }
                    let frame = match wire::parse_frame(line) {
                        Ok(frame) => frame,
                        Err(e) => {
                            tracing::debug!(target: "zeron_harness::claude", "unparseable frame (skipped): {e}");
                            continue;
                        }
                    };
                    if let Frame::ControlRequest(req) = frame {
                        if title_only {
                            let line = control_response_line(&req.request_id, serde_json::json!({
                                "behavior": "deny", "message": "Tools are disabled for title generation"
                            }));
                            let _ = stdin_tx.send(StdinMsg::Line(line));
                        } else {
                            handle_control_request(
                                req,
                                permission_mode,
                                computer_control,
                                &request_input,
                                &stdin_tx,
                            );
                        }
                        continue;
                    }
                    // Only the CLI's replay confirms that a prompt joined its
                    // conversation. Writing stdin must not split ongoing text.
                    if let Frame::User(ref user) = frame {
                        // A replay confirms its steer and every earlier one:
                        // superseded steers are never replayed themselves.
                        if user.parent_tool_use_id.is_none()
                            && let Some(at) = user
                                .uuid
                                .as_ref()
                                .and_then(|id| pending_steers.iter().position(|p| p == id))
                        {
                            for _ in 0..=at {
                                pending_steers.pop_front();
                                let (prev, next) = norm.rotate_for_steer();
                                if event_tx.send(Ok(AgentEvent::Steered {
                                    assistant_message_id: Some(prev), next_assistant_message_id: Some(next),
                                })).await.is_err() { break 'main; }
                            }
                        }
                    }
                    for ev in norm.normalize(frame, interrupted) {
                        match &ev {
                            AgentEvent::ToolCall { id, .. } => {
                                open_tools.insert(id.clone());
                            }
                            AgentEvent::ToolResult { id, .. } => {
                                open_tools.remove(id);
                            }
                            AgentEvent::Done { .. } => open_tools.clear(),
                            _ => {}
                        }
                        let is_done = matches!(ev, AgentEvent::Done { .. });
                        // A `now` steer ends the turn it interrupts with a
                        // result frame; the steer continues the run, so that
                        // result is a steer boundary, not the end of the turn.
                        if is_done && !interrupted && !pending_steers.is_empty() {
                            held_done =
                                Some((ev, tokio::time::Instant::now() + HELD_DONE_SETTLE));
                            continue;
                        }
                        if is_done {
                            held_done = None;
                        }
                        if event_tx.send(Ok(ev)).await.is_err() {
                            break 'main; // consumer gone — reap below
                        }
                        if is_done {
                            any_done = true;
                            if interrupted {
                                done_after_interrupt = true;
                                break 'main;
                            }
                        }
                    }
                }
                Ok(None) => break 'main, // stdout EOF: the CLI exited
                Err(e) => {
                    let _ = event_tx.send(Err(HarnessError::Io(e))).await;
                    break 'main;
                }
            },

            stop = async {
                match stop_tasks.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            }, if stop_tasks.is_some() && !interrupted => match stop {
                Some(id) => {
                    let task = norm.agent_task_for_spawn(&id).unwrap_or(id);
                    stop_seq += 1;
                    let line = wire::stop_task_request_line(&format!("zeron-stop-{stop_seq}"), &task);
                    let _ = stdin_tx.send(StdinMsg::Line(line));
                }
                None => stop_tasks = None,
            },

            steer = steering.recv(), if steering_open && !interrupted => match steer {
                Some(msg) => {
                    if let Some(level) = msg.reasoning.filter(|level| Some(*level) != reasoning) {
                        stop_seq += 1;
                        let line = wire::effort_request_line(
                            &format!("zeron-effort-{stop_seq}"),
                            to_effort(Some(level), model.as_deref()),
                            level == ReasoningLevel::Ultracode,
                        );
                        let _ = stdin_tx.send(StdinMsg::Line(line));
                        reasoning = Some(level);
                    }
                    let images = load_image_blocks(&msg.attachments).await;
                    let id = uuid::Uuid::new_v4().to_string();
                    let line = wire::steer_message_line(
                        &apply_ultrathink(
                            reasoning,
                            &crate::skill_mentions_as_slash(&msg.prompt, &skills),
                        ),
                        &images,
                        &id,
                        open_tools.is_empty(),
                    );
                    pending_steers.push_back(id);
                    if stdin_tx.send(StdinMsg::Line(line)).is_err() {
                        break 'main;
                    }
                }
                None => {
                    // Mailbox closed: end the input so the run can finish
                    // after the current turn.
                    steering_open = false;
                    let _ = stdin_tx.send(StdinMsg::Close);
                }
            },

            _ = interrupt.cancelled(), if !interrupt_sent => {
                interrupt_sent = true;
                interrupted = true;
                let _ = stdin_tx.send(StdinMsg::Line(wire::interrupt_request_line("int_1")));
                // Escalate if the CLI doesn't wind down within the grace
                // periods: SIGTERM (kills bash trees, runs SessionEnd hooks),
                // then SIGKILL. Aborted once the child is reaped.
                if let Some(pid) = crate::process::signal_target(&child) {
                    escalation = Some(tokio::spawn(async move {
                        tokio::time::sleep(interrupt_grace).await;
                        send_signal(&pid, Signal::Term);
                        tokio::time::sleep(kill_grace).await;
                        send_signal(&pid, Signal::Kill);
                    }));
                }
            },

            _ = tokio::time::sleep_until(
                held_done.as_ref().map_or_else(tokio::time::Instant::now, |(_, d)| *d)
            ), if held_done.is_some() => {
                // The steers were absorbed into the turn that just ended.
                while pending_steers.pop_front().is_some() {
                    let (prev, next) = norm.rotate_for_steer();
                    if event_tx.send(Ok(AgentEvent::Steered {
                        assistant_message_id: Some(prev), next_assistant_message_id: Some(next),
                    })).await.is_err() { break 'main; }
                }
                let (done, _) = held_done.take().expect("guarded by if");
                if event_tx.send(Ok(done)).await.is_err() {
                    break 'main;
                }
                any_done = true;
            },

            _ = event_tx.closed() => break 'main,
        }
    }

    // A turn end still held when the CLI exited is the run's real end.
    if let Some((done, _)) = held_done.take()
        && !event_tx.is_closed()
        && event_tx.send(Ok(done)).await.is_ok()
    {
        any_done = true;
    }

    // Terminal bookkeeping: never end the stream without a Done unless the
    // consumer already hung up.
    if !event_tx.is_closed() {
        if interrupted && !done_after_interrupt {
            let _ = event_tx
                .send(Ok(AgentEvent::Done {
                    status: DoneStatus::Interrupted,
                    result: None,
                    error: None,
                    session_id: norm.session_id.clone(),
                }))
                .await;
        } else if !interrupted && !any_done {
            let status = child.try_wait().ok().flatten();
            let error = norm
                .pending_error
                .take()
                .unwrap_or_else(|| crate::crash_message("claude", status, &stderr_tail));
            let _ = event_tx
                .send(Ok(AgentEvent::Done {
                    status: DoneStatus::Errored,
                    result: None,
                    error: Some(error),
                    session_id: norm.session_id.clone(),
                }))
                .await;
        }
    }

    shutdown_child(&mut child, kill_grace).await;
    if let Some(handle) = escalation {
        handle.abort();
    }
}

type RequestInputFn = Box<
    dyn Fn(Vec<UserInputQuestion>) -> tokio::sync::oneshot::Receiver<Vec<UserInputAnswer>>
        + Send
        + Sync,
>;

/// Serve one `can_use_tool` control request. The CLI blocks until SOME
/// response arrives, so every request must be answered. Under Full every
/// tool is auto-approved (the unattended behavior); under Ask/Auto a tool
/// request becomes an Allow/Deny question for the user. `AskUserQuestion`
/// is intercepted in every mode — surface the questions through the
/// engine's input bridge (which owns the `InputRequested`/`InputResolved`
/// lifecycle), wait for the user's answers (in a subtask so the frame loop
/// keeps flowing), and hand them back keyed by question text, as the tool
/// expects.
fn handle_control_request(
    req: ControlRequestFrame,
    permission_mode: zeron_proto::PermissionMode,
    computer_control: bool,
    request_input: &Arc<RequestInputFn>,
    stdin_tx: &mpsc::UnboundedSender<StdinMsg>,
) {
    if req.request.subtype != "can_use_tool" {
        tracing::debug!(
            target: "zeron_harness::claude",
            "unhandled control_request subtype: {}", req.request.subtype
        );
        return;
    }
    if req.request.tool_name != "AskUserQuestion" {
        // The engine's consent card is the only prompt a Zeron computer call gets.
        let computer_call =
            computer_control && crate::is_zeron_computer_tool(&req.request.tool_name);
        if computer_call || permission_mode == zeron_proto::PermissionMode::Full {
            let line = control_response_line(&req.request_id, allow_response(req.request.input));
            let _ = stdin_tx.send(StdinMsg::Line(line));
            return;
        }
        let request_input = Arc::clone(request_input);
        let stdin_tx = stdin_tx.clone();
        tokio::spawn(async move {
            let question = permission_question(&req.request.tool_name, &req.request.input);
            // A dropped sender (caller went away) degrades to a deny so the
            // agent is unblocked — never silently allowed.
            let answers = (request_input)(vec![question.clone()])
                .await
                .unwrap_or_default();
            let allowed = answers.iter().any(|a| {
                a.question_id == question.id
                    && a.labels.iter().any(|l| l.eq_ignore_ascii_case("allow"))
            });
            let response = if allowed {
                allow_response(req.request.input)
            } else {
                wire::deny_response("The user denied this tool use.")
            };
            let line = control_response_line(&req.request_id, response);
            let _ = stdin_tx.send(StdinMsg::Line(line));
        });
        return;
    }
    let request_input = Arc::clone(request_input);
    let stdin_tx = stdin_tx.clone();
    tokio::spawn(async move {
        let request_id = req.request_id;
        let input = req.request.input;
        let questions = parse_questions(&input);
        // The engine's input bridge is the SOLE emitter of
        // `InputRequested`/`InputResolved`: it mints the request id, parks the
        // resolver for `respond_input`, and surfaces both events. Emitting our
        // own copy here (keyed by Claude's control-request id) folded a SECOND
        // input part into the doc whose id no resolver knew — the QuestionPanel
        // answered that unanswerable twin and the run never resumed.
        //
        // A dropped sender (caller went away) degrades to empty answers so the
        // agent is unblocked rather than wedged.
        let answers = (request_input)(questions.clone()).await.unwrap_or_default();
        let updated = updated_input_with_answers(&input, &questions, &answers);
        let line = control_response_line(&request_id, allow_response(updated));
        let _ = stdin_tx.send(StdinMsg::Line(line));
    });
}

/// The Allow/Deny question a tool permission becomes under Ask/Auto: the
/// command for Bash, the path for file tools, the raw input otherwise.
fn permission_question(tool_name: &str, input: &Value) -> UserInputQuestion {
    let detail = input
        .get("command")
        .or_else(|| input.get("file_path"))
        .or_else(|| input.get("path"))
        .or_else(|| input.get("url"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| input.to_string());
    UserInputQuestion {
        id: uuid::Uuid::new_v4().to_string(),
        header: format!("Allow {tool_name}?"),
        question: format!("Claude wants to use {tool_name}: {detail}"),
        options: vec!["Allow".into(), "Deny".into()],
        multi_select: false,
        kind: None,
    }
}

/// Parse Claude's `AskUserQuestion` tool input into [`UserInputQuestion`]s
/// (tolerant of `header`/`title`, `question`/`prompt`, string or object
/// options — option descriptions are dropped, the wire type carries labels).
fn parse_questions(input: &Value) -> Vec<UserInputQuestion> {
    let raw = input.get("questions").and_then(Value::as_array);
    raw.map(|a| a.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|q| {
            let field =
                |keys: [&str; 2]| keys.iter().find_map(|k| q.get(*k).and_then(Value::as_str));
            UserInputQuestion {
                id: uuid::Uuid::new_v4().to_string(),
                header: field(["header", "title"]).unwrap_or("Question").into(),
                question: field(["question", "prompt"]).unwrap_or("").into(),
                multi_select: ["multiSelect", "multi_select"]
                    .iter()
                    .find_map(|k| q.get(*k).and_then(Value::as_bool))
                    .unwrap_or(false),
                options: q
                    .get("options")
                    .and_then(Value::as_array)
                    .map(|a| a.as_slice())
                    .unwrap_or_default()
                    .iter()
                    .map(|op| match op {
                        Value::String(s) => s.clone(),
                        other => other
                            .get("label")
                            .or_else(|| other.get("value"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .into(),
                    })
                    .collect(),
                kind: None,
            }
        })
        .collect()
}

/// Merge the user's answers back into the tool input, keyed by question text
/// (single-select ⇒ a string, multi-select ⇒ an array), as the tool expects.
fn updated_input_with_answers(
    input: &Value,
    questions: &[UserInputQuestion],
    answers: &[UserInputAnswer],
) -> Value {
    let mut updated = match input {
        Value::Object(map) => map.clone(),
        _ => serde_json::Map::new(),
    };
    let mut by_question = serde_json::Map::new();
    for q in questions {
        let labels: Vec<String> = answers
            .iter()
            .find(|a| a.question_id == q.id)
            .map(|a| a.labels.clone())
            .unwrap_or_default();
        let value = if q.multi_select {
            Value::Array(labels.into_iter().map(Value::String).collect())
        } else {
            Value::String(labels.into_iter().next().unwrap_or_default())
        };
        by_question.insert(q.question.clone(), value);
    }
    updated.insert("answers".into(), Value::Object(by_question));
    Value::Object(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn initialize_commands_split_builtins_from_skills() {
        let response = json!({ "response": { "commands": [
            { "name": "compact", "description": "Free up context", "argumentHint": "<focus>", "builtin": true },
            { "name": "maestro-work", "description": "Implement one unit (user)", "argumentHint": "" },
            { "name": "codex:review", "description": "(codex) Review" },
        ]}});
        let commands = parse_initialize_commands(&response);
        let kinds: Vec<(&str, SlashCommandKind)> =
            commands.iter().map(|c| (c.name.as_str(), c.kind)).collect();
        assert_eq!(
            kinds,
            vec![
                ("compact", SlashCommandKind::BuiltIn),
                ("maestro-work", SlashCommandKind::Skill),
                ("codex:review", SlashCommandKind::Skill),
            ]
        );
        assert_eq!(commands[0].input_hint.as_deref(), Some("<focus>"));
        assert_eq!(commands[1].input_hint, None);
    }

    // The Windows `Command` wrapper has no `as_std` to read the built line back.
    #[cfg(not(windows))]
    #[test]
    fn context_window_pick_reaches_the_cli() {
        let harness = ClaudeHarness::new();
        let exe = PathBuf::from("/bin/claude");
        let build = |choice: Option<&str>| {
            let mut request = RunRequest {
                prompt: String::new(),
                harness: None,
                model: Some("claude-fable-5-1".into()),
                reasoning: None,
                model_options: serde_json::Map::new(),
                cwd: String::new(),
                sandbox: zeron_proto::SandboxLevel::WorkspaceWrite,
                permission_mode: zeron_proto::PermissionMode::Full,
                auto_approve: false,
                resume: None,
                attachments: Vec::new(),
                worktree: None,
                mcp: None,
                branch: None,
                forked_at: None,
                computer_control: false,
            };
            if let Some(choice) = choice {
                request
                    .model_options
                    .insert("contextWindow".into(), json!(choice));
            }
            let cmd = harness.build_command(&exe, &request);
            let std = cmd.as_std();
            let args: Vec<String> = std
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            let model = args
                .iter()
                .position(|a| a == "--model")
                .map(|i| args[i + 1].clone());
            let disabled_1m = std
                .get_envs()
                .any(|(k, v)| k == "CLAUDE_CODE_DISABLE_1M_CONTEXT" && v.is_some());
            (model, disabled_1m)
        };
        assert_eq!(
            build(Some("1m")),
            (Some("claude-fable-5-1[1m]".into()), false)
        );
        assert_eq!(build(Some("200k")), (Some("claude-fable-5-1".into()), true));
        assert_eq!(build(None), (Some("claude-fable-5-1".into()), false));
    }

    #[cfg(not(windows))]
    #[test]
    fn permission_mode_selects_the_cli_flags() {
        use zeron_proto::PermissionMode;
        let harness = ClaudeHarness::new();
        let exe = PathBuf::from("/bin/claude");
        let args = |mode: PermissionMode| -> Vec<String> {
            let request = RunRequest {
                prompt: String::new(),
                harness: None,
                model: None,
                reasoning: None,
                model_options: serde_json::Map::new(),
                cwd: String::new(),
                sandbox: mode.sandbox(),
                permission_mode: mode,
                auto_approve: false,
                resume: None,
                attachments: Vec::new(),
                worktree: None,
                mcp: None,
                branch: None,
                forked_at: None,
                computer_control: false,
            };
            harness
                .build_command(&exe, &request)
                .as_std()
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect()
        };
        let mode_of = |args: &[String]| {
            args.iter()
                .position(|a| a == "--permission-mode")
                .map(|i| args[i + 1].clone())
        };
        let ask = args(PermissionMode::Ask);
        assert_eq!(mode_of(&ask).as_deref(), Some("default"));
        assert!(!ask.iter().any(|a| a == "--dangerously-skip-permissions"));
        let auto = args(PermissionMode::Auto);
        assert_eq!(mode_of(&auto).as_deref(), Some("acceptEdits"));
        assert!(!auto.iter().any(|a| a == "--dangerously-skip-permissions"));
        // Full is the pre-mode wire: `default` plus the handler's auto-allow.
        let full = args(PermissionMode::Full);
        assert_eq!(mode_of(&full).as_deref(), Some("default"));
        assert!(!full.iter().any(|a| a == "--dangerously-skip-permissions"));
    }

    #[tokio::test]
    async fn zeron_computer_tools_skip_the_harness_prompt_in_every_mode() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use zeron_proto::PermissionMode;
        let asked = Arc::new(AtomicUsize::new(0));
        let request_input: Arc<RequestInputFn> = {
            let asked = Arc::clone(&asked);
            Arc::new(Box::new(move |_questions| {
                asked.fetch_add(1, Ordering::SeqCst);
                let (tx, rx) = tokio::sync::oneshot::channel();
                let _ = tx.send(Vec::new());
                rx
            }))
        };
        let frame = |tool: &str| ControlRequestFrame {
            request_id: "r1".into(),
            request: wire::ControlRequestBody {
                subtype: "can_use_tool".into(),
                tool_name: tool.into(),
                input: json!({ "window_id": 7 }),
            },
        };
        let answer = |tool: &str, mode: PermissionMode, computer_control: bool| {
            let (tx, mut rx) = mpsc::unbounded_channel();
            handle_control_request(frame(tool), mode, computer_control, &request_input, &tx);
            async move {
                match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
                    Ok(Some(StdinMsg::Line(line))) => line,
                    _ => panic!("no control response"),
                }
            }
        };
        for mode in [
            PermissionMode::Ask,
            PermissionMode::Auto,
            PermissionMode::Full,
        ] {
            for tool in [
                "mcp__zeron__computer_click",
                "mcp__zeron__computer_browser_navigate",
                "mcp__zeron__computer_spaces",
            ] {
                let line = answer(tool, mode, true).await;
                assert!(
                    line.contains(r#""behavior":"allow""#),
                    "{mode:?} {tool}: {line}"
                );
                assert!(line.contains(r#""window_id":7"#), "{line}");
            }
        }
        assert_eq!(asked.load(Ordering::SeqCst), 0);
        for (tool, computer_control) in [
            ("Bash", true),
            ("mcp__zeron__list_threads", true),
            ("mcp__other__computer_click", true),
            ("mcp__zeron__computer_click", false),
        ] {
            for mode in [PermissionMode::Ask, PermissionMode::Auto] {
                let line = answer(tool, mode, computer_control).await;
                assert!(
                    line.contains(r#""behavior":"deny""#),
                    "{mode:?} {tool}: {line}"
                );
            }
        }
        assert_eq!(asked.load(Ordering::SeqCst), 8);
        let line = answer("Bash", PermissionMode::Full, true).await;
        assert!(line.contains(r#""behavior":"allow""#));
        assert_eq!(asked.load(Ordering::SeqCst), 8);
    }

    #[test]
    fn permission_question_names_the_command_or_path() {
        let q = permission_question(
            "Bash",
            &json!({"command": "rm -rf build", "description": "x"}),
        );
        assert_eq!(q.header, "Allow Bash?");
        assert!(q.question.contains("rm -rf build"));
        assert_eq!(q.options, vec!["Allow".to_string(), "Deny".to_string()]);
        let q = permission_question("Edit", &json!({"file_path": "/a/b.rs", "old_string": "x"}));
        assert!(q.question.contains("/a/b.rs"));
        let q = permission_question("WebSearch", &json!({"query": "zeron"}));
        assert!(q.question.contains("zeron"));
        assert_eq!(wire::deny_response("no")["behavior"], "deny");
    }

    #[cfg(unix)]
    #[test]
    fn a_branch_forks_the_resumed_session_at_the_kept_reply() {
        let harness = ClaudeHarness::new();
        let exe = PathBuf::from("/usr/bin/true");
        let mut request = RunRequest {
            prompt: "again".into(),
            harness: None,
            model: None,
            reasoning: None,
            model_options: serde_json::Map::new(),
            cwd: String::new(),
            sandbox: zeron_proto::SandboxLevel::WorkspaceWrite,
            permission_mode: zeron_proto::PermissionMode::Full,
            auto_approve: false,
            resume: Some("sess-a".into()),
            attachments: Vec::new(),
            worktree: None,
            mcp: None,
            branch: Some(zeron_proto::SessionBranch {
                keep: Some("uuid-reply-1".into()),
                drop: Some("uuid-prompt-2".into()),
            }),
            forked_at: None,
            computer_control: false,
        };
        let args = |cmd: &Command| -> Vec<String> {
            cmd.as_std()
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect()
        };
        let cmd = harness.build_command(&exe, &request);
        let argv = args(&cmd);
        for flag in [
            "--resume=sess-a",
            "--fork-session",
            "--resume-session-at=uuid-reply-1",
            "--resume-drops-turn=uuid-prompt-2",
            "--replay-user-messages",
        ] {
            assert!(
                argv.iter().any(|a| a == flag),
                "{flag} missing from {argv:?}"
            );
        }
        let env: Vec<(String, Option<String>)> = cmd
            .as_std()
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        assert!(
            env.contains(&(
                "CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING".to_string(),
                Some("true".to_string())
            )),
            "{env:?}"
        );

        assert!(!argv.iter().any(|a| a == "--session-id"));
        assert!(
            !env.iter()
                .any(|(k, _)| k == "CLAUDE_CODE_RESUME_SOURCE_ALIVE")
        );

        // No branch: a plain resume, never a fork.
        request.branch = None;
        let argv = args(&harness.build_command(&exe, &request));
        assert!(argv.iter().any(|a| a == "--resume=sess-a"));
        assert!(
            !argv
                .iter()
                .any(|a| a.starts_with("--fork") || a.starts_with("--resume-"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_forked_chat_keeps_the_source_s_background_agents_off_its_books() {
        let harness = ClaudeHarness::new();
        let exe = PathBuf::from("/usr/bin/true");
        let at = "2026-09-22T17:30:00.000Z";
        let mut request = RunRequest {
            prompt: "again".into(),
            harness: None,
            model: None,
            reasoning: None,
            model_options: serde_json::Map::new(),
            cwd: String::new(),
            sandbox: zeron_proto::SandboxLevel::WorkspaceWrite,
            permission_mode: zeron_proto::PermissionMode::Full,
            auto_approve: false,
            resume: Some("sess-source".into()),
            attachments: Vec::new(),
            worktree: None,
            mcp: None,
            branch: Some(zeron_proto::SessionBranch {
                keep: Some("uuid-reply-1".into()),
                drop: None,
            }),
            forked_at: Some(at.into()),
            computer_control: false,
        };
        let run = |request: &RunRequest| {
            let cmd = harness.build_command(&exe, request);
            let argv: Vec<String> = cmd
                .as_std()
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            let alive = cmd
                .as_std()
                .get_envs()
                .find(|(k, _)| *k == "CLAUDE_CODE_RESUME_SOURCE_ALIVE")
                .and_then(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()));
            (argv, alive)
        };

        let (argv, alive) = run(&request);
        let minted = argv
            .iter()
            .position(|a| a == "--session-id")
            .map(|i| argv[i + 1].clone())
            .expect("a branch of a fork names its session up front");
        assert!(uuid::Uuid::parse_str(&minted).is_ok());
        assert_ne!(minted, "sess-source");
        assert!(argv.iter().any(|a| a == "--fork-session"));
        assert!(argv.iter().any(|a| a == "--resume=sess-source"));
        assert_eq!(alive, Some(format!("{minted}|{at}")));
        let (again, _) = run(&request);
        assert_ne!(again, argv, "every branch run mints a fresh session id");

        request.branch = None;
        request.resume = Some("sess-fork".into());
        let (argv, alive) = run(&request);
        assert!(!argv.iter().any(|a| a == "--session-id"));
        assert_eq!(alive, Some(format!("sess-fork|{at}")));

        request.resume = None;
        let (argv, alive) = run(&request);
        assert!(!argv.iter().any(|a| a == "--session-id"));
        assert_eq!(alive, None);
    }

    #[test]
    fn parses_questions_tolerantly() {
        let input = json!({
            "questions": [
                {
                    "header": "Choice",
                    "question": "Pick one",
                    "options": ["A", {"label": "B", "description": "second"}],
                    "multiSelect": false
                },
                { "title": "Alt", "prompt": "Pick many", "multi_select": true }
            ]
        });
        let qs = parse_questions(&input);
        assert_eq!(qs.len(), 2);
        assert_eq!(qs[0].header, "Choice");
        assert_eq!(qs[0].options, vec!["A".to_string(), "B".to_string()]);
        assert!(!qs[0].multi_select);
        assert_eq!(qs[1].header, "Alt");
        assert_eq!(qs[1].question, "Pick many");
        assert!(qs[1].multi_select);
    }

    #[test]
    fn answers_key_by_question_text() {
        let input =
            json!({"questions": [{"header": "H", "question": "Pick one", "options": ["A", "B"]}]});
        let qs = parse_questions(&input);
        let answers = vec![UserInputAnswer {
            question_id: qs[0].id.clone(),
            labels: vec!["B".into()],
        }];
        let updated = updated_input_with_answers(&input, &qs, &answers);
        assert_eq!(updated["answers"]["Pick one"], json!("B"));
        // Original input is preserved alongside the answers.
        assert!(updated["questions"].is_array());
    }
}
