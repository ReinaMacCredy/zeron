//! Agent-side wire types: harness identity, run requests, streaming events, tool calls.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HarnessId {
    ClaudeCode,
    Codex,
    Cursor,
    /// Cognition's Devin agent, driven over ACP (`devin acp`).
    Devin,
    /// xAI's Grok Build agent, driven over ACP (`grok agent stdio`).
    Grok,
    /// Nous Research's Hermes Agent, driven over ACP (`hermes acp`).
    Hermes,
    /// The Pi coding agent (pi.dev), driven over its native JSONL RPC protocol.
    Pi,
    /// SST's opencode agent, driven natively over its own HTTP/SSE server
    /// protocol (`opencode serve` — the same wire the opencode desktop app
    /// speaks).
    Opencode,
    /// google's antigravity agent over acp (`agy_acp_server`, installed from
    /// its pinned release archive).
    Antigravity,
    /// Generic ACP agent driver; the provider instance names the agent.
    Acp,
    /// Meta's Muse Code agent, driven natively over the Muse Session Protocol
    /// (MSP: `muse` speaking JSON-RPC on stdio).
    Muse,
    /// Test harness; never shown in production pickers.
    Mock,
}

impl HarnessId {
    /// Every harness, in declaration order.
    pub const ALL: [HarnessId; 12] = [
        HarnessId::ClaudeCode,
        HarnessId::Codex,
        HarnessId::Cursor,
        HarnessId::Devin,
        HarnessId::Grok,
        HarnessId::Hermes,
        HarnessId::Pi,
        HarnessId::Opencode,
        HarnessId::Antigravity,
        HarnessId::Acp,
        HarnessId::Muse,
        HarnessId::Mock,
    ];

    /// The wire slug (`claude-code`, `codex`, ...) — what serde writes.
    pub fn as_str(self) -> &'static str {
        match self {
            HarnessId::ClaudeCode => "claude-code",
            HarnessId::Codex => "codex",
            HarnessId::Cursor => "cursor",
            HarnessId::Devin => "devin",
            HarnessId::Grok => "grok",
            HarnessId::Hermes => "hermes",
            HarnessId::Pi => "pi",
            HarnessId::Opencode => "opencode",
            HarnessId::Antigravity => "antigravity",
            HarnessId::Acp => "acp",
            HarnessId::Muse => "muse",
            HarnessId::Mock => "mock",
        }
    }

    /// Inverse of [`Self::as_str`]; `None` for a slug this build does not know.
    pub fn from_slug(slug: &str) -> Option<HarnessId> {
        Self::ALL.into_iter().find(|id| id.as_str() == slug)
    }

    pub fn display_name(self) -> &'static str {
        match self {
            HarnessId::ClaudeCode => "Claude Code",
            HarnessId::Codex => "Codex",
            HarnessId::Cursor => "Cursor",
            HarnessId::Devin => "Devin",
            HarnessId::Grok => "Grok",
            HarnessId::Hermes => "Hermes",
            HarnessId::Pi => "Pi",
            HarnessId::Opencode => "OpenCode",
            HarnessId::Antigravity => "Antigravity",
            HarnessId::Acp => "ACP agent",
            HarnessId::Muse => "Muse Code",
            HarnessId::Mock => "Mock",
        }
    }

    /// Whose brand mark an agent wears: the model's maker when the model id
    /// names one, else the harness running it.
    pub fn brand_of(harness: Option<HarnessId>, model: Option<&str>) -> Option<HarnessId> {
        let model = model
            .map(|m| m.split('[').next().unwrap_or(m).trim().to_ascii_lowercase())
            .unwrap_or_default();
        let (provider, id) = model.split_once('/').unwrap_or(("", &model));
        let maker = match provider {
            "anthropic" => Some(HarnessId::ClaudeCode),
            "openai" => Some(HarnessId::Codex),
            "xai" => Some(HarnessId::Grok),
            _ if id.starts_with("claude") => Some(HarnessId::ClaudeCode),
            _ if id.starts_with("gpt") || id.contains("codex") => Some(HarnessId::Codex),
            _ if id.starts_with("grok") => Some(HarnessId::Grok),
            _ => None,
        };
        maker.or(harness)
    }
}
/// Durable user preference for one agent's independently-installed CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HarnessUpdatePolicy {
    /// Check and surface an update, but never mutate the installation without
    /// an explicit action.
    #[default]
    Notify,
    /// Apply a discovered update after the harness execution gate becomes idle.
    AutoWhenIdle,
    /// Do not probe or update this harness.
    Off,
}

/// Best-effort classification of the installation that owns an agent CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HarnessInstallSource {
    Npm,
    Homebrew,
    Cargo,
    Vendor,
    ManagedByZeron,
    #[default]
    Unknown,
}

/// The externally visible harness-update state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HarnessUpdatePhase {
    Dormant,
    Checking,
    Current,
    Available,
    WaitingForIdle,
    Preparing,
    Downloading,
    Installing,
    Verifying,
    Updated,
    ManualActionRequired,
    Failed,
}

/// Real provider progress only. An absent value means the UI must render an
/// indeterminate activity indicator rather than inventing a percentage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessUpdateProgress {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessUpdateFailure {
    pub message: String,
    #[serde(default)]
    pub retryable: bool,
    /// What failed: looking for a release, or installing one. An engine that
    /// predates the field only ever reported failed checks.
    #[serde(default)]
    pub kind: HarnessUpdateFailureKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HarnessUpdateFailureKind {
    /// The version probe or release lookup failed; checking again retries.
    #[default]
    Check,
    /// The updater or its verification failed; the release is still known,
    /// so the update itself can be retried.
    Install,
}

/// One row in the device-local harness-update stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessUpdateStatus {
    pub harness: HarnessId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default)]
    pub source: HarnessInstallSource,
    #[serde(default)]
    pub policy: HarnessUpdatePolicy,
    pub phase: HarnessUpdatePhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<HarnessUpdateProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<HarnessUpdateFailure>,
    /// Whether this device can safely apply the update without guessing which
    /// package manager owns the installation. Older engines omit this field,
    /// so clients must default to the conservative read-only behavior.
    #[serde(default)]
    pub can_apply: bool,
    /// Provider command shown when mutation cannot be safely automated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_command: Option<String>,
    /// The installed release is known not to work with (or isn't supported
    /// by) this engine's zeron.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<HarnessCompatibility>,
    /// A newer release exists, but zeron won't install it: it is known not
    /// to work. Such a release is never reported as available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held_back: Option<HarnessCompatibility>,
}

/// How well a CLI release works with this zeron (see the engine's
/// compatibility policy).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HarnessCompatibilityStatus {
    Supported,
    /// Outside the releases this zeron was built and tested against.
    Unsupported,
    /// Known not to work with this zeron.
    Broken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessCompatibility {
    pub status: HarnessCompatibilityStatus,
    /// The release this is about.
    pub version: String,
    /// One line for the user, naming the release to use instead when known.
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended_version: Option<String>,
}

/// What the resume button and every automatic continuation send. A user
/// entry carrying exactly this text renders as a "Resumed" divider.
pub const RESUME_PROMPT: &str = "Resume without asking to continue.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitResume {
    pub chat_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<i64>,
    pub auto: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offer: Option<LimitOffer>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OfferChoice {
    Switch,
    Wait,
}

impl OfferChoice {
    pub fn wire(self) -> &'static str {
        match self {
            Self::Switch => "switch",
            Self::Wait => "wait",
        }
    }

    pub fn act_label(self) -> &'static str {
        match self {
            Self::Switch => "Switch now",
            Self::Wait => "Wait now",
        }
    }

    pub fn countdown_verb(self) -> &'static str {
        match self {
            Self::Switch => "Switching",
            Self::Wait => "Waiting",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitOffer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<crate::RouteTarget>,
    pub preselect: OfferChoice,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub act_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitResumes {
    pub auto_resume: bool,
    #[serde(default)]
    pub chats: Vec<LimitResume>,
}

impl HarnessUpdateStatus {
    /// A provider's version line: the installed version, plus only what the
    /// Update button doesn't already say (progress, a failure). Up-to-date
    /// providers, checks in flight and providers whose updates aren't
    /// monitored show just the version, so rows don't flash through states
    /// while checks run.
    pub fn version_line(&self) -> String {
        if let Some(compatibility) = self.blocking_compatibility() {
            return compatibility.message.clone();
        }
        let installed = self
            .installed_version
            .as_deref()
            .map(|version| format!("v{version}"));
        let with = |detail: &str| match &installed {
            Some(version) => format!("{version} · {detail}"),
            None => detail.to_owned(),
        };
        let version_only = || installed.clone().unwrap_or_default();
        if self.phase == HarnessUpdatePhase::Current
            && let Some(held) = &self.held_back
        {
            return with(&format!("v{} held back: not supported yet", held.version));
        }
        match self.phase {
            HarnessUpdatePhase::Dormant
            | HarnessUpdatePhase::Checking
            | HarnessUpdatePhase::Current => version_only(),
            HarnessUpdatePhase::Available | HarnessUpdatePhase::ManualActionRequired
                if self.can_apply =>
            {
                version_only()
            }
            HarnessUpdatePhase::Available => match self.latest_version.as_deref() {
                Some(latest) => with(&format!("v{latest} available")),
                None => with("Update available"),
            },
            HarnessUpdatePhase::ManualActionRequired => version_only(),
            HarnessUpdatePhase::WaitingForIdle => with("Waiting for agent to be idle"),
            HarnessUpdatePhase::Preparing => with("Preparing update…"),
            HarnessUpdatePhase::Downloading => with("Downloading…"),
            HarnessUpdatePhase::Installing => with("Installing…"),
            HarnessUpdatePhase::Verifying => with("Verifying…"),
            HarnessUpdatePhase::Updated => with("Updated"),
            HarnessUpdatePhase::Failed if self.install_failed() => with("Update failed"),
            HarnessUpdatePhase::Failed => with("Couldn't check for updates"),
        }
    }

    /// An installed release this Zeron can't run explains failing runs: it
    /// outranks everything but an update in flight.
    pub fn blocking_compatibility(&self) -> Option<&HarnessCompatibility> {
        self.compatibility.as_ref().filter(|_| {
            matches!(
                self.phase,
                HarnessUpdatePhase::Dormant
                    | HarnessUpdatePhase::Checking
                    | HarnessUpdatePhase::Current
                    | HarnessUpdatePhase::Available
                    | HarnessUpdatePhase::ManualActionRequired
                    | HarnessUpdatePhase::Failed
            )
        })
    }

    /// The failure was the update itself (its release is still known, so it
    /// can be retried), not a failed check.
    pub fn install_failed(&self) -> bool {
        self.error
            .as_ref()
            .is_some_and(|error| error.kind == HarnessUpdateFailureKind::Install)
    }

    /// Home notices describe discovered releases or an update in progress.
    /// Manual checks remain available in Settings without implying that a
    /// newer release exists.
    pub fn show_update_notice(&self) -> bool {
        matches!(
            self.phase,
            HarnessUpdatePhase::Available
                | HarnessUpdatePhase::WaitingForIdle
                | HarnessUpdatePhase::Preparing
                | HarnessUpdatePhase::Downloading
                | HarnessUpdatePhase::Installing
                | HarnessUpdatePhase::Verifying
                | HarnessUpdatePhase::Updated
                | HarnessUpdatePhase::Failed
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningLevel {
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
    Max,
    Ultra,
    /// xhigh + harness-specific setting.
    Ultracode,
    /// Prompt-prefix driven (Claude).
    Ultrathink,
}

impl ReasoningLevel {
    pub fn label(self) -> &'static str {
        match self {
            ReasoningLevel::Minimal => "Minimal",
            ReasoningLevel::Low => "Low",
            ReasoningLevel::Medium => "Medium",
            ReasoningLevel::High => "High",
            ReasoningLevel::XHigh => "X-High",
            ReasoningLevel::Max => "Max",
            ReasoningLevel::Ultra => "Ultra",
            ReasoningLevel::Ultracode => "Ultracode",
            ReasoningLevel::Ultrathink => "Ultrathink",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxLevel {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

/// How a chat's agent actions are approved — one setting shared by every
/// harness, each mapping it onto its own approval/sandbox wire. The sole
/// source harnesses read; the older `sandbox` and `auto_approve` fields on
/// the same structs are no longer consulted and stay on the wire only so
/// peers on an older build still decode the row (a config they can't decode
/// is dropped wholesale — see the lenient chat-config decode in the doc
/// crate).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionMode {
    /// Every tool and command asks first.
    Ask,
    /// The harness's own "auto" mode: edits inside the workspace run, anything
    /// it flags as unsafe (commands, network, outside the workspace) asks.
    Auto,
    /// No prompts at all — the pre-existing unattended behavior.
    #[default]
    Full,
}

impl PermissionMode {
    pub const ALL: [PermissionMode; 3] = [
        PermissionMode::Ask,
        PermissionMode::Auto,
        PermissionMode::Full,
    ];

    pub fn sandbox(self) -> SandboxLevel {
        match self {
            PermissionMode::Ask | PermissionMode::Auto => SandboxLevel::WorkspaceWrite,
            PermissionMode::Full => SandboxLevel::DangerFullAccess,
        }
    }

    pub fn auto_approve(self) -> bool {
        matches!(self, PermissionMode::Full)
    }

    pub fn rank(self) -> u8 {
        match self {
            PermissionMode::Ask => 0,
            PermissionMode::Auto => 1,
            PermissionMode::Full => 2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PermissionMode::Ask => "Ask for approval",
            PermissionMode::Auto => "Approve for me",
            PermissionMode::Full => "Full access",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            PermissionMode::Ask => "Ask",
            PermissionMode::Auto => "Auto",
            PermissionMode::Full => "Full",
        }
    }

    pub fn access(self) -> AccessMode {
        match self {
            PermissionMode::Ask => AccessMode::Supervised,
            PermissionMode::Auto => AccessMode::AutoAcceptEdits,
            PermissionMode::Full => AccessMode::FullAccess,
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            PermissionMode::Ask => "Always ask before tools and commands",
            PermissionMode::Auto => "Only ask for actions flagged as unsafe",
            PermissionMode::Full => "Any file, any command, no prompts",
        }
    }
}

/// How much an agent may do before it has to ask. Each harness maps the
/// mode onto its own approval policy and sandbox; it lists the modes it can
/// honor (`Harness::access_modes`), and the composer offers only
/// those. Whatever a run must still ask reaches the person as a question.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AccessMode {
    /// Ask before commands and file changes.
    Supervised,
    /// Apply file changes without asking; ask before anything else.
    AutoAcceptEdits,
    /// Run commands and change files without asking. Every run before modes
    /// existed ran this way, so it is the default.
    #[default]
    FullAccess,
}

impl AccessMode {
    /// Every mode, least to most trusting (the picker's order).
    pub const ALL: [AccessMode; 3] = [
        AccessMode::Supervised,
        AccessMode::AutoAcceptEdits,
        AccessMode::FullAccess,
    ];

    pub fn is_full_access(&self) -> bool {
        *self == AccessMode::FullAccess
    }

    pub fn label(self) -> &'static str {
        match self {
            AccessMode::Supervised => "Supervised",
            AccessMode::AutoAcceptEdits => "Auto-accept edits",
            AccessMode::FullAccess => "Full access",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            AccessMode::Supervised => "Ask before commands and file changes.",
            AccessMode::AutoAcceptEdits => "Apply edits, ask before other actions.",
            AccessMode::FullAccess => "Run commands and edits without asking.",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SteeringMode {
    /// Steer delivered at the next step boundary within the live turn.
    StepBoundary,
    /// Steer delivered only between turns.
    TurnBoundary,
}

/// The caption under the composer when a Steer follow-up will queue because
/// the agent only takes input between turns.
pub fn steer_queues_notice(
    queueing: bool,
    prefers_steer: bool,
    steers_mid_turn: bool,
) -> Option<&'static str> {
    (queueing && prefers_steer && !steers_mid_turn).then_some(
        "This agent can't be steered mid-turn. Your message will be queued and sent when the current turn finishes.",
    )
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub id: String,
    pub label: String,
    /// Short tagline rendered under the name in the model picker (11px muted),
    /// mirroring the Electron app's `ModelInfo.description`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub reasoning_levels: Vec<ReasoningLevel>,
    #[serde(default)]
    pub options: Vec<ModelOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelOption {
    pub id: String,
    pub label: String,
    pub choices: Vec<ModelOptionChoice>,
    pub default_choice: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelOptionChoice {
    pub id: String,
    pub label: String,
}

/// Newness is decided in the engine so every harness gets it from one path.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalog {
    pub models: Vec<Model>,
    #[serde(default)]
    pub new_ids: Vec<String>,
    /// Only the listing that first saw them, so a model is announced once
    /// across every window and device view.
    #[serde(default)]
    pub announce_ids: Vec<String>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRequest {
    pub prompt: String,
    /// The harness picked at send time. Rides the command plane so
    /// claim-on-first-command (chat row still in flight on the registry
    /// channel) dispatches — and records — the picked harness instead of the
    /// engine default. Additive + serde-defaulted for wire compat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<HarnessId>,
    pub model: Option<String>,
    pub reasoning: Option<ReasoningLevel>,
    /// Harness-specific option selections (option id -> choice id), JSON round-tripped.
    #[serde(default)]
    pub model_options: serde_json::Map<String, serde_json::Value>,
    pub cwd: String,
    pub sandbox: SandboxLevel,
    #[serde(default)]
    pub auto_approve: bool,
    /// See [`PermissionMode`]. Additive + serde-defaulted (Full) for wire
    /// compat: an old client's request runs exactly as before.
    #[serde(default)]
    pub permission_mode: PermissionMode,
    /// What the agent may do without asking (see [`AccessMode`]). Additive +
    /// serde-defaulted: a request from a client predating modes runs with
    /// full access, as every run did then.
    #[serde(default, skip_serializing_if = "AccessMode::is_full_access")]
    pub access: AccessMode,
    /// Plan mode: the agent explores and proposes a plan instead of
    /// changing anything. Honored by harnesses that advertise it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub plan: bool,
    /// Harness-native session id to resume, if any.
    pub resume: Option<String>,
    /// Absolute paths of image attachments already staged on the run device
    /// (composer uploads: UploadChunk/UploadCommit → durable path). The same
    /// paths also ride the prompt text as `Attached images (local files …)`
    /// refs (zeron's `withAttachments` transport — that's what persists in the
    /// doc); this field additionally lets a harness inline the bytes as image
    /// content blocks. Additive + serde-defaulted for wire compat.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<String>,
    /// Host-side isolated-worktree creation (see [`WorktreeSpec`]): when set,
    /// the HOST materializes the worktree at command-drain time and runs there
    /// instead of `cwd`. Additive + serde-defaulted for wire compat — an old
    /// host ignores it and runs in `cwd` (the repo's main checkout).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<WorktreeSpec>,
    /// Cut the resumed harness session before this run (see
    /// [`SessionBranch`]). Engine-injected from the chat's pending branch;
    /// callers send `None`. Additive + serde-defaulted for wire compat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<SessionBranch>,
    // History before this instant is the source thread's; its background agents are not ours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_at: Option<String>,
    // Engine-resolved at dispatch; a value a client sends is overwritten.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub computer_control: bool,
    /// Zeron's own MCP server, injected by the HOST engine as it starts the
    /// run: the `zeron mcp` subcommand of this same binary, pointed at the
    /// engine's loopback IPC and stamped with the originating chat so the
    /// agent can spawn, read, and message side chats. Additive +
    /// serde-defaulted — an old host leaves it unset and the agent simply has
    /// no Zeron tools; title runs never carry it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<McpServer>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_tools: Vec<String>,
    /// Host-resolved environment for the spawned agent process (credential
    /// broker secrets on a Cloud device: `CODEX_HOME`, `GH_TOKEN`,
    /// `ANTHROPIC_API_KEY`, …). Filled by the HOST right before the request
    /// reaches the harness; every driver applies it to the child it spawns.
    /// `serde(skip)` in BOTH directions: secrets must never ride the command
    /// plane, the session doc, the run journal, or a log line — and a remote
    /// sender can never inject environment into a host's agents.
    #[serde(skip)]
    pub env: std::collections::BTreeMap<String, String>,
}

/// Hand-written so a `{:?}` can never print a credential: `env` shows its
/// keys only. Destructured without `..`, so a new field fails to compile
/// until it is listed here.
impl std::fmt::Debug for RunRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let RunRequest {
            prompt,
            harness,
            model,
            reasoning,
            model_options,
            cwd,
            sandbox,
            auto_approve,
            permission_mode,
            access,
            plan,
            resume,
            attachments,
            worktree,
            branch,
            forked_at,
            computer_control,
            mcp,
            disabled_tools,
            env,
        } = self;
        f.debug_struct("RunRequest")
            .field("prompt", prompt)
            .field("harness", harness)
            .field("model", model)
            .field("reasoning", reasoning)
            .field("model_options", model_options)
            .field("cwd", cwd)
            .field("sandbox", sandbox)
            .field("auto_approve", auto_approve)
            .field("permission_mode", permission_mode)
            .field("access", access)
            .field("plan", plan)
            .field("resume", resume)
            .field("attachments", attachments)
            .field("worktree", worktree)
            .field("branch", branch)
            .field("forked_at", forked_at)
            .field("computer_control", computer_control)
            .field("mcp", mcp)
            .field("disabled_tools", disabled_tools)
            .field("env", &env.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// A stdio MCP server the harness should add to the agent's session, on top
/// of whatever the user configured. Each driver spells it in its own dialect
/// (Claude `--mcp-config`, ACP `session/new` `mcpServers`, Codex
/// `mcp_servers.*` config overrides).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServer {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
}

/// Where a resumed harness session is cut when the run starts: history
/// through `keep` (the harness's own handle for the last reply kept) survives,
/// `drop` (its handle for the first discarded prompt, when known) onward is
/// discarded. Applied on a forked copy of the session, never in place, so the
/// original stays resumable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionBranch {
    pub keep: Option<String>,
    pub drop: Option<String>,
}

/// Which side of a turn an [`AgentEvent::Anchor`] names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AnchorRole {
    User,
    Assistant,
}

impl McpServer {
    /// Set to `1` on Zeron's own server when its host turned the provider's
    /// native subagents off (Settings → Providers) and the chat has no
    /// parent: the agent delegates through this server's side chats instead,
    /// and the driver launches it without its subagent tools.
    pub const SIDE_CHATS_ONLY_ENV: &'static str = "ZERON_SIDE_CHATS_ONLY";

    pub fn side_chats_only(&self) -> bool {
        self.env
            .get(Self::SIDE_CHATS_ONLY_ENV)
            .is_some_and(|value| value == "1")
    }
}

/// Isolated-worktree directive riding [`RunRequest`]. The worktree is created
/// by the HOST while draining the queued Run — not by the sender over a
/// blocking CreateWorktree RPC — so the send path stays durable: a lost relay
/// frame can't wedge the composer on "Sending…" while the session runs anyway
/// (2026-08-18 user report).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeSpec {
    /// The repo whose worktree to create (the space's folder on the host).
    pub repo_path: String,
    /// Base ref the fresh `zeron/<name>` branch is created off.
    pub base: String,
    /// Owning project used to resolve host-local setup Actions. Optional for
    /// wire compatibility with clients that only request worktree creation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_id: Option<String>,
}

/// The session-scoped singleton id for the live plan/todo chip. ACP plan
/// updates carry no wire id; adapters emit every update under this one id so
/// the fold refreshes the same chip in place. Consumers that de-duplicate
/// tool ids across segment boundaries (the engine's stale-echo filter) must
/// EXEMPT this id — it legitimately reappears in every segment for the whole
/// life of a run.
pub const LIVE_PLAN_TOOL_ID: &str = "acp-plan";

/// A decoded tool invocation, reduced to the fields each kind renders.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ToolCall {
    Exec {
        command: String,
    },
    ReadFile {
        path: String,
    },
    WriteFile {
        path: String,
        /// Full content; STRIPPED by the render-parts policy before entering the doc.
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<String>,
    },
    EditFile {
        path: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        old_string: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        new_string: Option<String>,
    },
    ApplyPatch {
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    Search {
        pattern: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    Glob {
        pattern: String,
    },
    WebFetch {
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        prompt: Option<String>,
    },
    WebSearch {
        query: String,
    },
    Todo {
        #[serde(default)]
        items: Vec<TodoItem>,
    },
    Mcp {
        server: String,
        tool: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        input: Option<serde_json::Value>,
    },
    Unknown {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        input: Option<serde_json::Value>,
    },
}

impl ToolCall {
    /// A subagent SPAWN call — the `Agent[: <description>]` naming convention
    /// every driver decodes its spawn tool into (claude/codex `Task`, cursor
    /// `task`, grok `spawn_subagent`, opencode `task`). This is the single
    /// genus gate for subagent binding: tagged subagent traffic may only ever
    /// stamp a ref/status onto a spawn call, so a driver keying bug can never
    /// turn an ordinary Run/Read chip into a spawn chip (2026-08-20: claude's
    /// background-shell `task_notification` did exactly that — the chip
    /// linked to a never-created doc and opened an empty panel).
    pub fn is_subagent_spawn(&self) -> bool {
        let name = match self {
            ToolCall::Unknown { name, .. } => name,
            ToolCall::Mcp { tool, .. } => tool,
            _ => return false,
        };
        name == "Agent" || name.starts_with("Agent: ")
    }

    /// Never a native spawn: the engine binds its child by task text (d67).
    pub fn is_delegate_task(&self) -> bool {
        match self {
            ToolCall::Mcp { tool, .. } => tool == "delegate_task",
            ToolCall::Unknown { name, .. } => name.ends_with("delegate_task"),
            _ => false,
        }
    }

    pub fn is_create_thread(&self) -> bool {
        match self {
            ToolCall::Mcp { tool, .. } => tool == "create_thread",
            ToolCall::Unknown { name, .. } => name.ends_with("create_thread"),
            _ => false,
        }
    }

    /// An agent-published HTML page (the `html_render` thread tool). Only
    /// zeron's own server: another MCP server may ship a same-named tool.
    pub fn is_html_render(&self) -> bool {
        match self {
            ToolCall::Mcp { server, tool, .. } => server == "zeron" && tool == "html_render",
            ToolCall::Unknown { name, .. } => {
                matches!(
                    name.as_str(),
                    "zeron_html_render" | "mcp__zeron__html_render" | "zeron/html_render"
                )
            }
            _ => false,
        }
    }

    /// The page an `html_render` call published, as the agent sent it.
    pub fn html_render_html(&self) -> Option<&str> {
        match self {
            ToolCall::Unknown { input, .. } | ToolCall::Mcp { input, .. }
                if self.is_html_render() =>
            {
                input.as_ref()?.get("html")?.as_str()
            }
            _ => None,
        }
    }

    pub fn delegate_task_text(&self) -> Option<&str> {
        match self {
            ToolCall::Unknown { input, .. } | ToolCall::Mcp { input, .. }
                if self.is_delegate_task() =>
            {
                input.as_ref()?.get("task")?.as_str().map(str::trim)
            }
            _ => None,
        }
    }

    /// The model a subagent SPAWN was given, when the spawn named one.
    ///
    /// Read off the spawn's own input rather than the session's picked model:
    /// a spawn may override it per child (claude's `Agent` takes `model`, grok
    /// `spawn_subagent` a `model_id`), and two chips spawned in one turn can
    /// legitimately name different models. `None` means the spawn didn't say —
    /// the child inherits the parent's model, which the chip already implies,
    /// so nothing is rendered rather than guessing a name.
    ///
    /// Only ever answers for [`is_subagent_spawn`](Self::is_subagent_spawn)
    /// calls: an ordinary tool with a stray `model` argument is not a spawn.
    pub fn subagent_model(&self) -> Option<&str> {
        if !self.is_subagent_spawn() {
            return None;
        }
        let input = match self {
            ToolCall::Unknown { input, .. } | ToolCall::Mcp { input, .. } => input.as_ref()?,
            _ => return None,
        };
        SUBAGENT_MODEL_KEYS
            .iter()
            .find_map(|key| input.get(key).and_then(serde_json::Value::as_str))
            .map(str::trim)
            .filter(|model| !model.is_empty())
    }

    /// Tab title for a spawn chip's subagent surface: the BARE task description
    /// ("verify the marker pipeline"). The chip keeps the tool's fuller name;
    /// the input's description/prompt back up a bare name (older docs).
    pub fn subagent_title(&self) -> String {
        let (name, input) = match self {
            ToolCall::Unknown { name, input } => (name.as_str(), input.as_ref()),
            ToolCall::Mcp { tool, input, .. } => (tool.as_str(), input.as_ref()),
            _ => return "Subagent".into(),
        };
        let candidates = [
            Some(name),
            input.and_then(|i| i.get("description")?.as_str()),
            input.and_then(|i| i.get("prompt")?.as_str()),
        ];
        candidates
            .into_iter()
            .flatten()
            .find_map(|text| title_line(strip_spawn_prefix(text), SUBAGENT_TITLE_MAX))
            .unwrap_or_else(|| "Subagent".into())
    }

    pub fn subagent_reasoning(&self) -> Option<ReasoningLevel> {
        let (ToolCall::Unknown { input, .. } | ToolCall::Mcp { input, .. }) = self else {
            return None;
        };
        let input = input.as_ref()?;
        SUBAGENT_EFFORT_KEYS.iter().find_map(|key| {
            let raw = input
                .get(*key)?
                .as_str()?
                .trim()
                .to_ascii_lowercase()
                .replace(['-', '_'], "");
            serde_json::from_value(serde_json::Value::String(raw)).ok()
        })
    }

    /// Claude's advisor consultation — a SERVER tool whose answer comes back
    /// encrypted, so the only client-visible facts are that it ran and for
    /// how long. Decoded as an Unknown named [`ADVISOR_TOOL_NAME`]; the chip
    /// label, group summary and working trailer key on this.
    pub fn is_advisor(&self) -> bool {
        matches!(self, ToolCall::Unknown { name, .. } if name == ADVISOR_TOOL_NAME)
    }

    pub fn skill_name(&self) -> Option<&str> {
        self.skill_input("skill")
    }

    pub fn skill_path(&self) -> Option<&str> {
        self.skill_input("path")
            .filter(|path| path.ends_with("/SKILL.md"))
    }

    fn skill_input(&self, key: &str) -> Option<&str> {
        let ToolCall::Unknown {
            name,
            input: Some(input),
        } = self
        else {
            return None;
        };
        if name != "Skill" {
            return None;
        }
        let value = input.get(key)?.as_str()?.trim();
        (!value.is_empty()).then_some(value)
    }
}

/// The name the claude driver decodes a `server_tool_use` advisor call into.
pub const ADVISOR_TOOL_NAME: &str = "Advisor";

/// Spawn-input keys that carry a child model, in precedence order. Drivers
/// disagree on the spelling, so the lookup is by key set, not by harness —
/// a new adapter naming it any of these needs no code change here.
pub const SUBAGENT_MODEL_KEYS: [&str; 4] = ["model", "modelId", "model_id", "subagent_model"];

const SUBAGENT_EFFORT_KEYS: [&str; 4] = ["effort", "reasoning_effort", "reasoningEffort", "reasoning"];

/// Max chars a subagent tab title keeps. The strip chip is fixed-width and
/// truncates visually, but the derived title also rides drag ghosts and any
/// future pickers — cap it at the source.
pub const SUBAGENT_TITLE_MAX: usize = 40;

/// First line of `text`, trimmed, capped at `max` chars with an ellipsis.
fn title_line(text: &str, max: usize) -> Option<String> {
    let line = text.lines().find(|l| !l.trim().is_empty())?.trim();
    let mut out: String = line.chars().take(max).collect();
    if line.chars().count() > max {
        out.push('…');
    }
    Some(out)
}

/// Drop a leading "Agent"/"Task" genus (with its `:` and spacing) from a
/// spawn-title candidate. Only a real word boundary strips — "Taskmaster"
/// keeps its name. A bare "Agent"/"Task" strips to "" (no context at all).
fn strip_spawn_prefix(text: &str) -> &str {
    let t = text.trim();
    for prefix in ["agent", "task"] {
        if t.len() >= prefix.len()
            && t.is_char_boundary(prefix.len())
            && t[..prefix.len()].eq_ignore_ascii_case(prefix)
        {
            let rest = &t[prefix.len()..];
            if rest.is_empty() {
                return "";
            }
            if rest.starts_with(':') || rest.starts_with(char::is_whitespace) {
                return rest.trim_start_matches(':').trim();
            }
        }
    }
    t
}

/// The spawn-input keys [`sanitize_tool_call`](crate::) must preserve so the
/// chip can name the child's model. Deliberately tiny: everything else on a
/// spawn's input (the whole prompt, most of all) stays host-local.
pub const SUBAGENT_INPUT_KEEP: [&str; 5] = [
    "model",
    "modelId",
    "model_id",
    "subagent_model",
    "subagent_type",
];

/// Bounds on an agent's inline visual ([`AgentEvent::Visual`]): a Luau
/// script Zeron runs in a sandbox and renders natively.
pub const VISUAL_MAX_BYTES: usize = 256 * 1024;
pub const VISUAL_MAX_TITLE_CHARS: usize = 200;
/// Files a visual may ship with (models, textures, environment maps).
pub const VISUAL_MAX_ASSETS: usize = 16;
pub const VISUAL_MAX_ASSET_BYTES: usize = 8 * 1024 * 1024;
pub const VISUAL_MAX_TOTAL_ASSET_BYTES: usize = 10 * 1024 * 1024;

// The MCP server sends a visual to the engine in one WebSocket message, its
// assets base64-encoded; that message must fit the engine's 16 MiB frame
// (tungstenite's default), with room for the script escaped as JSON.
const _: () = assert!(
    VISUAL_MAX_TOTAL_ASSET_BYTES.div_ceil(3) * 4 + VISUAL_MAX_BYTES * 6 + 64 * 1024
        < 16 * 1024 * 1024
);

/// Largest image file an agent may show with `show_image` (it rides one
/// WebSocket message, base64-encoded, like a visual's assets).
pub const IMAGE_MAX_BYTES: usize = 10 * 1024 * 1024;
/// Longest side, in pixels, of an image the desktop decodes for a preview;
/// `show_image` refuses larger ones rather than publish an unviewable part.
pub const IMAGE_MAX_SIDE: u32 = 4096;
const _: () = assert!(IMAGE_MAX_BYTES.div_ceil(3) * 4 + 64 * 1024 < 16 * 1024 * 1024);

/// Whether `name` can name a visual's asset: a plain file name.
pub fn valid_visual_asset_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// Where one checklist item stands. Claude's TodoWrite, OpenCode and ACP plans
/// distinguish the item being worked on from those still waiting; Codex and
/// Cursor only report done / not done.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TodoStatus {
    #[default]
    Pending,
    InProgress,
    Completed,
}

impl TodoStatus {
    /// Decode a harness status string. Anything unrecognised (including
    /// `cancelled`) is `Pending`: not finished, not being worked on.
    pub fn parse(raw: &str) -> Self {
        match raw {
            "completed" | "complete" | "done" => Self::Completed,
            "in_progress" | "inProgress" | "in-progress" | "active" => Self::InProgress,
            _ => Self::Pending,
        }
    }
}

/// One checklist entry.
///
/// `done` is the original wire field and stays authoritative for completion,
/// so docs written before `status` existed (and readers that ignore it) keep
/// working. `status` is additive and written only when it adds information —
/// i.e. for an in-progress item; a missing or unreadable value derives from
/// `done`. Build items with [`TodoItem::new`] to keep the two consistent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoItem {
    pub text: String,
    pub done: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "lenient_todo_status"
    )]
    pub status: Option<TodoStatus>,
}

impl TodoItem {
    pub fn new(text: impl Into<String>, status: TodoStatus) -> Self {
        Self {
            text: text.into(),
            done: status == TodoStatus::Completed,
            status: (status == TodoStatus::InProgress).then_some(status),
        }
    }

    /// Effective status: `done` wins, then the explicit status, else pending.
    pub fn status(&self) -> TodoStatus {
        if self.done {
            TodoStatus::Completed
        } else {
            self.status.unwrap_or_default()
        }
    }
}

/// A status this build does not know (a future `blocked`, say) must not fail
/// the whole tool call — the part would vanish from the transcript. Treat it
/// as absent so the item falls back to `done`.
fn lenient_todo_status<'de, D>(deserializer: D) -> Result<Option<TodoStatus>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(|v| serde_json::from_value(v).ok()))
}

/// A command advertised by the agent (ACP `availableCommands`), sent to the
/// agent as prompt text. Its kind picks the composer sigil: built-ins are
/// typed as `/name` at the start of the prompt, skills as `$name` anywhere.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlashCommand {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Placeholder hint for the command's argument, when it takes one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_hint: Option<String>,
    #[serde(default)]
    pub kind: SlashCommandKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SlashCommandKind {
    /// The agent's own command (claude's `/clear`, `/model`), or zeron's.
    #[default]
    BuiltIn,
    /// A `SKILL.md` the agent loads on request.
    Skill,
}

/// How a harness compacts its own conversation context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Compaction {
    /// The driver drives compaction over the agent's own wire (`Harness::compact`).
    Native,
    /// Prompt text the agent's CLI interprets itself (claude's `/compact`),
    /// sent through the ordinary run path.
    SlashText(String),
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompactionKind {
    Native,
    Slash,
    None,
}

impl From<Compaction> for CompactionKind {
    fn from(compaction: Compaction) -> Self {
        match compaction {
            Compaction::Native => Self::Native,
            Compaction::SlashText(_) => Self::Slash,
            Compaction::None => Self::None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessCompactionParams {
    pub harness: HarnessId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_device_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessCompactionResult {
    pub compaction: CompactionKind,
    #[serde(default)]
    pub branching: bool,
}

/// A file modification carried inline on a tool result (ACP
/// `ToolCallContent::Diff`). `old_text: None` means a new file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDiff {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_text: Option<String>,
    pub new_text: String,
    /// Unchanged lines cut from the top of both texts to fit the byte cap;
    /// the texts start at line `line_offset + 1` of the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_offset: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserInputQuestion {
    pub id: String,
    pub header: String,
    pub question: String,
    pub options: Vec<String>,
    #[serde(default)]
    pub multi_select: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<UserInputKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefill: Option<String>,
    #[serde(default)]
    pub multiline: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub approval: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum UserInputKind {
    Consent {
        scope: crate::computer::ComputerConsentScope,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        app: Option<crate::computer::ComputerConsentApp>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        params: Option<String>,
    },
    Foreground {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        app: Option<crate::computer::ComputerConsentApp>,
    },
}

impl UserInputQuestion {
    pub fn is_consent(&self) -> bool {
        matches!(self.kind, Some(UserInputKind::Consent { .. }))
    }

    pub fn is_computer(&self) -> bool {
        self.kind.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserInputAnswer {
    pub question_id: String,
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DoneStatus {
    Completed,
    Interrupted,
    Errored,
}

/// The normalized streaming event every harness emits.
///
/// Mirrors zeron's `AgentEvent` tagged enum.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AgentEvent {
    #[serde(rename_all = "camelCase")]
    SessionStarted {
        harness: HarnessId,
        model: String,
        #[serde(default)]
        tools: Vec<String>,
        cwd: String,
        /// Harness-native session id (used for resume).
        session_id: String,
        assistant_message_id: String,
    },
    TextDelta {
        text: String,
    },
    /// A raster image in the reply: one a harness generated (the engine
    /// materializes this path before publication) or one an agent showed with
    /// Zeron's `show_image` MCP tool (already stored in uploads).
    #[serde(rename_all = "camelCase")]
    GeneratedImage {
        id: String,
        path: String,
        name: String,
        mime_type: String,
    },
    /// An agent-authored visual shown inline in the reply: a Luau script
    /// that builds native UI (`zeron-visual`). The engine emits it when the
    /// agent calls Zeron's `show_visual` MCP tool; `path` is the stored
    /// script on the host device.
    Visual {
        id: String,
        path: String,
        title: String,
    },
    ReasoningDelta {
        text: String,
    },
    /// Backend-internal steering boundary marker.
    #[serde(rename_all = "camelCase")]
    AssistantMessageCompleted {
        assistant_message_id: String,
    },
    ToolCall {
        id: String,
        call: ToolCall,
    },
    #[serde(rename_all = "camelCase")]
    ToolResult {
        id: String,
        is_error: bool,
        /// Tool output text, capped by the emitting harness (ACP tool-call
        /// content; claude/codex adapters never populate it). The doc-side
        /// fold applies its own byte cap before anything persists.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<String>,
        /// Inline file diff for edit-shaped tools (ACP `Diff` content).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        diff: Option<ToolDiff>,
    },
    /// Latest context occupancy, independent of cumulative billing usage.
    /// Missing fields preserve the previous measurement; zero tokens is valid.
    #[serde(rename_all = "camelCase")]
    ContextUsage {
        tokens: Option<u64>,
        window: Option<u64>,
    },
    /// The harness compacted its own conversation context. Token counts are
    /// the harness's own pre/post measurement, absent when it reports none.
    #[serde(rename_all = "camelCase")]
    Compacted {
        pre_tokens: Option<u64>,
        post_tokens: Option<u64>,
    },
    Compacting {
        active: bool,
    },
    /// The harness's full set of live background tasks (replace, never
    /// merge); empty when the last one ends. Not subagent traffic.
    BackgroundTasks {
        tasks: Vec<crate::BackgroundTask>,
    },
    /// The subscription's usage as the provider reported it mid-run (Muse's
    /// `usage/changed`): the engine records it on the account the run used,
    /// for the Providers meters. Never persisted to docs.
    #[serde(rename_all = "camelCase")]
    PlanUsage {
        windows: Vec<crate::AgentUsageWindow>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plan_label: Option<String>,
    },
    /// Kept as a harness passthrough (rate-limit probes); only its running
    /// totals reach the doc, through the session stats.
    #[serde(rename_all = "camelCase")]
    Usage {
        input_tokens: u64,
        output_tokens: u64,
    },
    /// A turn's prompt-cache traffic. `ttl_secs` is the tier the turn wrote
    /// to, absent when it only read.
    #[serde(rename_all = "camelCase")]
    PromptCache {
        read_tokens: u64,
        write_tokens: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ttl_secs: Option<u32>,
    },
    /// The harness session's running cost. Claude reports a total for the
    /// whole session and restores it on `--resume` (live 2.1.289), so it is
    /// never a per-turn figure.
    #[serde(rename_all = "camelCase")]
    Cost {
        session_id: String,
        total_usd: f64,
    },
    /// The agent advertised (or changed) its slash-command set — ACP
    /// `available_commands_update`. The engine caches the latest list per
    /// harness for the composer's `/` popup; never persisted to docs.
    #[serde(rename_all = "camelCase")]
    AvailableCommands {
        commands: Vec<SlashCommand>,
    },
    Error {
        message: String,
    },
    /// `resets_at`: epoch ms when every exhausted window has reset; `None` when any gave no reset time.
    #[serde(rename_all = "camelCase")]
    UsageLimit {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resets_at: Option<i64>,
    },
    #[serde(rename_all = "camelCase")]
    InputRequested {
        request_id: String,
        questions: Vec<UserInputQuestion>,
    },
    #[serde(rename_all = "camelCase")]
    InputResolved {
        request_id: String,
        /// What the person answered, exactly as the run received it. Empty
        /// when the questions closed without an answer (dismissed, or the run
        /// stopped waiting). Absent on events written before answers were
        /// recorded — nothing is known about them.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        answers: Option<Vec<UserInputAnswer>>,
    },
    /// A confirmed new assignment. When tagged as Subagent, this reopens the
    /// same child transcript even if the provider does not echo the user text.
    #[serde(rename_all = "camelCase")]
    Steered {
        assistant_message_id: Option<String>,
        next_assistant_message_id: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Done {
        status: DoneStatus,
        result: Option<String>,
        error: Option<String>,
        session_id: Option<String>,
    },
    /// A USER-role message injected into a running session — today only seen
    /// wrapped in [`AgentEvent::Subagent`]: the PARENT agent steering its
    /// subagent mid-run (claude: a tagged user frame's text blocks). The
    /// engine writes it to the subagent doc as its own user entry, closing
    /// the streaming assistant segment above it — the subagent transcript
    /// then reads like any steered chat. Never emitted untagged (the parent
    /// chat's user messages come from doc commands, not the wire).
    /// The harness's own handle for the current prompt (`User`) or reply
    /// (`Assistant`): a Claude message uuid, a Codex turn id, an OpenCode
    /// message id. Drivers leave `entry_id` empty; the engine stamps the doc
    /// entry it belongs to before journaling. Never persisted to docs.
    #[serde(rename_all = "camelCase")]
    Anchor {
        role: AnchorRole,
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        entry_id: Option<String>,
    },
    /// Engine-written journal record: the chat's next run branches its
    /// resumed harness session at this point (see [`SessionBranch`]). The
    /// run's `SessionStarted` retires it. Never persisted to docs.
    #[serde(rename_all = "camelCase")]
    BranchPending {
        keep: Option<String>,
        drop: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        forked_at: Option<String>,
    },
    /// Engine-written journal record: the chat's harness session was
    /// tombstoned (a provider switch, a rewind to the first prompt). Anchors
    /// recorded before it belong to a session no later run can branch. Never
    /// persisted to docs.
    #[serde(rename_all = "camelCase")]
    SessionTombstoned {},
    #[serde(rename_all = "camelCase")]
    UserMessage {
        text: String,
    },
    /// An event belonging to a SUBAGENT's nested transcript, attributed to
    /// the spawning tool call (`parent_tool_use_id` = the parent-feed
    /// `ToolCall::id` that launched it). Never folded into the parent chat
    /// doc — the engine routes these to the subagent's own doc; the parent
    /// keeps only the spawn chip. The spawn `ToolCall` must come first: the
    /// engine opens that doc only for an id that already names one of the
    /// chat's spawn chips ([`ToolCall::is_subagent_spawn`]), and keeps
    /// traffic tagged with any other id in the run journal only. Additive:
    /// old consumers that don't match this variant drop the nested traffic,
    /// which is the pre-subagent-viz behavior.
    #[serde(rename_all = "camelCase")]
    Subagent {
        parent_tool_use_id: String,
        event: Box<AgentEvent>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_persisted_consent_card_without_an_app_still_loads_and_keeps_its_shape() {
        let old = serde_json::json!({ "type": "consent", "scope": "task" });
        let kind: UserInputKind = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(
            kind,
            UserInputKind::Consent {
                scope: crate::computer::ComputerConsentScope::Task,
                app: None,
                tool: None,
                params: None,
            }
        );
        assert_eq!(serde_json::to_value(&kind).unwrap(), old);
        let with_app = serde_json::json!({
            "type": "consent",
            "scope": "task",
            "app": { "name": "Safari", "bundleId": "com.apple.Safari", "pid": 42 },
        });
        let kind: UserInputKind = serde_json::from_value(with_app.clone()).unwrap();
        let UserInputKind::Consent { app: Some(app), .. } = &kind else {
            panic!("the app identity survives");
        };
        assert!(app.verified());
        assert_eq!(serde_json::to_value(&kind).unwrap(), with_app);
        let with_action = serde_json::json!({
            "type": "consent",
            "scope": "task",
            "tool": "computer_click",
            "params": "{\"window_id\":\"w1\"}",
        });
        let kind: UserInputKind = serde_json::from_value(with_action.clone()).unwrap();
        let UserInputKind::Consent {
            tool: Some(tool), ..
        } = &kind
        else {
            panic!("the action survives");
        };
        assert_eq!(tool, "computer_click");
        assert_eq!(serde_json::to_value(&kind).unwrap(), with_action);
    }

    #[test]
    fn a_foreground_question_is_a_computer_question_and_keeps_its_shape() {
        let wire = serde_json::json!({ "type": "foreground", "app": { "name": "Ghostty" } });
        let kind: UserInputKind = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(&kind).unwrap(), wire);
        let question = UserInputQuestion {
            approval: false,
            id: "foreground".into(),
            header: "Computer".into(),
            question: "Ghostty needs to come to the front to type.".into(),
            options: vec!["Allow for this turn".into(), "No".into()],
            multi_select: false,
            kind: Some(kind),
            prefill: None,
            multiline: false,
        };
        assert!(question.is_computer() && !question.is_consent());
        let plain = UserInputQuestion {
            kind: None,
            ..question
        };
        assert!(!plain.is_computer());
    }

    #[test]
    fn compaction_capability_keeps_the_existing_wire_shape() {
        for (capability, wire) in [
            (Compaction::Native, "native"),
            (Compaction::SlashText("/compact".into()), "slash"),
            (Compaction::None, "none"),
        ] {
            let result = HarnessCompactionResult {
                compaction: capability.into(),
                branching: true,
            };
            let json = serde_json::to_value(&result).unwrap();
            assert_eq!(
                json,
                serde_json::json!({"compaction": wire, "branching": true})
            );
            assert_eq!(
                serde_json::from_value::<HarnessCompactionResult>(json)
                    .unwrap()
                    .compaction,
                result.compaction
            );
        }
        let legacy = serde_json::from_value::<HarnessCompactionResult>(
            serde_json::json!({"compaction": "none"}),
        )
        .unwrap();
        assert!(!legacy.branching);
        assert!(
            serde_json::from_value::<HarnessCompactionResult>(
                serde_json::json!({"compaction": "invalid"})
            )
            .is_err()
        );
        let params = HarnessCompactionParams {
            harness: HarnessId::Mock,
            target_device_id: None,
        };
        assert_eq!(
            serde_json::to_value(params).unwrap(),
            serde_json::json!({"harness": "mock"})
        );
    }

    #[test]
    fn agent_event_round_trips() {
        let ev = AgentEvent::ToolCall {
            id: "t1".into(),
            call: ToolCall::Exec {
                command: "cargo test".into(),
            },
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert_eq!(serde_json::from_str::<AgentEvent>(&json).unwrap(), ev);
    }

    #[test]
    fn a_skill_chip_opens_only_a_skill_file() {
        let skill = |path: &str| ToolCall::Unknown {
            name: "Skill".into(),
            input: Some(serde_json::json!({ "skill": "review", "path": path })),
        };
        assert_eq!(
            skill("/home/u/.claude/skills/review/SKILL.md").skill_path(),
            Some("/home/u/.claude/skills/review/SKILL.md")
        );
        assert_eq!(skill("/home/u/.ssh/config").skill_path(), None);
    }

    #[test]
    fn subagent_titles() {
        // The tab is the BARE task — the "Agent:" genus is stripped.
        let named = ToolCall::Unknown {
            name: "Agent: scan repo".into(),
            input: None,
        };
        assert_eq!(named.subagent_title(), "scan repo");
        // A bare "Task"/"Agent" digs the description out of the call input
        // (which sheds any genus of its own).
        let bare = ToolCall::Unknown {
            name: "Task".into(),
            input: Some(serde_json::json!({
                "description": "Agent: audit the auth flow",
                "prompt": "very long instructions…",
            })),
        };
        assert_eq!(bare.subagent_title(), "audit the auth flow");
        // Word boundaries only — a name that merely STARTS with the genus
        // keeps itself.
        let compound = ToolCall::Unknown {
            name: "Taskmaster".into(),
            input: None,
        };
        assert_eq!(compound.subagent_title(), "Taskmaster");
        // Nothing to derive → the generic label.
        let blank = ToolCall::Unknown {
            name: "agent".into(),
            input: None,
        };
        assert_eq!(blank.subagent_title(), "Subagent");
        // Absurd lengths cap with an ellipsis; multiline prompts keep only
        // their first line.
        let long = ToolCall::Unknown {
            name: "x".repeat(120),
            input: None,
        };
        let title = long.subagent_title();
        assert_eq!(title.chars().count(), SUBAGENT_TITLE_MAX + 1);
        assert!(title.ends_with('…'));
        // Non-spawn-shaped calls stay generic.
        assert_eq!(
            ToolCall::Exec {
                command: "ls".into()
            }
            .subagent_title(),
            "Subagent"
        );
    }

    #[test]
    fn a_spawn_names_its_own_level_under_any_spelling() {
        let spawn = |input: serde_json::Value| ToolCall::Unknown {
            name: "Agent".into(),
            input: Some(input),
        };
        assert_eq!(
            spawn(serde_json::json!({ "effort": "X-High" })).subagent_reasoning(),
            Some(ReasoningLevel::XHigh)
        );
        assert_eq!(
            spawn(serde_json::json!({ "reasoning_effort": "medium" })).subagent_reasoning(),
            Some(ReasoningLevel::Medium)
        );
        assert_eq!(
            spawn(serde_json::json!({ "reasoning": "loud" })).subagent_reasoning(),
            None
        );
        assert_eq!(
            spawn(serde_json::json!({ "model": "opus" })).subagent_reasoning(),
            None
        );
    }

    /// Drivers spell the key differently; the chip must not care which one
    /// spawned the child. Non-spawns never answer, whatever they carry.
    #[test]
    fn subagent_model_reads_every_spelling_and_only_off_a_spawn() {
        let spawn = |input: serde_json::Value| ToolCall::Unknown {
            name: "Agent: scan".into(),
            input: Some(input),
        };
        for key in SUBAGENT_MODEL_KEYS {
            let call = spawn(serde_json::json!({ key: "haiku" }));
            assert_eq!(call.subagent_model(), Some("haiku"), "key {key}");
        }
        // An MCP-shaped spawn (cursor routes its `task` through MCP) too.
        assert_eq!(
            ToolCall::Mcp {
                server: "s".into(),
                tool: "Agent: scan".into(),
                input: Some(serde_json::json!({ "model": "sonnet" })),
            }
            .subagent_model(),
            Some("sonnet")
        );
        // Not a spawn: the name gate wins over the key.
        assert_eq!(
            ToolCall::Unknown {
                name: "Bash".into(),
                input: Some(serde_json::json!({ "model": "haiku" })),
            }
            .subagent_model(),
            None
        );
        // A spawn that named nothing usable inherits — nothing to render.
        assert_eq!(
            spawn(serde_json::json!({ "model": " " })).subagent_model(),
            None
        );
        assert_eq!(
            spawn(serde_json::json!({ "prompt": "x" })).subagent_model(),
            None
        );
        assert_eq!(
            ToolCall::Unknown {
                name: "Agent".into(),
                input: None
            }
            .subagent_model(),
            None
        );
        // Non-string values are not names.
        assert_eq!(
            spawn(serde_json::json!({ "model": 5 })).subagent_model(),
            None
        );
    }

    #[test]
    fn run_request_attachments_default_and_round_trip() {
        // Old-wire JSON without the field parses (additive compat)…
        let old = r#"{"prompt":"p","model":null,"reasoning":null,"cwd":".","sandbox":"workspace-write","resume":null}"#;
        let req: RunRequest = serde_json::from_str(old).unwrap();
        assert!(req.attachments.is_empty());
        // …and an empty list serializes away (old readers never see it).
        let json = serde_json::to_value(&req).unwrap();
        assert!(json.get("attachments").is_none());
        // Populated lists round-trip.
        let req = RunRequest {
            attachments: vec!["/tmp/a.png".into()],
            ..req
        };
        let round: RunRequest =
            serde_json::from_value(serde_json::to_value(&req).unwrap()).unwrap();
        assert_eq!(round.attachments, vec!["/tmp/a.png".to_string()]);
    }

    #[test]
    fn run_request_worktree_default_and_round_trip() {
        // Old-wire JSON without the field parses (additive compat)…
        let old = r#"{"prompt":"p","model":null,"reasoning":null,"cwd":".","sandbox":"workspace-write","resume":null}"#;
        let req: RunRequest = serde_json::from_str(old).unwrap();
        assert!(req.worktree.is_none());
        // …and `None` serializes away (old readers never see it).
        let json = serde_json::to_value(&req).unwrap();
        assert!(json.get("worktree").is_none());
        // A populated spec round-trips camelCased.
        let req = RunRequest {
            worktree: Some(WorktreeSpec {
                repo_path: "/repos/comet".into(),
                base: "main".into(),
                space_id: Some("space-1".into()),
            }),
            ..req
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["worktree"]["repoPath"], "/repos/comet");
        assert_eq!(json["worktree"]["spaceId"], "space-1");
        let round: RunRequest = serde_json::from_value(json).unwrap();
        assert_eq!(round.worktree, req.worktree);
    }

    /// Credential-broker secrets ride `RunRequest::env` from the host to the
    /// harness only: they must never reach the wire, the session doc, or the
    /// journal, and a sender can never inject them.
    #[test]
    fn run_request_env_never_serializes_or_deserializes() {
        let old = r#"{"prompt":"p","model":null,"reasoning":null,"cwd":".","sandbox":"workspace-write","resume":null}"#;
        let mut req: RunRequest = serde_json::from_str(old).unwrap();
        assert!(req.env.is_empty());
        req.env
            .insert("GH_TOKEN".into(), "ghu_secret_token_value".into());
        req.env
            .insert("ANTHROPIC_API_KEY".into(), "sk-ant-secret".into());
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("ghu_secret_token_value"), "{json}");
        assert!(!json.contains("sk-ant-secret"), "{json}");
        assert!(!json.contains("GH_TOKEN"), "{json}");
        assert!(!json.contains("\"env\""), "{json}");
        let round: RunRequest = serde_json::from_str(&json).unwrap();
        assert!(round.env.is_empty());
        // A hostile sender spelling the field out is ignored too.
        let injected = r#"{"prompt":"p","model":null,"reasoning":null,"cwd":".","sandbox":"workspace-write","resume":null,"env":{"LD_PRELOAD":"/tmp/x.so"}}"#;
        let req: RunRequest = serde_json::from_str(injected).unwrap();
        assert!(req.env.is_empty());
        // Debug shows which variables were set, never their values.
        let mut req = req;
        req.env
            .insert("GH_TOKEN".into(), "ghu_secret_token_value".into());
        let printed = format!("{req:?}");
        assert!(printed.contains("GH_TOKEN"), "{printed}");
        assert!(!printed.contains("ghu_secret_token_value"), "{printed}");
    }

    #[test]
    fn run_request_permission_mode_defaults_to_full() {
        // Old-wire JSON without the field runs unattended, exactly as before.
        let old = r#"{"prompt":"p","model":null,"reasoning":null,"cwd":".","sandbox":"workspace-write","resume":null}"#;
        let req: RunRequest = serde_json::from_str(old).unwrap();
        assert_eq!(req.permission_mode, PermissionMode::Full);
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["permissionMode"], "full");
        let round: RunRequest = serde_json::from_value(json).unwrap();
        assert_eq!(round.permission_mode, PermissionMode::Full);
        assert_eq!(
            serde_json::to_string(&PermissionMode::Ask).unwrap(),
            "\"ask\""
        );
    }

    #[test]
    fn harness_id_uses_kebab_case() {
        assert_eq!(
            serde_json::to_string(&HarnessId::ClaudeCode).unwrap(),
            "\"claude-code\""
        );
    }

    #[test]
    fn harness_update_status_uses_stable_wire_names() {
        let status = HarnessUpdateStatus {
            harness: HarnessId::ClaudeCode,
            installed_version: Some("1.0.0".into()),
            latest_version: Some("1.1.0".into()),
            channel: Some("stable".into()),
            source: HarnessInstallSource::Npm,
            policy: HarnessUpdatePolicy::AutoWhenIdle,
            phase: HarnessUpdatePhase::WaitingForIdle,
            progress: None,
            checked_at: Some(42),
            error: None,
            can_apply: true,
            manual_command: Some("claude update".into()),
            compatibility: None,
            held_back: None,
        };
        let json = serde_json::to_value(&status).unwrap();
        // Older peers neither send nor expect the compatibility fields.
        assert!(json.get("compatibility").is_none() && json.get("heldBack").is_none());
        assert_eq!(json["harness"], "claude-code");
        assert_eq!(json["installedVersion"], "1.0.0");
        assert_eq!(json["policy"], "auto-when-idle");
        assert_eq!(json["phase"], "waiting-for-idle");
        assert_eq!(json["canApply"], true);
        assert_eq!(
            serde_json::from_value::<HarnessUpdateStatus>(json.clone()).unwrap(),
            status
        );
        let mut legacy = json;
        legacy.as_object_mut().unwrap().remove("canApply");
        assert!(
            !serde_json::from_value::<HarnessUpdateStatus>(legacy)
                .unwrap()
                .can_apply
        );
    }

    #[test]
    fn manual_checks_do_not_claim_an_available_update() {
        let mut status: HarnessUpdateStatus = serde_json::from_value(serde_json::json!({
            "harness": "cursor",
            "phase": "manual-action-required",
            "canApply": true,
        }))
        .unwrap();
        assert!(!status.show_update_notice());
        status.phase = HarnessUpdatePhase::Available;
        status.latest_version = Some("2.0.0".into());
        status.can_apply = false;
        assert!(
            status.show_update_notice(),
            "known manual releases still need attention"
        );
        status.phase = HarnessUpdatePhase::Installing;
        assert!(status.show_update_notice());
    }
}

/// Host-owned context snapshot, replicated with the chat document.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsage {
    pub tokens: Option<u64>,
    pub window: Option<u64>,
}

impl ContextUsage {
    pub fn fraction(self) -> Option<f64> {
        Some(self.tokens? as f64 / self.window.filter(|n| *n > 0)? as f64)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SessionStats {
    pub turns: u64,
    pub steps: u64,
    pub llm_ms: u64,
    pub tool_ms: u64,
    pub ttft_ms: u64,
    pub ttft_count: u64,
    pub output_tokens: u64,
    /// LLM time of the segments that reported usage: harnesses without a
    /// usage report must not drag tokens per second toward zero.
    pub usage_llm_ms: u64,
    pub input_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_ttl_secs: Option<u32>,
    /// Epoch millis of the last turn that touched the prompt cache.
    pub cache_touched_at: Option<i64>,
    pub cost_micros: u64,
    /// Some of `cost_micros` was priced from public per-token rates rather
    /// than reported by the harness.
    pub cost_estimated: bool,
    /// Turns whose model has no known price.
    pub unpriced_turns: u64,
    /// Hash of the harness session whose running total `cost_session_micros`
    /// holds; see [`SessionStats::bank_cost`].
    pub cost_session: u64,
    pub cost_session_micros: u64,
}

fn cost_session_hash(session_id: &str) -> u64 {
    session_id
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        })
}

impl SessionStats {
    pub fn add(&mut self, other: &SessionStats) {
        self.turns += other.turns;
        self.steps += other.steps;
        self.llm_ms += other.llm_ms;
        self.tool_ms += other.tool_ms;
        self.ttft_ms += other.ttft_ms;
        self.ttft_count += other.ttft_count;
        self.output_tokens += other.output_tokens;
        self.usage_llm_ms += other.usage_llm_ms;
        self.input_tokens += other.input_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
        self.cache_write_tokens += other.cache_write_tokens;
        self.cache_ttl_secs = other.cache_ttl_secs.or(self.cache_ttl_secs);
        self.cache_touched_at = self.cache_touched_at.max(other.cache_touched_at);
        self.unpriced_turns += other.unpriced_turns;
        self.cost_micros += other.cost_micros;
        self.cost_estimated |= other.cost_estimated;
        if other.cost_session != 0 {
            self.cost_session = other.cost_session;
            self.cost_session_micros = other.cost_session_micros;
        }
    }

    pub fn note_cost(&mut self, session_id: &str, total_usd: f64) {
        self.cost_session = cost_session_hash(session_id);
        self.cost_session_micros = (total_usd.max(0.0) * 1e6).round() as u64;
    }

    /// Turns this segment's running total into the cost it added on top of
    /// `prior`. A fork (`--fork-session`) gets a new id but inherits its
    /// source's total (live 2.1.289), so resuming the banked session counts
    /// as continuing it; a total below the base means the counter restarted.
    pub fn bank_cost(&mut self, prior: &SessionStats, resumed_from: Option<&str>) {
        if self.cost_session == 0 {
            return;
        }
        let continues = self.cost_session == prior.cost_session
            || resumed_from.is_some_and(|id| cost_session_hash(id) == prior.cost_session);
        let base = if continues {
            prior.cost_session_micros
        } else {
            0
        };
        self.cost_micros = if self.cost_session_micros >= base {
            self.cost_session_micros - base
        } else {
            self.cost_session_micros
        };
    }

    pub fn cost_usd(&self) -> Option<f64> {
        (self.cost_session != 0 || self.cost_estimated).then(|| self.cost_micros as f64 / 1e6)
    }

    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_read_tokens + self.cache_write_tokens
    }

    pub fn avg_ttft_ms(&self) -> Option<u64> {
        (self.ttft_count > 0).then(|| self.ttft_ms / self.ttft_count)
    }

    pub fn tokens_per_second(&self) -> Option<f64> {
        (self.output_tokens > 0 && self.usage_llm_ms > 0)
            .then(|| self.output_tokens as f64 * 1000.0 / self.usage_llm_ms as f64)
    }

    pub fn cache_hit_rate(&self) -> Option<f64> {
        let cached = self.cache_read_tokens + self.cache_write_tokens;
        (cached > 0).then(|| self.cache_read_tokens as f64 / (cached + self.input_tokens) as f64)
    }

    /// Millis until the prompt cache expires; negative once it has.
    pub fn cache_remaining_ms(&self, now_ms: i64) -> Option<i64> {
        Some(self.cache_touched_at? + i64::from(self.cache_ttl_secs?) * 1000 - now_ms)
    }
}

#[cfg(test)]
mod session_cost_tests {
    use super::*;

    fn bank(
        stats: &mut SessionStats,
        session_id: &str,
        total_usd: f64,
        resumed_from: Option<&str>,
    ) {
        let mut segment = SessionStats::default();
        segment.note_cost(session_id, total_usd);
        segment.bank_cost(stats, resumed_from);
        stats.add(&segment);
    }

    #[test]
    fn running_totals_bank_their_deltas() {
        let mut stats = SessionStats::default();
        assert_eq!(stats.cost_usd(), None);
        bank(&mut stats, "a", 0.076908, None);
        bank(&mut stats, "a", 0.0811149, Some("a"));
        assert_eq!(stats.cost_micros, 81_115);
        stats.add(&SessionStats {
            turns: 1,
            unpriced_turns: 1,
            ..Default::default()
        });
        assert_eq!(stats.cost_micros, 81_115);
        bank(&mut stats, "fork", 0.0853264, Some("a"));
        assert_eq!(stats.cost_micros, 85_326);
        bank(&mut stats, "fresh", 0.05, None);
        assert_eq!(stats.cost_micros, 135_326);
        bank(&mut stats, "fresh", 0.01, Some("fresh"));
        assert_eq!(stats.cost_micros, 145_326);
        assert_eq!(stats.unpriced_turns, 1);
        assert_eq!(stats.cost_usd(), Some(0.145326));
    }

    #[test]
    fn copying_stats_keeps_the_banked_cost() {
        let mut source = SessionStats::default();
        bank(&mut source, "a", 0.3, None);
        bank(&mut source, "b", 0.2, None);
        let mut copy = SessionStats::default();
        copy.add(&source);
        assert_eq!(copy, source);
        assert_eq!(copy.cost_micros, 500_000);
    }

    #[test]
    fn estimated_segments_price_the_session_next_to_reported_totals() {
        let mut stats = SessionStats::default();
        stats.add(&SessionStats {
            cost_estimated: true,
            ..Default::default()
        });
        assert_eq!(stats.cost_usd(), Some(0.0));
        bank(&mut stats, "a", 0.5, None);
        stats.add(&SessionStats {
            cost_micros: 250_000,
            cost_estimated: true,
            ..Default::default()
        });
        bank(&mut stats, "a", 0.6, Some("a"));
        assert_eq!(stats.cost_usd(), Some(0.85));
    }
}

#[cfg(test)]
mod generated_image_tests {
    use super::*;

    #[test]
    fn generated_image_event_round_trip() {
        let event = AgentEvent::GeneratedImage {
            id: "item:image".into(),
            path: "/uploads/generated.png".into(),
            name: "generated.png".into(),
            mime_type: "image/png".into(),
        };
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["type"], "generatedImage");
        assert_eq!(value["mimeType"], "image/png");
        assert_eq!(serde_json::from_value::<AgentEvent>(value).unwrap(), event);
    }
}

#[cfg(test)]
mod todo_item_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn legacy_items_without_status_derive_from_done() {
        let done: TodoItem = serde_json::from_value(json!({ "text": "a", "done": true })).unwrap();
        let open: TodoItem = serde_json::from_value(json!({ "text": "b", "done": false })).unwrap();
        assert_eq!(done.status(), TodoStatus::Completed);
        assert_eq!(open.status(), TodoStatus::Pending);
        assert_eq!(open.status, None);
    }

    #[test]
    fn only_in_progress_adds_a_field_so_other_docs_are_unchanged() {
        let pending = serde_json::to_value(TodoItem::new("a", TodoStatus::Pending)).unwrap();
        let done = serde_json::to_value(TodoItem::new("b", TodoStatus::Completed)).unwrap();
        let active = serde_json::to_value(TodoItem::new("c", TodoStatus::InProgress)).unwrap();
        assert_eq!(pending, json!({ "text": "a", "done": false }));
        assert_eq!(done, json!({ "text": "b", "done": true }));
        assert_eq!(
            active,
            json!({ "text": "c", "done": false, "status": "inProgress" })
        );
    }

    #[test]
    fn unknown_status_falls_back_to_done_instead_of_failing_the_call() {
        let call: ToolCall = serde_json::from_value(json!({
            "kind": "todo",
            "items": [
                { "text": "a", "done": false, "status": "blocked" },
                { "text": "b", "done": true, "status": 7 },
            ]
        }))
        .unwrap();
        let ToolCall::Todo { items } = call else {
            panic!("expected a todo call");
        };
        assert_eq!(items[0].status(), TodoStatus::Pending);
        assert_eq!(items[1].status(), TodoStatus::Completed);
    }

    #[test]
    fn done_outranks_a_stale_status() {
        let item = TodoItem {
            text: "a".into(),
            done: true,
            status: Some(TodoStatus::InProgress),
        };
        assert_eq!(item.status(), TodoStatus::Completed);
    }

    #[test]
    fn harness_status_strings_decode() {
        assert_eq!(TodoStatus::parse("completed"), TodoStatus::Completed);
        assert_eq!(TodoStatus::parse("in_progress"), TodoStatus::InProgress);
        assert_eq!(TodoStatus::parse("inProgress"), TodoStatus::InProgress);
        assert_eq!(TodoStatus::parse("pending"), TodoStatus::Pending);
        assert_eq!(TodoStatus::parse("cancelled"), TodoStatus::Pending);
        assert_eq!(TodoStatus::parse(""), TodoStatus::Pending);
    }
}
