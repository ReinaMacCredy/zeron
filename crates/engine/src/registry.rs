//! HarnessRegistry — the engine's harness catalog: eager instances (mock) plus lazy
//! slots resolved on first use (claude-code spawns subprocess discovery; codex/cursor
//! later). Lazy slots carry a static descriptor so `ListHarnesses` never forces a spawn.
//!
//! Also owns the device's harness ENABLEMENT (Settings → Providers): which harnesses
//! this device's composer offers, persisted in `{data_dir}/harness-prefs.json`.
//! Per-device because CLI installs are — a viewer retargets the settings page at
//! another device and edits THAT device's set over the forwarded RPCs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};

use zeron_harness::{Harness, HarnessError, LaunchOverrides, mock::MockHarness};
use zeron_proto::{
    AccessMode, AgentEvent, DoneStatus, HarnessId, Model, ModelCatalog, ProviderInstance,
    ReasoningLevel, SteeringMode,
};

/// What `ListHarnesses` reports per harness.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessDescriptor {
    pub id: HarnessId,
    pub name: String,
    pub supports_steering: bool,
    pub steering_mode: SteeringMode,
    pub reasoning_levels: Vec<ReasoningLevel>,
    /// Whether the agent's CLI is present on the listing device (the settings
    /// enable-gate). Defaults true so catalogs from engines predating the
    /// field never read as uninstallable.
    #[serde(default = "default_installed")]
    pub installed: bool,
    /// Explicit CLI installation is available on this listing device.
    #[serde(default)]
    pub can_install: bool,
    /// Whether the listing device offers this harness (Settings → Providers).
    /// `None` — the catalog came from an engine predating the setting — means
    /// "unknown": consumers fall back to detection (see [`descriptor_enabled`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// The access modes this harness honors, least to most trusting. Empty
    /// — a catalog from an engine predating modes — offers no choice: those
    /// engines run every chat with full access.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub access_modes: Vec<AccessMode>,
    /// Whether this harness honors plan mode.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub supports_plan: bool,
}

impl HarnessDescriptor {
    /// Whether this harness can accept a prompt inside the turn that is
    /// currently running. Turn-boundary steering is still useful to the
    /// automatic queue drain, but it is not the non-interrupting "Steer"
    /// action exposed on an individual queued row.
    pub fn steers_mid_turn(&self) -> bool {
        self.supports_steering && self.steering_mode == SteeringMode::StepBoundary
    }
}

fn default_installed() -> bool {
    true
}

/// Whether detection alone may switch a harness on. The mock always "resolves"
/// but is a test rig, and counting it would let the last real agent be turned
/// off (the composer needs something it can actually run); the dev rig opts it
/// in through `COMET_HARNESS=mock`, which bypasses enablement in the pickers.
fn auto_enabled(id: HarnessId) -> bool {
    id != HarnessId::Mock
}

/// A descriptor's effective enabled flag. `None` — a catalog from an engine
/// predating the setting — falls back to detection, the same rule new devices
/// start from (see [`HarnessRegistry::enabled_set`]).
pub fn descriptor_enabled(descriptor: &HarnessDescriptor) -> bool {
    descriptor
        .enabled
        .unwrap_or_else(|| descriptor.installed && auto_enabled(descriptor.id))
}

fn describe(harness: &dyn Harness) -> HarnessDescriptor {
    HarnessDescriptor {
        id: harness.id(),
        name: harness.display_name().to_string(),
        supports_steering: harness.supports_steering(),
        steering_mode: harness.steering_mode(),
        reasoning_levels: harness.reasoning_levels().to_vec(),
        installed: harness.installed(),
        can_install: false,
        enabled: None,
        access_modes: harness.access_modes().to_vec(),
        supports_plan: harness.supports_plan(),
    }
}

/// The persisted shape of `harness-prefs.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct HarnessPrefsFile {
    /// The user's explicit opt-OUTS. Enablement otherwise follows detection,
    /// so the file only records "no" — an agent installed later turns itself
    /// on without a trip to Settings.
    disabled: Vec<HarnessId>,
    titles: TitleSettings,
    predictions: PredictionSettings,
    delegation: DelegationSettings,
    supervisor: zeron_proto::supervisor::SupervisorSettings,
    #[serde(alias = "idleCompaction")]
    compaction: CompactionSettings,
    tools: zeron_proto::tools::ToolSettings,
    review: Option<crate::review::ReviewSettings>,
    routing: crate::routing::RoutingSettings,
    sidebar: zeron_proto::SidebarSettings,
    system_prompt: String,
    seen_models: Vec<SeenModels>,
    /// The allow-list written back when enablement was a fixed default set.
    /// Read once, folded into `disabled`, and never written again.
    #[serde(skip_serializing)]
    enabled: Option<Vec<HarnessId>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SeenModels {
    harness: Option<HarnessId>,
    ids: Vec<String>,
    arrived: Vec<ArrivedModel>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ArrivedModel {
    id: String,
    at_ms: i64,
}

const NEW_MODEL_WINDOW_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// Per-device automatic session title preferences.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TitleSettings {
    /// None follows the session harness, using a supported installed fallback.
    pub harness: Option<HarnessId>,
    /// None selects the cheapest model offered by the selected harness.
    pub model: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PredictionSettings {
    pub enabled: bool,
    pub harness: Option<HarnessId>,
    pub model: Option<String>,
    /// Wins over `harness`/`model` when set.
    pub custom: Option<CustomPredictionApi>,
}

/// An OpenAI-compatible chat completions API. Its key lives in the Keychain.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CustomPredictionApi {
    pub base_url: String,
    pub model: String,
}

impl CustomPredictionApi {
    pub fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    fn normalized(self) -> Result<Self, String> {
        let base_url = self.base_url.trim().trim_end_matches('/').to_string();
        let url = reqwest::Url::parse(&base_url)
            .map_err(|_| "Enter the API base URL, like https://api.openai.com/v1".to_string())?;
        let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        match url.scheme() {
            "https" => {}
            "http" if loopback => {}
            _ => {
                return Err("Use an https URL; plain http only works for localhost".into());
            }
        }
        let model = self.model.trim().to_string();
        if model.is_empty() {
            return Err("Enter the model the API should use".into());
        }
        Ok(Self { base_url, model })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DelegationSettings {
    pub max_depth: u32,
    pub max_running_children: u32,
}

impl Default for DelegationSettings {
    fn default() -> Self {
        Self {
            max_depth: 2,
            max_running_children: 8,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompactionMode {
    /// Every message past `nudge_tokens` reminds the agent to decide.
    Threshold,
    /// Past `nudge_tokens`, a one-shot model decides before the message is sent.
    #[default]
    Classifier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JevSource {
    TypeSafe,
    OpenRouter,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CompactionSettings {
    pub enabled: bool,
    pub min_tokens: u64,
    /// 0 turns the nudge off.
    pub nudge_tokens: u64,
    pub mode: CompactionMode,
    /// None model picks a Sonnet-class model of the harness.
    pub classifier: TitleSettings,
    pub jev: Option<JevSource>,
    pub missions: bool,
}

impl Default for CompactionSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            min_tokens: 100_000,
            nudge_tokens: 150_000,
            mode: CompactionMode::default(),
            classifier: TitleSettings::default(),
            jev: None,
            missions: true,
        }
    }
}

/// Per-device native subagent preferences (Settings → Providers).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NativeSubagentSettings {
    /// Registered harnesses whose native subagents can be turned off.
    pub switchable: Vec<HarnessId>,
    /// The ones turned off: their runs delegate through side chats only.
    pub off: Vec<HarnessId>,
}

type Factory = Box<dyn Fn() -> Result<Arc<dyn Harness>, HarnessError> + Send + Sync>;
type InstalledProbe = Box<dyn Fn() -> bool + Send + Sync>;

/// The harnesses a Cloud device (`ZERON_DEVICE_PLATFORM=cloud`) registers —
/// the typed twin of [`zeron_proto::CLOUD_HARNESSES`]. Everything else is not
/// merely disabled but absent: never probed (`--version`, install detection,
/// model discovery, update checks). The rule is generic to the `cloud`
/// platform; the motivating example is a hosted sandbox whose preinstalled
/// CLIs are lazy installer shims (Boat's, at the time of writing) that
/// download and install the real agent when merely probed.
pub const CLOUD_HARNESS_IDS: &[HarnessId] = &[HarnessId::Codex, HarnessId::ClaudeCode];

/// Host-side environment for agent spawns (the Cloud device's credential
/// broker). Called right before a request reaches the harness; whatever it
/// puts in [`zeron_proto::RunRequest::env`] is never persisted.
#[async_trait::async_trait]
pub trait RunEnvironment: Send + Sync {
    /// `Err` = the run can't start: no credential for `harness` (the
    /// message says where to sign in).
    async fn prepare(
        &self,
        harness: HarnessId,
        request: &mut zeron_proto::RunRequest,
    ) -> Result<(), String>;

    /// Which credential a run of `harness` would use now (an opaque
    /// fingerprint), `None` when unknown. A parked session started with a
    /// different one is replaced before its next turn, so switching the
    /// account applies from the next turn.
    async fn credential(&self, _harness: HarnessId) -> Option<String> {
        None
    }

    /// A run of `harness` that started with `credential` reported the
    /// subscription's usage (see [`zeron_proto::AgentEvent::PlanUsage`]).
    async fn record_usage(
        &self,
        _harness: HarnessId,
        _credential: &str,
        _windows: Vec<zeron_proto::AgentUsageWindow>,
        _plan_label: Option<String>,
    ) {
    }
}

enum Slot {
    Ready(Arc<dyn Harness>),
    Lazy {
        descriptor: HarnessDescriptor,
        /// Re-run on every `descriptors()` call — a CLI installed mid-session
        /// shows up on the next settings/picker open, no restart needed.
        installed: InstalledProbe,
        factory: Factory,
    },
}

pub struct HarnessRegistry {
    pub(crate) installs: crate::rpc::Installations,
    slots: Mutex<HashMap<HarnessId, Slot>>,
    instances: Mutex<HashMap<String, (ProviderInstance, Arc<dyn Harness>)>>,
    order: Mutex<Vec<HarnessId>>,
    /// This device's enabled set; `None` inner value = the default set.
    prefs: Mutex<HarnessPrefsFile>,
    /// Where the prefs persist; `None` (tests, bare registries) skips writes.
    prefs_path: Mutex<Option<PathBuf>>,
    /// Fair per-harness execution gates. Runs and title generation hold a
    /// shared lease; an accepted update queues an exclusive lease. Tokio's
    /// write-preferring FIFO policy prevents a stream of new runs from
    /// starving an update that is already waiting.
    gates: Mutex<HashMap<HarnessId, Arc<tokio::sync::RwLock<()>>>>,
    pending_updates: Mutex<std::collections::HashSet<HarnessId>>,
    update_generation: tokio::sync::watch::Sender<u64>,
    /// Spawn-time environment provider (Cloud runner only).
    run_environment: Mutex<Option<Arc<dyn RunEnvironment>>>,
    /// Spawn-time environment of the agent accounts zeron keeps itself
    /// (Muse's per-account roots); after the Cloud broker, never over it.
    account_environment: Mutex<Option<Arc<dyn RunEnvironment>>>,
    /// Credits Zeron in agents' commits ([`crate::commit_credit`]).
    commit_credit: std::sync::OnceLock<Arc<crate::commit_credit::CommitCredit>>,
}

impl Default for HarnessRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl HarnessRegistry {
    pub async fn discover_models(
        &self,
        id: HarnessId,
    ) -> Result<Vec<zeron_proto::Model>, HarnessError> {
        let lease = Arc::new(self.execution_lease(id).await);
        self.discover_models_with_lease(id, lease).await
    }

    /// A caller such as titling already holds a lease. Reacquiring after an
    /// update queues would deadlock it against its own existing reader.
    pub(crate) async fn discover_models_with_lease(
        &self,
        id: HarnessId,
        lease: Arc<tokio::sync::OwnedRwLockReadGuard<()>>,
    ) -> Result<Vec<zeron_proto::Model>, HarnessError> {
        models_with_lease(self.resolve(id)?, lease).await
    }

    pub async fn discover_commands(
        &self,
        harness: Arc<dyn Harness>,
        cwd: &Path,
    ) -> Result<Vec<zeron_proto::SlashCommand>, HarnessError> {
        let cwd = cwd.to_owned();
        let lease = self.execution_lease(harness.id()).await;
        tokio::spawn(async move {
            let _lease = lease;
            harness.commands_for(&cwd).await
        })
        .await
        .map_err(|error| {
            HarnessError::Protocol(format!("command discovery task failed: {error}"))
        })?
    }

    pub async fn discover_tool_inventory(
        &self,
        id: HarnessId,
        cwd: &Path,
    ) -> Result<zeron_proto::tools::HarnessToolInventory, HarnessError> {
        let cwd = cwd.to_owned();
        let lease = self.execution_lease(id).await;
        let harness = self.resolve(id)?;
        tokio::spawn(async move {
            let _lease = lease;
            harness.tool_inventory(&cwd).await
        })
        .await
        .map_err(|error| HarnessError::Protocol(format!("tool discovery task failed: {error}")))?
    }

    pub async fn discover_skills(
        &self,
        harness: Arc<dyn Harness>,
        cwd: &Path,
    ) -> Result<Option<Vec<zeron_proto::invocation::Skill>>, HarnessError> {
        let cwd = cwd.to_owned();
        let lease = self.execution_lease(harness.id()).await;
        tokio::spawn(async move {
            // Retain the read lease through the adapter's deadline and cleanup,
            // even when the requesting RPC is dropped.
            let _lease = lease;
            harness.skills(&cwd).await
        })
        .await
        .map_err(|error| HarnessError::Protocol(format!("skill discovery task failed: {error}")))?
    }

    pub fn new() -> Self {
        let (update_generation, _) = tokio::sync::watch::channel(0);
        Self {
            installs: Default::default(),
            slots: Mutex::new(HashMap::new()),
            instances: Mutex::new(HashMap::new()),
            order: Mutex::new(Vec::new()),
            prefs: Mutex::new(HarnessPrefsFile::default()),
            prefs_path: Mutex::new(None),
            gates: Mutex::new(HashMap::new()),
            pending_updates: Mutex::new(std::collections::HashSet::new()),
            update_generation,
            run_environment: Mutex::new(None),
            account_environment: Mutex::new(None),
            commit_credit: std::sync::OnceLock::new(),
        }
    }

    /// Install the spawn-time environment provider (the Cloud runner's
    /// credential broker). Laptops never set one.
    pub fn set_run_environment(&self, environment: Arc<dyn RunEnvironment>) {
        *self
            .run_environment
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(environment);
    }

    /// Install the agent accounts' environment provider (every device).
    pub fn set_account_environment(&self, environment: Arc<dyn RunEnvironment>) {
        *self
            .account_environment
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(environment);
    }

    fn environments(&self) -> Vec<Arc<dyn RunEnvironment>> {
        [&self.run_environment, &self.account_environment]
            .into_iter()
            .filter_map(|slot| slot.lock().unwrap_or_else(PoisonError::into_inner).clone())
            .collect()
    }

    pub fn set_commit_credit(&self, credit: Arc<crate::commit_credit::CommitCredit>) {
        let _ = self.commit_credit.set(credit);
    }

    pub fn commit_credit(&self) -> Option<&Arc<crate::commit_credit::CommitCredit>> {
        self.commit_credit.get()
    }

    /// Fill `request.env` for a spawn of `harness` (no-op without a provider),
    /// and credit Zeron in its commits when that's on.
    pub async fn prepare_run(
        &self,
        harness: HarnessId,
        request: &mut zeron_proto::RunRequest,
    ) -> Result<(), String> {
        for environment in self.environments() {
            environment.prepare(harness, request).await?;
        }
        if let Some(credit) = self.commit_credit.get() {
            credit.apply(&mut request.env);
        }
        Ok(())
    }

    /// [`RunEnvironment::credential`] (`None` without a provider).
    pub async fn run_credential(&self, harness: HarnessId) -> Option<String> {
        for environment in self.environments() {
            if let Some(credential) = environment.credential(harness).await {
                return Some(credential);
            }
        }
        None
    }

    /// Hand a run's reported subscription usage to every provider (the
    /// account it belongs to records it).
    pub async fn record_plan_usage(
        &self,
        harness: HarnessId,
        credential: &str,
        windows: Vec<zeron_proto::AgentUsageWindow>,
        plan_label: Option<String>,
    ) {
        for environment in self.environments() {
            environment
                .record_usage(harness, credential, windows.clone(), plan_label.clone())
                .await;
        }
    }

    /// Whether `id` exists on this device at all (a Cloud device registers
    /// only [`CLOUD_HARNESS_IDS`]).
    pub fn is_registered(&self, id: HarnessId) -> bool {
        self.slots().contains_key(&id)
    }

    /// Drop every slot `keep` rejects. Lazy slots are dropped unresolved, so
    /// a removed harness is never probed.
    fn retain(&self, keep: impl Fn(HarnessId) -> bool) {
        self.slots().retain(|id, _| keep(*id));
        self.order().retain(|id| keep(*id));
    }

    fn gate(&self, id: HarnessId) -> Arc<tokio::sync::RwLock<()>> {
        self.gates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(id)
            .or_insert_with(|| Arc::new(tokio::sync::RwLock::new(())))
            .clone()
    }

    /// Shared lease held for the full lifetime of a harness subprocess.
    pub async fn execution_lease(&self, id: HarnessId) -> tokio::sync::OwnedRwLockReadGuard<()> {
        let mut updates = self.update_generation.subscribe();
        loop {
            // The marker closes the small begin-update → writer-future polling
            // gap. Watch retains a generation change, so an update finishing
            // between this check and `changed()` cannot lose the wakeup.
            if self.update_pending(id) {
                let _ = updates.changed().await;
                continue;
            }
            let lease = self.gate(id).read_owned().await;
            if !self.update_pending(id) {
                return lease;
            }
            drop(lease);
        }
    }

    /// Exclusive lease held from immediately before update mutation through
    /// post-install verification.
    pub async fn update_lease(&self, id: HarnessId) -> tokio::sync::OwnedRwLockWriteGuard<()> {
        self.gate(id).write_owned().await
    }

    /// Mark an accepted update before queueing its writer. Existing persistent
    /// runtimes use this signal to retire at their next turn boundary instead
    /// of parking indefinitely while the writer waits.
    pub fn begin_update(&self, id: HarnessId) {
        self.pending_updates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id);
    }

    pub fn end_update(&self, id: HarnessId) {
        self.pending_updates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id);
        self.update_generation
            .send_modify(|generation| *generation = generation.wrapping_add(1));
    }

    pub fn update_pending(&self, id: HarnessId) -> bool {
        self.pending_updates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&id)
    }

    /// Run a synchronous dispatch-boundary action only if no update has been
    /// accepted for this harness. Holding the marker lock through the action
    /// gives direct steering a strict order against `begin_update`: either the
    /// prompt is accepted first and belongs to the existing run, or it waits
    /// behind the update through the ordinary dispatch path.
    pub(crate) fn while_update_clear<T>(
        &self,
        id: HarnessId,
        action: impl FnOnce() -> T,
    ) -> Option<T> {
        let pending = self
            .pending_updates
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if pending.contains(&id) {
            None
        } else {
            Some(action())
        }
    }

    fn slots(&self) -> MutexGuard<'_, HashMap<HarnessId, Slot>> {
        self.slots.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn order(&self) -> MutexGuard<'_, Vec<HarnessId>> {
        self.order.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn prefs(&self) -> MutexGuard<'_, HarnessPrefsFile> {
        self.prefs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Load `harness-prefs.json` from the engine data dir and remember the
    /// path for writes. Corrupt/missing files fall back to the default set.
    pub fn load_prefs(&self, data_dir: &Path) {
        let path = data_dir.join("harness-prefs.json");
        let loaded = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<HarnessPrefsFile>(&text).ok())
            .unwrap_or_default();
        *self.prefs() = loaded;
        *self
            .prefs_path
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(path);
        self.migrate_legacy_prefs();
        self.adopt_ui_review_settings(data_dir);
    }

    /// Review settings lived in ui-settings.json before the engine owned them.
    fn adopt_ui_review_settings(&self, data_dir: &Path) {
        if self.prefs().review.is_some() {
            return;
        }
        let Some(ui) = std::fs::read_to_string(data_dir.join("ui-settings.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        else {
            return;
        };
        let reviewer = ui
            .get("reviewReviewer")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok());
        let instructions = ui.get("reviewInstructions").and_then(|v| v.as_str());
        if reviewer.is_none() && instructions.is_none() {
            return;
        }
        self.prefs().review = Some(crate::review::ReviewSettings {
            reviewer: reviewer.unwrap_or_default(),
            instructions: instructions.unwrap_or_default().to_string(),
        });
        self.persist_prefs();
    }

    /// Fold a legacy allow-list into the opt-out shape: a registered harness
    /// missing from it was a deliberate "no", so it stays off. Rewrites the
    /// file once, which is what lets later installs auto-enable.
    fn migrate_legacy_prefs(&self) {
        let legacy = { self.prefs().enabled.take() };
        let Some(legacy) = legacy else { return };
        let registered: Vec<HarnessId> = self.order().iter().copied().collect();
        let disabled: Vec<HarnessId> = registered
            .into_iter()
            .filter(|id| auto_enabled(*id) && !legacy.contains(id))
            .collect();
        self.prefs().disabled = disabled;
        self.persist_prefs();
    }

    pub fn ids(&self) -> Vec<HarnessId> {
        self.order().clone()
    }

    /// What this device offers: every harness whose CLI is FOUND, minus the
    /// user's explicit opt-outs. Enablement follows detection, so installing
    /// an agent is all it takes for it to appear in the composer.
    pub fn enabled_set(&self) -> Vec<HarnessId> {
        // Both guards drop before the installed probes run: `descriptors()`
        // takes `slots` then `order`, so holding `order` across a probe (which
        // takes `slots`) would invert the lock order.
        let registered: Vec<HarnessId> = self.order().iter().copied().collect();
        let disabled = self.prefs().disabled.clone();
        registered
            .into_iter()
            .filter(|id| auto_enabled(*id) && !disabled.contains(id) && self.installed_for(*id))
            .collect()
    }

    pub(crate) fn turned_off(&self, id: HarnessId) -> bool {
        self.prefs().disabled.contains(&id)
    }

    /// Why a chat with `config` cannot run here; a provider instance is checked at its own launch.
    pub(crate) fn cannot_run(&self, config: &zeron_proto::ChatConfig) -> Option<String> {
        if config.instance_id.is_some() {
            return None;
        }
        let name = config.harness.display_name();
        let other = match &config.profile_id {
            Some(profile) => format!("give the {profile} profile another agent"),
            None => "pick another agent".to_string(),
        };
        if !self.installed_for(config.harness) {
            Some(format!(
                "{name} is not installed on this Mac. Install it, or {other}."
            ))
        } else if self.turned_off(config.harness) {
            Some(format!(
                "{name} is turned off in Settings > Providers. Turn it on, or {other}."
            ))
        } else {
            None
        }
    }

    /// Whether this device's CLI probe passes for `id` (no spawn, no resolve).
    pub(crate) fn installed_for(&self, id: HarnessId) -> bool {
        match self.slots().get(&id) {
            Some(Slot::Ready(harness)) => harness.installed(),
            Some(Slot::Lazy { installed, .. }) => installed(),
            None => false,
        }
    }

    /// Flip one harness's enablement and persist. Refuses unknown harnesses,
    /// enabling one whose CLI is missing (the settings gate, enforced where
    /// the state lives), and disabling the last enabled harness — under
    /// detection-based enablement everything enabled is runnable, so the
    /// last one standing is always worth protecting (the composer needs
    /// something to run). A harness whose CLI is missing is never enabled
    /// in the first place, so turning it off is a clean no-op.
    pub fn set_enabled(&self, id: HarnessId, on: bool) -> Result<(), String> {
        if !self.slots().contains_key(&id) {
            return Err(format!("unknown harness {id:?}"));
        }
        if on && !auto_enabled(id) {
            return Err(format!("{id:?} cannot be enabled from Settings"));
        }
        if on && !self.installed_for(id) {
            return Err(format!("{id:?} CLI is not installed on this device"));
        }
        let enabled = self.enabled_set();
        match (on, enabled.contains(&id)) {
            (true, false) => {
                let mut prefs = self.prefs();
                prefs.disabled.retain(|h| *h != id);
            }
            (false, true) => {
                if enabled.len() == 1 {
                    return Err("cannot disable the last enabled harness".into());
                }
                let mut prefs = self.prefs();
                prefs.disabled.push(id);
            }
            _ => return Ok(()),
        }
        self.persist_prefs();
        Ok(())
    }

    /// Best-effort atomic write (temp + rename, the ui-settings pattern).
    fn persist_prefs(&self) {
        let Some(path) = self
            .prefs_path
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        else {
            return;
        };
        let json = match serde_json::to_string_pretty(&*self.prefs()) {
            Ok(json) => json,
            Err(err) => {
                tracing::warn!(error = %err, "harness-prefs serialize failed");
                return;
            }
        };
        let tmp = path.with_extension("json.tmp");
        if let Err(err) = std::fs::write(&tmp, json).and_then(|()| std::fs::rename(&tmp, &path)) {
            tracing::warn!(error = %err, "harness-prefs save failed");
        }
    }

    pub fn title_settings(&self) -> TitleSettings {
        self.prefs().titles.clone()
    }

    pub fn set_title_settings(&self, mut settings: TitleSettings) -> Result<(), String> {
        if let Some(id) = settings.harness {
            if !zeron_harness::supports_titles(id) || !self.enabled_set().contains(&id) {
                return Err("Choose an enabled harness that supports title generation".into());
            }
        } else if settings.model.is_some() {
            return Err("Choose a title harness before choosing a model".into());
        }
        settings.model = settings.model.filter(|model| !model.trim().is_empty());
        self.prefs().titles = settings;
        self.persist_prefs();
        Ok(())
    }

    pub fn prediction_settings(&self) -> PredictionSettings {
        self.prefs().predictions.clone()
    }

    pub fn set_prediction_settings(&self, mut settings: PredictionSettings) -> Result<(), String> {
        if let Some(id) = settings.harness {
            if !zeron_harness::supports_titles(id) || !self.enabled_set().contains(&id) {
                return Err("Choose an enabled harness that supports predictions".into());
            }
        } else if settings.model.is_some() {
            return Err("Choose a prediction harness before choosing a model".into());
        }
        settings.model = settings.model.filter(|model| !model.trim().is_empty());
        settings.custom = settings
            .custom
            .map(CustomPredictionApi::normalized)
            .transpose()?;
        self.prefs().predictions = settings;
        self.persist_prefs();
        Ok(())
    }

    pub fn data_dir(&self) -> Option<PathBuf> {
        self.prefs_path
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(|path| path.parent().map(Path::to_path_buf))
    }

    pub fn delegation_settings(&self) -> DelegationSettings {
        self.prefs().delegation
    }

    pub fn set_delegation_settings(&self, settings: DelegationSettings) -> Result<(), String> {
        if settings.max_depth == 0 || settings.max_running_children == 0 {
            return Err("Delegation limits must be at least 1".into());
        }
        self.prefs().delegation = settings;
        self.persist_prefs();
        Ok(())
    }

    pub fn routing_settings(&self) -> crate::routing::RoutingSettings {
        self.prefs().routing.clone()
    }

    pub fn set_routing_settings(
        &self,
        settings: crate::routing::RoutingSettings,
    ) -> Result<(), String> {
        for target in settings.targets() {
            crate::routing::check_target(target, |id| self.slots().contains_key(&id))?;
        }
        self.prefs().routing = settings;
        self.persist_prefs();
        Ok(())
    }

    pub fn supervisor_settings(&self) -> zeron_proto::supervisor::SupervisorSettings {
        self.prefs().supervisor
    }

    pub fn set_supervisor_settings(&self, settings: zeron_proto::supervisor::SupervisorSettings) {
        self.prefs().supervisor = settings;
        self.persist_prefs();
    }

    pub fn compaction_settings(&self) -> CompactionSettings {
        self.prefs().compaction.clone()
    }

    pub fn set_compaction_settings(&self, settings: CompactionSettings) -> Result<(), String> {
        if settings.min_tokens == 0 {
            return Err("Compaction thresholds are out of range".into());
        }
        self.prefs().compaction = settings;
        self.persist_prefs();
        Ok(())
    }

    pub fn sidebar_settings(&self) -> zeron_proto::SidebarSettings {
        self.prefs().sidebar
    }

    pub fn system_prompt(&self) -> String {
        self.prefs().system_prompt.clone()
    }

    pub fn set_system_prompt(&self, prompt: &str) -> Result<(), String> {
        let prompt = prompt.trim();
        if prompt.chars().count() > crate::agent_profiles::MAX_INSTRUCTIONS_CHARS {
            return Err(format!(
                "The system prompt must not exceed {} characters",
                crate::agent_profiles::MAX_INSTRUCTIONS_CHARS
            ));
        }
        self.prefs().system_prompt = prompt.to_string();
        self.persist_prefs();
        Ok(())
    }

    pub fn review_settings(&self) -> crate::review::ReviewSettings {
        self.prefs().review.clone().unwrap_or_default()
    }

    pub fn set_review_settings(
        &self,
        settings: crate::review::ReviewSettings,
    ) -> Result<(), String> {
        if settings.instructions.chars().count() > crate::agent_profiles::MAX_INSTRUCTIONS_CHARS {
            return Err(format!(
                "Review instructions must not exceed {} characters",
                crate::agent_profiles::MAX_INSTRUCTIONS_CHARS
            ));
        }
        self.prefs().review = Some(crate::review::ReviewSettings {
            instructions: settings.instructions.trim().to_string(),
            ..settings
        });
        self.persist_prefs();
        Ok(())
    }

    pub fn tool_settings(&self) -> zeron_proto::tools::ToolSettings {
        self.prefs().tools.clone()
    }

    pub fn tool_settings_state(
        &self,
        devices_on: bool,
        chat_disabled: &[String],
    ) -> zeron_proto::tools::ToolSettingsState {
        let also_off: &[zeron_proto::tools::ToolGroup] = if devices_on {
            &[]
        } else {
            &[zeron_proto::tools::ToolGroup::Devices]
        };
        let mut state = self.tool_settings().for_chat(chat_disabled).state(also_off);
        state.settings = self.tool_settings();
        state
    }

    pub fn set_tool_settings(&self, settings: zeron_proto::tools::ToolSettings) {
        self.prefs().tools = settings;
        self.persist_prefs();
    }

    /// The first listing for a harness seeds silently; otherwise every model
    /// on a fresh install would read as new.
    pub fn model_catalog(
        &self,
        harness: HarnessId,
        models: Vec<Model>,
        now_ms: i64,
    ) -> ModelCatalog {
        let mut prefs = self.prefs();
        let (new_ids, announce_ids, changed) = match prefs
            .seen_models
            .iter_mut()
            .find(|entry| entry.harness == Some(harness))
        {
            Some(entry) => {
                let announce_ids: Vec<String> = models
                    .iter()
                    .map(|model| &model.id)
                    .filter(|id| !entry.ids.contains(id))
                    .cloned()
                    .collect();
                // Ids the listing dropped stay: a model returning after a
                // failed probe is not news.
                entry.ids.extend(announce_ids.iter().cloned());
                let before = entry.arrived.len();
                entry
                    .arrived
                    .retain(|arrived| now_ms - arrived.at_ms < NEW_MODEL_WINDOW_MS);
                let pruned = entry.arrived.len() != before;
                entry
                    .arrived
                    .extend(announce_ids.iter().map(|id| ArrivedModel {
                        id: id.clone(),
                        at_ms: now_ms,
                    }));
                let new_ids = models
                    .iter()
                    .map(|model| &model.id)
                    .filter(|id| entry.arrived.iter().any(|arrived| &arrived.id == *id))
                    .cloned()
                    .collect();
                let changed = pruned || !announce_ids.is_empty();
                (new_ids, announce_ids, changed)
            }
            None => {
                prefs.seen_models.push(SeenModels {
                    harness: Some(harness),
                    ids: models.iter().map(|model| model.id.clone()).collect(),
                    arrived: Vec::new(),
                });
                (Vec::new(), Vec::new(), true)
            }
        };
        drop(prefs);
        if changed {
            self.persist_prefs();
        }
        ModelCatalog {
            models,
            new_ids,
            announce_ids,
        }
    }

    /// Whether runs of `id` keep the provider's own subagent tools: off only
    /// under the routing native policy Off, for a driver that can launch
    /// without them.
    pub fn native_subagents(&self, id: HarnessId) -> bool {
        !(zeron_harness::switches_native_subagents(id)
            && self.prefs().routing.native == crate::routing::NativePolicy::Off)
    }

    pub fn native_subagent_settings(&self) -> NativeSubagentSettings {
        let switchable: Vec<HarnessId> = self
            .order()
            .iter()
            .copied()
            .filter(|id| zeron_harness::switches_native_subagents(*id))
            .collect();
        let off = switchable
            .iter()
            .copied()
            .filter(|id| !self.native_subagents(*id))
            .collect();
        NativeSubagentSettings { switchable, off }
    }

    /// The upstream per-harness switch, kept as a view of the routing native
    /// policy so there is one setting: off sets the policy to Off, on puts
    /// back the default.
    pub fn set_native_subagents(&self, id: HarnessId, on: bool) -> Result<(), String> {
        if !zeron_harness::switches_native_subagents(id) {
            return Err(format!("{id:?} has no native subagents to turn off"));
        }
        {
            let mut prefs = self.prefs();
            let off = prefs.routing.native == crate::routing::NativePolicy::Off;
            prefs.routing.native = match (on, off) {
                (true, true) => crate::routing::NativePolicy::default(),
                (false, false) => crate::routing::NativePolicy::Off,
                _ => return Ok(()),
            };
        }
        self.persist_prefs();
        Ok(())
    }

    pub fn register(&self, harness: Arc<dyn Harness>) {
        let id = harness.id();
        if self.slots().insert(id, Slot::Ready(harness)).is_none() {
            self.order().push(id);
        }
    }

    /// Register a slot resolved on first `resolve` (the factory result is
    /// cached). `installed` is the CLI-presence probe run per `descriptors()`
    /// call; it must never spawn.
    pub fn register_lazy(
        &self,
        descriptor: HarnessDescriptor,
        installed: InstalledProbe,
        factory: Factory,
    ) {
        let id = descriptor.id;
        if self
            .slots()
            .insert(
                id,
                Slot::Lazy {
                    descriptor,
                    installed,
                    factory,
                },
            )
            .is_none()
        {
            self.order().push(id);
        }
    }

    pub fn resolve(&self, id: HarnessId) -> Result<Arc<dyn Harness>, HarnessError> {
        let mut slots = self.slots();
        match slots.get(&id) {
            Some(Slot::Ready(harness)) => Ok(harness.clone()),
            Some(Slot::Lazy { factory, .. }) => {
                let harness = factory()?;
                slots.insert(id, Slot::Ready(harness.clone()));
                Ok(harness)
            }
            None => Err(HarnessError::NotInstalled(format!("{id:?}"))),
        }
    }

    pub(crate) fn resolve_instance(
        &self,
        instance: &ProviderInstance,
        data_dir: &Path,
    ) -> Result<Arc<dyn Harness>, crate::EngineError> {
        let mut instances = self
            .instances
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some((cached, harness)) = instances.get(&instance.id)
            && crate::provider_instances::same_launch(cached, instance)
        {
            return Ok(harness.clone());
        }
        let harness: Arc<dyn Harness> = match instance.driver {
            driver if crate::provider_instances::runs_over_acp(driver) => {
                Arc::new(crate::provider_instances::acp_harness(instance, data_dir)?)
            }
            driver => {
                let launch = crate::provider_instances::launch_overrides(instance, data_dir)?;
                native_instance(driver, launch).map_err(|reason| {
                    crate::EngineError::Other(format!(
                        "Provider instance '{}': {reason}",
                        instance.id
                    ))
                })?
            }
        };
        instances.insert(instance.id.clone(), (instance.clone(), harness.clone()));
        Ok(harness)
    }

    #[cfg(test)]
    pub(crate) fn register_instance(&self, instance: ProviderInstance, harness: Arc<dyn Harness>) {
        self.instances
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(instance.id.clone(), (instance, harness));
    }

    /// Installed and enabled harnesses a child may run on; the mock only under `ZERON_HARNESS=mock`.
    pub fn offered(&self) -> Vec<HarnessDescriptor> {
        let mock = std::env::var("ZERON_HARNESS").is_ok_and(|v| v.trim() == "mock");
        self.descriptors()
            .into_iter()
            .filter(|d| {
                d.installed
                    && (descriptor_enabled(d) || (mock && d.id == HarnessId::Mock))
                    && (mock || d.id != HarnessId::Mock)
            })
            .collect()
    }

    /// Catalog for `ListHarnesses` — never forces a lazy resolve.
    pub fn descriptors(&self) -> Vec<HarnessDescriptor> {
        let enabled = self.enabled_set();
        let slots = self.slots();
        self.order()
            .iter()
            .filter_map(|id| {
                let mut descriptor = match slots.get(id) {
                    Some(Slot::Ready(harness)) => describe(harness.as_ref()),
                    Some(Slot::Lazy {
                        descriptor,
                        installed,
                        ..
                    }) => HarnessDescriptor {
                        installed: installed(),
                        ..descriptor.clone()
                    },
                    None => return None,
                };
                descriptor.enabled = Some(enabled.contains(id));
                descriptor.can_install = zeron_harness::install::can_install(*id);
                Some(descriptor)
            })
            .collect()
    }
}

fn native_instance(driver: HarnessId, launch: LaunchOverrides) -> Result<Arc<dyn Harness>, String> {
    let name = driver.display_name();
    Ok(match driver {
        HarnessId::ClaudeCode => Arc::new(zeron_harness::ClaudeHarness::new().with_launch(launch)),
        HarnessId::Codex => Arc::new(zeron_harness::CodexHarness::new().with_launch(launch)),
        HarnessId::Pi => Arc::new(zeron_harness::PiHarness::new().with_launch(launch)),
        HarnessId::Opencode => Arc::new(zeron_harness::OpencodeHarness::new().with_launch(launch)),
        HarnessId::Cursor => Arc::new(zeron_harness::CursorHarness::new().with_launch(launch)),
        _ => return Err(format!("{name} does not run as a provider instance yet")),
    })
}

pub(crate) async fn models_with_lease(
    harness: Arc<dyn Harness>,
    lease: Arc<tokio::sync::OwnedRwLockReadGuard<()>>,
) -> Result<Vec<zeron_proto::Model>, HarnessError> {
    tokio::spawn(async move {
        // An RPC cancellation must not drop the gate before the probe's
        // own deadline and child cleanup finish.
        let _lease = lease;
        harness.models().await
    })
    .await
    .map_err(|error| HarnessError::Protocol(format!("model discovery task failed: {error}")))?
}

/// The production registry for this engine's platform (see
/// [`registry_for_platform`]).
pub fn default_registry() -> HarnessRegistry {
    registry_for_platform(&crate::runner::device_platform())
}

/// The production registry: MockHarness (hidden from production pickers) plus a lazy
/// `claude-code` slot resolved through `zeron_harness` on first use (subprocess
/// discovery only happens when a run/model call actually needs it). On a
/// `cloud` platform only [`CLOUD_HARNESS_IDS`] (plus the test-only mock)
/// are registered.
pub fn registry_for_platform(platform: &str) -> HarnessRegistry {
    let registry = full_registry();
    if platform == zeron_proto::CLOUD_PLATFORM {
        registry.retain(|id| id == HarnessId::Mock || CLOUD_HARNESS_IDS.contains(&id));
    }
    registry
}

fn full_registry() -> HarnessRegistry {
    // Warm the login-shell PATH snapshot in the background so the first
    // claude/codex resolve doesn't pay the shell-startup latency inline.
    zeron_harness::shell_env::prewarm();
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(MockHarness {
        script: vec![
            AgentEvent::TextDelta {
                text: "## Streaming pipeline\n\nEvery turn flows through the same path:\n\n".into(),
            },
            AgentEvent::TextDelta {
                text: "1. **Doc command** — the composer queues a durable `run` entry\n2. **Host executor** — the chat's host device marks it processed, then dispatches\n3. **Fold** — events fold into parts and diff into the Loro doc every 120ms\n\n".into(),
            },
            AgentEvent::ToolCall {
                id: "mock-tool-1".into(),
                call: zeron_proto::ToolCall::Exec {
                    command: "cargo test --workspace".into(),
                },
            },
            AgentEvent::ToolResult {
                id: "mock-tool-1".into(),
                is_error: false,
                output: None,
                diff: None,
            },
            AgentEvent::ToolCall {
                id: "mock-tool-2".into(),
                call: zeron_proto::ToolCall::Exec {
                    command: "git log -5 --oneline --decorate && git merge-base HEAD origin/main"
                        .into(),
                },
            },
            AgentEvent::ToolResult {
                id: "mock-tool-2".into(),
                is_error: false,
                output: None,
                diff: None,
            },
            AgentEvent::TextDelta {
                text: "The `SegmentWriter` appends into `LoroText` so the oplog stays RLE-merged:\n\n```rust\nfolded = fold_event_into_parts(&folded, &event);\nwriter.sync(&folded)?; // 120ms coalesced commits\n```\n\nSynced to every device through the session room. *Mock harness reporting in.*".into(),
            },
            AgentEvent::Done {
                status: DoneStatus::Completed,
                result: None,
                error: None,
                session_id: None,
            },
        ],
    }));
    registry.register_lazy(
        HarnessDescriptor {
            id: HarnessId::ClaudeCode,
            name: "Claude Code".into(),
            supports_steering: true,
            steering_mode: SteeringMode::StepBoundary,
            // Must mirror ClaudeHarness exactly — the descriptor-stability
            // rule (see the codex test below).
            reasoning_levels: vec![
                ReasoningLevel::Low,
                ReasoningLevel::Medium,
                ReasoningLevel::High,
                ReasoningLevel::XHigh,
                ReasoningLevel::Max,
            ],
            installed: true,
            can_install: false,
            enabled: None,
            access_modes: AccessMode::ALL.to_vec(),
            supports_plan: true,
        },
        Box::new(|| zeron_harness::ClaudeHarness::new().installed()),
        Box::new(|| Ok(Arc::new(zeron_harness::ClaudeHarness::new()) as Arc<dyn Harness>)),
    );
    // Codex, same lazy pattern: the static descriptor mirrors AcpHarness::codex()
    // exactly (`describe()` after the first resolve must not change the
    // catalog entry) — "Codex" per the original HARNESS_LABEL, StepBoundary
    // steering via native `turn/steer`, and the unified reasoning ladder from
    // zeron_harness::codex::catalog. CLI discovery only happens when a
    // run/model call actually resolves the slot.
    registry.register_lazy(
        HarnessDescriptor {
            id: HarnessId::Codex,
            name: "Codex".into(),
            supports_steering: true,
            steering_mode: SteeringMode::StepBoundary,
            reasoning_levels: vec![
                ReasoningLevel::Minimal,
                ReasoningLevel::Low,
                ReasoningLevel::Medium,
                ReasoningLevel::High,
                ReasoningLevel::XHigh,
                ReasoningLevel::Max,
                ReasoningLevel::Ultra,
            ],
            installed: true,
            can_install: false,
            enabled: None,
            access_modes: AccessMode::ALL.to_vec(),
            supports_plan: true,
        },
        Box::new(|| zeron_harness::CodexHarness::new().installed()),
        Box::new(|| Ok(Arc::new(zeron_harness::CodexHarness::new()) as Arc<dyn Harness>)),
    );
    // Cursor via the pinned @cursor/sdk shim (NOT ACP — that surface strips
    // subagent transcripts), same lazy pattern: the static descriptor mirrors
    // CursorHarness exactly. Native step-boundary steering; no effort ladder.
    registry.register_lazy(
        HarnessDescriptor {
            id: HarnessId::Cursor,
            name: "Cursor".into(),
            supports_steering: true,
            steering_mode: SteeringMode::StepBoundary,
            reasoning_levels: Vec::new(),
            installed: true,
            can_install: false,
            enabled: None,
            access_modes: vec![AccessMode::FullAccess],
            supports_plan: false,
        },
        Box::new(|| zeron_harness::CursorHarness::new().installed()),
        Box::new(|| Ok(Arc::new(zeron_harness::CursorHarness::new()) as Arc<dyn Harness>)),
    );
    // Devin over ACP (`devin acp`), same lazy pattern: the static descriptor
    // mirrors AcpHarness::devin() exactly. No steering extension (turn
    // boundaries) and no effort ladder — Devin bakes effort into the
    // advertised model ids instead of a `thought_level` option.
    registry.register_lazy(
        HarnessDescriptor {
            id: HarnessId::Devin,
            name: "Devin".into(),
            supports_steering: true,
            steering_mode: SteeringMode::TurnBoundary,
            reasoning_levels: Vec::new(),
            installed: true,
            can_install: false,
            enabled: None,
            access_modes: vec![AccessMode::FullAccess],
            supports_plan: false,
        },
        Box::new(|| zeron_harness::AcpHarness::devin().installed()),
        Box::new(|| Ok(Arc::new(zeron_harness::AcpHarness::devin()) as Arc<dyn Harness>)),
    );
    // Grok Build over ACP, same lazy pattern: the static descriptor mirrors
    // AcpHarness::grok() exactly. No `_session/steering` extension yet, so
    // steers deliver at turn boundaries; the effort ladder applies per
    // session via the `thought_level` config option.
    registry.register_lazy(
        HarnessDescriptor {
            id: HarnessId::Grok,
            name: "Grok".into(),
            supports_steering: true,
            steering_mode: SteeringMode::TurnBoundary,
            reasoning_levels: vec![
                ReasoningLevel::Low,
                ReasoningLevel::Medium,
                ReasoningLevel::High,
            ],
            installed: true,
            can_install: false,
            enabled: None,
            access_modes: vec![AccessMode::FullAccess],
            supports_plan: false,
        },
        Box::new(|| zeron_harness::AcpHarness::grok().installed()),
        Box::new(|| Ok(Arc::new(zeron_harness::AcpHarness::grok()) as Arc<dyn Harness>)),
    );
    // Hermes Agent over ACP (`hermes acp`), same lazy pattern: the static
    // descriptor mirrors AcpHarness::hermes() exactly. No steering extension
    // (turn boundaries) and no effort ladder — Hermes exposes no effort
    // config over ACP today (hybrid reasoning is model-internal).
    registry.register_lazy(
        HarnessDescriptor {
            id: HarnessId::Hermes,
            name: "Hermes".into(),
            supports_steering: true,
            steering_mode: SteeringMode::TurnBoundary,
            reasoning_levels: Vec::new(),
            installed: true,
            can_install: false,
            enabled: None,
            access_modes: vec![AccessMode::FullAccess],
            supports_plan: false,
        },
        Box::new(|| zeron_harness::AcpHarness::hermes().installed()),
        Box::new(|| Ok(Arc::new(zeron_harness::AcpHarness::hermes()) as Arc<dyn Harness>)),
    );
    // Native Pi RPC. Thinking levels are discovered per model.
    registry.register_lazy(
        HarnessDescriptor {
            id: HarnessId::Pi,
            name: "Pi".into(),
            supports_steering: true,
            steering_mode: SteeringMode::StepBoundary,
            reasoning_levels: Vec::new(),
            installed: true,
            can_install: false,
            enabled: None,
            access_modes: vec![AccessMode::FullAccess],
            supports_plan: false,
        },
        Box::new(|| zeron_harness::PiHarness::new().installed()),
        Box::new(|| Ok(Arc::new(zeron_harness::PiHarness::new()) as Arc<dyn Harness>)),
    );
    // opencode over its NATIVE HTTP/SSE protocol (the one the opencode
    // desktop app speaks — `opencode serve` + the /global/event bus), same
    // lazy pattern: the static descriptor mirrors OpencodeHarness exactly.
    // Turn-boundary steering; the effort ladder rides model VARIANTS (the
    // run sends the first advertised variant id for the picked level).
    registry.register_lazy(
        HarnessDescriptor {
            id: HarnessId::Opencode,
            name: "OpenCode".into(),
            supports_steering: true,
            steering_mode: SteeringMode::TurnBoundary,
            reasoning_levels: vec![
                ReasoningLevel::Low,
                ReasoningLevel::Medium,
                ReasoningLevel::High,
                ReasoningLevel::XHigh,
                ReasoningLevel::Max,
            ],
            installed: true,
            can_install: false,
            enabled: None,
            access_modes: AccessMode::ALL.to_vec(),
            supports_plan: true,
        },
        Box::new(|| zeron_harness::OpencodeHarness::new().installed()),
        Box::new(|| Ok(Arc::new(zeron_harness::OpencodeHarness::new()) as Arc<dyn Harness>)),
    );
    // antigravity over acp (google's agy_acp_server), same lazy pattern: the
    // static descriptor mirrors AcpHarness::antigravity() exactly. No steering
    // extension (turn boundaries), and effort is baked into the model ids, so
    // the ladder lives on each model rather than the harness.
    registry.register_lazy(
        HarnessDescriptor {
            id: HarnessId::Antigravity,
            name: "Antigravity".into(),
            supports_steering: true,
            steering_mode: SteeringMode::TurnBoundary,
            reasoning_levels: Vec::new(),
            installed: true,
            can_install: false,
            enabled: None,
            access_modes: AccessMode::ALL.to_vec(),
            supports_plan: false,
        },
        Box::new(|| zeron_harness::AcpHarness::antigravity().installed()),
        Box::new(|| Ok(Arc::new(zeron_harness::AcpHarness::antigravity()) as Arc<dyn Harness>)),
    );
    // Muse Code over its native session protocol (`muse serve`), same lazy
    // pattern: the static descriptor mirrors MuseHarness exactly.
    registry.register_lazy(
        HarnessDescriptor {
            id: HarnessId::Muse,
            name: "Muse Code".into(),
            supports_steering: true,
            steering_mode: SteeringMode::StepBoundary,
            reasoning_levels: Vec::new(),
            installed: true,
            can_install: false,
            enabled: None,
            access_modes: vec![AccessMode::Supervised, AccessMode::FullAccess],
            supports_plan: false,
        },
        Box::new(|| zeron_harness::MuseHarness::new().installed()),
        Box::new(|| Ok(Arc::new(zeron_harness::MuseHarness::new()) as Arc<dyn Harness>)),
    );
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str) -> Model {
        Model {
            id: id.into(),
            label: id.into(),
            description: None,
            reasoning_levels: Vec::new(),
            options: Vec::new(),
        }
    }

    #[test]
    fn new_models_announce_once_and_stay_new_for_the_window() {
        let registry = HarnessRegistry::new();
        let list = |ids: &[&str]| ids.iter().map(|id| model(id)).collect::<Vec<_>>();
        let day = 24 * 60 * 60 * 1000;

        let seeded = registry.model_catalog(
            HarnessId::ClaudeCode,
            list(&["claude-opus-5", "claude-sonnet-5"]),
            0,
        );
        assert!(seeded.new_ids.is_empty() && seeded.announce_ids.is_empty());

        let current = ["claude-opus-5-5", "claude-opus-5", "claude-sonnet-5"];
        let arrived = registry.model_catalog(HarnessId::ClaudeCode, list(&current), day);
        assert_eq!(arrived.announce_ids, vec!["claude-opus-5-5".to_string()]);
        assert_eq!(arrived.new_ids, vec!["claude-opus-5-5".to_string()]);

        // Reopening the picker must not clear the badge or re-announce.
        let reopened = registry.model_catalog(HarnessId::ClaudeCode, list(&current), 3 * day);
        assert!(reopened.announce_ids.is_empty());
        assert_eq!(reopened.new_ids, vec!["claude-opus-5-5".to_string()]);

        let expired = registry.model_catalog(HarnessId::ClaudeCode, list(&current), 9 * day);
        assert!(expired.new_ids.is_empty() && expired.announce_ids.is_empty());

        // A model the listing drops and later restores is not news again.
        registry.model_catalog(HarnessId::ClaudeCode, list(&["claude-opus-5-5"]), 10 * day);
        let restored = registry.model_catalog(HarnessId::ClaudeCode, list(&current), 10 * day);
        assert!(restored.announce_ids.is_empty());

        // Newness is tracked per harness, so another agent still seeds.
        let other = registry.model_catalog(HarnessId::Codex, list(&["gpt-6"]), 10 * day);
        assert!(other.new_ids.is_empty() && other.announce_ids.is_empty());
    }

    #[test]
    fn mid_turn_steering_requires_support_and_a_step_boundary() {
        let mut descriptor = HarnessDescriptor {
            id: HarnessId::Mock,
            name: "Mock".into(),
            supports_steering: true,
            steering_mode: SteeringMode::StepBoundary,
            reasoning_levels: Vec::new(),
            installed: true,
            can_install: false,
            enabled: Some(true),
            access_modes: Vec::new(),
            supports_plan: false,
        };
        assert!(descriptor.steers_mid_turn());

        descriptor.steering_mode = SteeringMode::TurnBoundary;
        assert!(!descriptor.steers_mid_turn());

        descriptor.steering_mode = SteeringMode::StepBoundary;
        descriptor.supports_steering = false;
        assert!(!descriptor.steers_mid_turn());
    }

    #[test]
    fn lazy_slot_lists_without_resolving() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let registry = HarnessRegistry::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        registry.register_lazy(
            HarnessDescriptor {
                id: HarnessId::Mock,
                name: "Lazy Mock".into(),
                supports_steering: true,
                steering_mode: SteeringMode::StepBoundary,
                reasoning_levels: vec![],
                installed: true,
                can_install: false,
                enabled: None,
                access_modes: Vec::new(),
                supports_plan: false,
            },
            Box::new(|| false),
            Box::new(move || {
                counted.fetch_add(1, Ordering::SeqCst);
                Err(HarnessError::NotInstalled("nope".into()))
            }),
        );
        let listed = registry.descriptors();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Lazy Mock");
        // The listing runs the probe, not the stored placeholder.
        assert!(!listed[0].installed);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "listing must not force a resolve"
        );
        assert!(registry.resolve(HarnessId::Mock).is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// The composer offers what the catalog lists before any harness has
    /// resolved, so each lazy descriptor must offer the modes its harness
    /// honors once it does.
    #[test]
    fn lazy_descriptors_offer_the_modes_their_harness_honors() {
        let registry = full_registry();
        for described in registry.descriptors() {
            let harness = registry.resolve(described.id).unwrap();
            assert_eq!(
                (described.access_modes.as_slice(), described.supports_plan),
                (harness.access_modes(), harness.supports_plan()),
                "{:?}",
                described.id
            );
        }
    }

    #[test]
    fn default_registry_lists_mock_claude_codex_and_grok_slots() {
        let registry = default_registry();
        let ids: Vec<HarnessId> = registry.descriptors().iter().map(|d| d.id).collect();
        assert_eq!(
            ids,
            vec![
                HarnessId::Mock,
                HarnessId::ClaudeCode,
                HarnessId::Codex,
                HarnessId::Cursor,
                HarnessId::Devin,
                HarnessId::Grok,
                HarnessId::Hermes,
                HarnessId::Pi,
                HarnessId::Opencode,
                HarnessId::Antigravity,
                HarnessId::Muse
            ]
        );
        assert!(registry.resolve(HarnessId::Mock).is_ok());
        assert!(registry.resolve(HarnessId::ClaudeCode).is_ok());
        // A codex-configured chat resolves the right harness (construction is
        // cheap; CLI discovery is deferred to models()/run()).
        let codex = registry.resolve(HarnessId::Codex).unwrap();
        assert_eq!(codex.id(), HarnessId::Codex);
        // Grok resolves through the shared ACP harness; its descriptor must
        // mirror the resolved harness (descriptor-stability rule).
        let grok = registry.resolve(HarnessId::Grok).unwrap();
        assert_eq!(grok.id(), HarnessId::Grok);
        assert_eq!(grok.display_name(), "Grok");
        assert_eq!(grok.steering_mode(), SteeringMode::StepBoundary);
        assert_eq!(
            grok.reasoning_levels(),
            &[
                ReasoningLevel::Low,
                ReasoningLevel::Medium,
                ReasoningLevel::High
            ]
        );
        // Cursor, Devin, Hermes and Pi mirror their specs the same way.
        let cursor = registry.resolve(HarnessId::Cursor).unwrap();
        assert_eq!(cursor.id(), HarnessId::Cursor);
        assert_eq!(cursor.display_name(), "Cursor");
        assert_eq!(cursor.steering_mode(), SteeringMode::StepBoundary);
        assert!(cursor.reasoning_levels().is_empty());
        let devin = registry.resolve(HarnessId::Devin).unwrap();
        assert_eq!(devin.id(), HarnessId::Devin);
        assert_eq!(devin.display_name(), "Devin");
        assert_eq!(devin.steering_mode(), SteeringMode::StepBoundary);
        assert!(devin.reasoning_levels().is_empty());
        let hermes = registry.resolve(HarnessId::Hermes).unwrap();
        assert_eq!(hermes.id(), HarnessId::Hermes);
        assert_eq!(hermes.display_name(), "Hermes");
        assert_eq!(hermes.steering_mode(), SteeringMode::TurnBoundary);
        assert!(hermes.reasoning_levels().is_empty());
        let opencode = registry.resolve(HarnessId::Opencode).unwrap();
        assert_eq!(opencode.id(), HarnessId::Opencode);
        assert_eq!(opencode.display_name(), "OpenCode");
        assert_eq!(opencode.steering_mode(), SteeringMode::StepBoundary);
        assert_eq!(
            opencode.reasoning_levels(),
            &[
                ReasoningLevel::Low,
                ReasoningLevel::Medium,
                ReasoningLevel::High,
                ReasoningLevel::XHigh,
                ReasoningLevel::Max,
            ]
        );
        let antigravity = registry.resolve(HarnessId::Antigravity).unwrap();
        assert_eq!(antigravity.id(), HarnessId::Antigravity);
        assert_eq!(antigravity.display_name(), "Antigravity");
        assert_eq!(antigravity.steering_mode(), SteeringMode::TurnBoundary);
        assert!(antigravity.reasoning_levels().is_empty());
        let pi = registry.resolve(HarnessId::Pi).unwrap();
        assert_eq!(pi.id(), HarnessId::Pi);
        assert_eq!(pi.display_name(), "Pi");
        assert_eq!(pi.steering_mode(), SteeringMode::StepBoundary);
        assert!(pi.reasoning_levels().is_empty());
    }

    /// Catalogs serialized by engines that predate the `installed`/`enabled`
    /// fields must keep deserializing — installed, and enabled per the
    /// detection fallback.
    #[test]
    fn descriptor_without_new_fields_parses_with_fallbacks() {
        let parse = |id: &str| -> HarnessDescriptor {
            serde_json::from_str(&format!(
                r#"{{
                    "id": "{id}",
                    "name": "x",
                    "supportsSteering": true,
                    "steeringMode": "step-boundary",
                    "reasoningLevels": []
                }}"#
            ))
            .unwrap()
        };
        let claude = parse("claude-code");
        assert!(claude.installed);
        assert!(!claude.can_install);
        assert_eq!(claude.enabled, None);
        // Unknown enablement follows detection: a found CLI is offered...
        assert!(descriptor_enabled(&claude));
        // ...and one this device never found is not.
        let missing = HarnessDescriptor {
            installed: false,
            can_install: false,
            ..parse("grok")
        };
        assert!(!descriptor_enabled(&missing));
    }

    /// A registry slot for the tests below: installed probe fixed, factory
    /// never expected to run.
    fn test_slot(registry: &HarnessRegistry, id: HarnessId, installed: bool) {
        registry.register_lazy(
            HarnessDescriptor {
                id,
                name: format!("{id:?}"),
                supports_steering: true,
                steering_mode: SteeringMode::StepBoundary,
                reasoning_levels: vec![],
                installed: true,
                can_install: false,
                enabled: None,
                access_modes: Vec::new(),
                supports_plan: false,
            },
            Box::new(move || installed),
            Box::new(|| Err(HarnessError::NotInstalled("test slot".into()))),
        );
    }

    /// `descriptors()` stamps the per-device enabled flag; `set_enabled`
    /// guards the gate (no enabling missing CLIs, no disabling the last one)
    /// and persists across a reload.
    #[test]
    fn a_chats_tool_set_narrows_the_global_one_and_keeps_its_settings() {
        let dir = tempfile::tempdir().unwrap();
        let registry = HarnessRegistry::new();
        registry.load_prefs(dir.path());
        let mut global = zeron_proto::tools::ToolSettings::default();
        global.set_tool("html_render", false);
        registry.set_tool_settings(global.clone());
        let chat = vec!["mcp__zeron__ask_user".to_owned(), "WebFetch".to_owned()];
        let state = registry.tool_settings_state(false, &chat);
        assert_eq!(
            state.settings, global,
            "Settings > Tools shows the global switches"
        );
        assert!(state.disabled.iter().any(|t| t == "html_render"));
        assert!(state.disabled.iter().any(|t| t == "ask_user"));
        assert!(
            state.disabled.iter().any(|t| t == "device_list"),
            "agent access off"
        );
        assert!(!state.disabled.iter().any(|t| t == "WebFetch"));
        assert!(
            !registry
                .tool_settings_state(true, &[])
                .disabled
                .iter()
                .any(|t| t == "device_list")
        );
    }

    #[test]
    fn enablement_stamps_guards_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let registry = HarnessRegistry::new();
        registry.load_prefs(dir.path());
        test_slot(&registry, HarnessId::ClaudeCode, true);
        test_slot(&registry, HarnessId::Codex, true);
        test_slot(&registry, HarnessId::Grok, true);
        test_slot(&registry, HarnessId::Hermes, false);

        // Enablement follows detection: the three found CLIs are on with no
        // prefs file at all, and the missing one is off.
        let flags: Vec<(HarnessId, Option<bool>)> = registry
            .descriptors()
            .into_iter()
            .map(|d| (d.id, d.enabled))
            .collect();
        assert_eq!(
            flags,
            vec![
                (HarnessId::ClaudeCode, Some(true)),
                (HarnessId::Codex, Some(true)),
                (HarnessId::Grok, Some(true)),
                (HarnessId::Hermes, Some(false)),
            ]
        );

        // The gate: a missing CLI can't be enabled; unknown ids refuse.
        assert!(registry.set_enabled(HarnessId::Hermes, true).is_err());
        assert!(registry.set_enabled(HarnessId::Pi, true).is_err());
        assert!(registry.set_enabled(HarnessId::Mock, true).is_err());
        // Installed CLIs toggle both ways; no-op flips are fine.
        registry.set_enabled(HarnessId::Grok, true).unwrap();
        registry.set_enabled(HarnessId::Grok, true).unwrap();
        registry.set_enabled(HarnessId::Codex, false).unwrap();
        registry.set_enabled(HarnessId::ClaudeCode, false).unwrap();
        // Grok is the last one standing — refusing keeps the composer usable.
        assert!(registry.set_enabled(HarnessId::Grok, false).is_err());
        assert_eq!(registry.enabled_set(), vec![HarnessId::Grok]);

        // A fresh registry over the same data dir reads the persisted opt-outs.
        let reloaded = HarnessRegistry::new();
        reloaded.load_prefs(dir.path());
        test_slot(&reloaded, HarnessId::ClaudeCode, true);
        test_slot(&reloaded, HarnessId::Codex, true);
        test_slot(&reloaded, HarnessId::Grok, true);
        assert_eq!(reloaded.enabled_set(), vec![HarnessId::Grok]);
    }

    /// The point of following detection: an agent installed after the user has
    /// already edited Settings turns itself on, while the ones they switched
    /// off stay off. An allow-list can't express that — it can't tell "the
    /// user said no" from "this wasn't installed yet".
    #[test]
    fn newly_found_harnesses_enable_themselves_without_reviving_opt_outs() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let registry = HarnessRegistry::new();
        registry.load_prefs(dir.path());
        test_slot(&registry, HarnessId::ClaudeCode, true);
        test_slot(&registry, HarnessId::Codex, true);
        // Not installed yet — the probe flips when the user installs the CLI.
        let found = Arc::new(AtomicBool::new(false));
        let probe = Arc::clone(&found);
        registry.register_lazy(
            HarnessDescriptor {
                id: HarnessId::Grok,
                name: "Grok".into(),
                supports_steering: true,
                steering_mode: SteeringMode::TurnBoundary,
                reasoning_levels: vec![],
                installed: true,
                can_install: false,
                enabled: None,
                access_modes: Vec::new(),
                supports_plan: false,
            },
            Box::new(move || probe.load(Ordering::SeqCst)),
            Box::new(|| Err(HarnessError::NotInstalled("test slot".into()))),
        );
        registry.set_enabled(HarnessId::Codex, false).unwrap();
        assert_eq!(registry.enabled_set(), vec![HarnessId::ClaudeCode]);

        // The CLI appears mid-session; no restart, no visit to Settings.
        found.store(true, Ordering::SeqCst);
        assert_eq!(
            registry.enabled_set(),
            vec![HarnessId::ClaudeCode, HarnessId::Grok]
        );
        // The opt-out survives the reload that picks the new agent up.
        let reloaded = HarnessRegistry::new();
        reloaded.load_prefs(dir.path());
        test_slot(&reloaded, HarnessId::ClaudeCode, true);
        test_slot(&reloaded, HarnessId::Codex, true);
        test_slot(&reloaded, HarnessId::Grok, true);
        assert_eq!(
            reloaded.enabled_set(),
            vec![HarnessId::ClaudeCode, HarnessId::Grok]
        );
    }

    #[test]
    fn antigravity_detection_ignores_legacy_opt_in_and_preserves_opt_out() {
        let dir = tempfile::tempdir().unwrap();
        let registry = HarnessRegistry::new();
        registry.load_prefs(dir.path());
        test_slot(&registry, HarnessId::ClaudeCode, true);
        test_slot(&registry, HarnessId::Antigravity, true);
        let prefs: HarnessPrefsFile =
            serde_json::from_str(r#"{"optedIn":["antigravity"],"disabled":["antigravity"]}"#)
                .unwrap();
        *registry.prefs() = prefs;
        assert_eq!(registry.enabled_set(), vec![HarnessId::ClaudeCode]);

        registry.set_enabled(HarnessId::Antigravity, true).unwrap();
        let both = vec![HarnessId::ClaudeCode, HarnessId::Antigravity];
        assert_eq!(registry.enabled_set(), both);

        let reloaded = HarnessRegistry::new();
        reloaded.load_prefs(dir.path());
        test_slot(&reloaded, HarnessId::ClaudeCode, true);
        test_slot(&reloaded, HarnessId::Antigravity, true);
        assert_eq!(reloaded.enabled_set(), both);

        reloaded.set_enabled(HarnessId::Antigravity, false).unwrap();
        assert_eq!(reloaded.enabled_set(), vec![HarnessId::ClaudeCode]);
    }

    #[cfg(unix)]
    #[test]
    fn antigravity_detection_subprocess() {
        let Ok(expected) = std::env::var("ZERON_TEST_AGY_INSTALLED") else {
            return;
        };
        let registry = HarnessRegistry::new();
        let data = tempfile::tempdir().unwrap();
        registry.load_prefs(data.path());
        registry.register(Arc::new(zeron_harness::AcpHarness::antigravity()));
        let expected = expected == "true";
        assert_eq!(registry.descriptors()[0].installed, expected);
        assert_eq!(
            registry.enabled_set().contains(&HarnessId::Antigravity),
            expected
        );
        assert_eq!(descriptor_enabled(&registry.descriptors()[0]), expected);
    }

    #[cfg(unix)]
    #[test]
    fn antigravity_server_path_controls_detection_and_enablement() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let cli = bin.join("agy");
        std::fs::write(&cli, "#!/bin/sh\nexit 91\n").unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
        let server = bin.join("agy_acp_server.par");
        for installed in [false, true] {
            if installed {
                std::fs::write(&server, "#!/bin/sh\nexit 91\n").unwrap();
                std::fs::set_permissions(&server, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "registry::tests::antigravity_detection_subprocess",
                    "--nocapture",
                ])
                .env("HOME", home.path())
                .env("PATH", &bin)
                .env_remove("ANTIGRAVITY_ACP_EXECUTABLE")
                .env("ZERON_NO_LOGIN_SHELL", "1")
                .env("ZERON_TEST_AGY_INSTALLED", installed.to_string())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
    }

    /// The mock resolves on every machine, so detection alone would enable it
    /// and let the last REAL agent be switched off — leaving a composer with
    /// nothing runnable behind a guard that thinks it is covered.
    #[test]
    fn detection_never_enables_the_mock() {
        let registry = default_registry();
        let enabled = registry.enabled_set();
        assert!(!enabled.contains(&HarnessId::Mock), "{enabled:?}");
        let mock = registry
            .descriptors()
            .into_iter()
            .find(|d| d.id == HarnessId::Mock)
            .expect("mock slot");
        assert!(mock.installed);
        assert_eq!(mock.enabled, Some(false));
    }

    /// A prefs file from the fixed-default era is an allow-list: everything
    /// registered and absent from it was a deliberate "no", so it converts to
    /// opt-outs rather than silently re-enabling on the next launch.
    #[test]
    fn legacy_allow_list_migrates_to_opt_outs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("harness-prefs.json"),
            r#"{ "enabled": ["claude-code"] }"#,
        )
        .unwrap();
        let registry = HarnessRegistry::new();
        test_slot(&registry, HarnessId::ClaudeCode, true);
        test_slot(&registry, HarnessId::Codex, true);
        test_slot(&registry, HarnessId::Grok, true);
        registry.load_prefs(dir.path());
        assert_eq!(registry.enabled_set(), vec![HarnessId::ClaudeCode]);

        // The rewritten file is the new shape, and the legacy key is gone.
        let text = std::fs::read_to_string(dir.path().join("harness-prefs.json")).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(json.get("enabled").is_none(), "{text}");
        assert!(text.contains("codex") && text.contains("grok"), "{text}");
        // An agent registered after the migration is new, not a past "no".
        test_slot(&registry, HarnessId::Cursor, true);
        assert_eq!(
            registry.enabled_set(),
            vec![HarnessId::ClaudeCode, HarnessId::Cursor]
        );
    }

    /// The fresh-machine shape (#128): no CLIs installed at all. Under
    /// detection-based enablement nothing is enabled to begin with — no
    /// dimmed default toggles to dismiss — and switching an uninstalled
    /// harness "off" is a clean no-op, not an error.
    #[test]
    fn routing_settings_persist_and_refuse_unknown_harnesses() {
        use crate::routing::{Role, RoleRoute, RoutingSettings};
        let dir = tempfile::tempdir().unwrap();
        let registry = HarnessRegistry::new();
        registry.load_prefs(dir.path());
        test_slot(&registry, HarnessId::Codex, true);
        let target = |harness| {
            RoleRoute::Target(zeron_proto::RouteTarget {
                harness,
                model: Some("gpt-6-luna".into()),
                reasoning: None,
            })
        };
        let mut settings = RoutingSettings::default();
        settings.roles.insert(Role::Worker, target(HarnessId::Grok));
        assert!(registry.set_routing_settings(settings.clone()).is_err());
        settings
            .roles
            .insert(Role::Worker, target(HarnessId::Codex));
        registry.set_routing_settings(settings.clone()).unwrap();

        let written = std::fs::read_to_string(dir.path().join("harness-prefs.json")).unwrap();
        let written: serde_json::Value = serde_json::from_str(&written).unwrap();
        assert_eq!(written["routing"]["roles"]["worker"]["harness"], "codex");
        let reloaded = HarnessRegistry::new();
        reloaded.load_prefs(dir.path());
        assert_eq!(reloaded.routing_settings(), settings);
    }

    #[test]
    fn machine_without_clis_enables_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let registry = HarnessRegistry::new();
        registry.load_prefs(dir.path());
        test_slot(&registry, HarnessId::ClaudeCode, false);
        test_slot(&registry, HarnessId::Codex, false);

        assert_eq!(registry.enabled_set(), Vec::<HarnessId>::new());
        registry.set_enabled(HarnessId::Codex, false).unwrap();
        registry.set_enabled(HarnessId::ClaudeCode, false).unwrap();
        assert_eq!(registry.enabled_set(), Vec::<HarnessId>::new());

        let reloaded = HarnessRegistry::new();
        reloaded.load_prefs(dir.path());
        assert_eq!(reloaded.enabled_set(), Vec::<HarnessId>::new());
    }

    /// The Codex lazy descriptor must be indistinguishable from `describe()`
    /// after the first resolve — otherwise the catalog entry silently changes
    /// the moment the harness is used (name/ladder flip in the picker rail).
    /// (KNOWN GAP, predates this slot: the claude-code descriptor advertises
    /// `[Ultrathink]` while the resolved adapter reports `[Low..Max]` — left
    /// as-is here; flagged for its own pass.)
    #[test]
    fn codex_lazy_descriptor_matches_resolved_harness() {
        let registry = default_registry();
        let before = registry
            .descriptors()
            .into_iter()
            .find(|d| d.id == HarnessId::Codex)
            .unwrap();
        registry.resolve(HarnessId::Codex).unwrap();
        let after = registry
            .descriptors()
            .into_iter()
            .find(|d| d.id == HarnessId::Codex)
            .unwrap();
        assert_eq!(before.name, after.name);
        assert_eq!(before.supports_steering, after.supports_steering);
        assert_eq!(before.steering_mode, after.steering_mode);
        assert_eq!(before.reasoning_levels, after.reasoning_levels);
    }

    #[test]
    fn native_subagents_switch_off_per_harness_and_persist() {
        let dir = tempfile::tempdir().unwrap();
        let registry = HarnessRegistry::new();
        registry.load_prefs(dir.path());
        test_slot(&registry, HarnessId::ClaudeCode, true);
        test_slot(&registry, HarnessId::Codex, true);
        test_slot(&registry, HarnessId::Opencode, true);
        assert!(registry.native_subagents(HarnessId::ClaudeCode));
        assert_eq!(
            registry.native_subagent_settings().switchable,
            vec![HarnessId::ClaudeCode]
        );
        // A Codex thread keeps the mode it ran with: no switch to undo.
        assert!(
            registry
                .set_native_subagents(HarnessId::Codex, false)
                .is_err()
        );

        registry
            .set_native_subagents(HarnessId::ClaudeCode, false)
            .unwrap();
        registry
            .set_native_subagents(HarnessId::ClaudeCode, false)
            .unwrap();
        assert!(
            registry
                .set_native_subagents(HarnessId::Opencode, false)
                .is_err()
        );
        assert!(!registry.native_subagents(HarnessId::ClaudeCode));
        assert!(registry.native_subagents(HarnessId::Codex));

        // One setting: the switch is the routing native policy.
        assert_eq!(
            registry.routing_settings().native,
            crate::routing::NativePolicy::Off
        );
        assert!(registry.native_subagents(HarnessId::Opencode));

        let reloaded = HarnessRegistry::new();
        reloaded.load_prefs(dir.path());
        test_slot(&reloaded, HarnessId::ClaudeCode, true);
        assert_eq!(
            reloaded.native_subagent_settings().off,
            vec![HarnessId::ClaudeCode]
        );
        reloaded
            .set_native_subagents(HarnessId::ClaudeCode, true)
            .unwrap();
        assert!(reloaded.native_subagents(HarnessId::ClaudeCode));
        assert_eq!(
            reloaded.routing_settings().native,
            crate::routing::NativePolicy::default()
        );
    }
}

#[cfg(test)]
mod title_tests {
    use super::*;

    #[test]
    fn title_preferences_persist_and_validate_harness_model_pairs() {
        let dir = tempfile::tempdir().unwrap();
        let registry = HarnessRegistry::new();
        registry.load_prefs(dir.path());
        registry.register(Arc::new(
            zeron_harness::ClaudeHarness::new().with_executable(std::env::current_exe().unwrap()),
        ));
        let settings = TitleSettings {
            harness: Some(HarnessId::ClaudeCode),
            model: Some("haiku-test".into()),
        };
        registry.set_title_settings(settings.clone()).unwrap();
        let reloaded = HarnessRegistry::new();
        reloaded.load_prefs(dir.path());
        assert_eq!(reloaded.title_settings(), settings);
        assert!(
            registry
                .set_title_settings(TitleSettings {
                    harness: None,
                    model: Some("orphan".into())
                })
                .is_err()
        );
        assert!(
            registry
                .set_title_settings(TitleSettings {
                    harness: Some(HarnessId::Cursor),
                    model: None
                })
                .is_err()
        );
        assert_eq!(registry.title_settings(), settings);
        registry
            .set_title_settings(TitleSettings::default())
            .unwrap();
        reloaded.load_prefs(dir.path());
        assert_eq!(reloaded.title_settings(), TitleSettings::default());
    }

    #[test]
    fn old_harness_preferences_default_to_automatic_titles() {
        let prefs: HarnessPrefsFile = serde_json::from_str(r#"{"disabled":["codex"]}"#).unwrap();
        assert_eq!(prefs.titles, TitleSettings::default());
        assert_eq!(prefs.disabled, vec![HarnessId::Codex]);
    }
}

#[cfg(test)]
mod gate_tests {
    use super::*;

    struct DiscoveryHarness {
        started: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }
    #[async_trait::async_trait]
    impl Harness for DiscoveryHarness {
        fn id(&self) -> HarnessId {
            HarnessId::Codex
        }
        fn display_name(&self) -> &str {
            "Discovery fixture"
        }
        fn supports_steering(&self) -> bool {
            false
        }
        fn steering_mode(&self) -> SteeringMode {
            SteeringMode::TurnBoundary
        }
        fn reasoning_levels(&self) -> &[ReasoningLevel] {
            &[]
        }
        async fn models(&self) -> Result<Vec<zeron_proto::Model>, HarnessError> {
            self.started.notify_one();
            self.release.notified().await;
            Ok(vec![])
        }
        async fn commands(&self) -> Result<Vec<zeron_proto::SlashCommand>, HarnessError> {
            self.started.notify_one();
            self.release.notified().await;
            Ok(vec![])
        }
        async fn run(
            &self,
            _: zeron_proto::RunRequest,
            _: zeron_harness::RunControls,
        ) -> Result<
            futures::stream::BoxStream<'static, Result<AgentEvent, HarnessError>>,
            HarnessError,
        > {
            unreachable!("discovery must not start a conversation")
        }
    }

    #[tokio::test]
    async fn discovery_waits_for_updates_and_keeps_lease_after_caller_cancellation() {
        use std::time::Duration;
        for commands in [false, true] {
            let registry = Arc::new(HarnessRegistry::new());
            let harness = Arc::new(DiscoveryHarness {
                started: tokio::sync::Notify::new(),
                release: tokio::sync::Notify::new(),
            });
            registry.register(harness.clone());
            registry.begin_update(HarnessId::Codex);
            let installing = registry.update_lease(HarnessId::Codex).await;
            let caller = tokio::spawn({
                let registry = registry.clone();
                async move {
                    if commands {
                        registry
                            .discover_commands(
                                registry.resolve(HarnessId::Codex)?,
                                Path::new("/tmp"),
                            )
                            .await
                            .map(|_| ())
                    } else {
                        registry.discover_models(HarnessId::Codex).await.map(|_| ())
                    }
                }
            });
            assert!(
                tokio::time::timeout(Duration::from_millis(30), harness.started.notified())
                    .await
                    .is_err()
            );
            drop(installing);
            registry.end_update(HarnessId::Codex);
            tokio::time::timeout(Duration::from_secs(1), harness.started.notified())
                .await
                .unwrap();
            caller.abort();
            assert!(caller.await.unwrap_err().is_cancelled());
            registry.begin_update(HarnessId::Codex);
            let mut writer = tokio::spawn({
                let registry = registry.clone();
                async move { registry.update_lease(HarnessId::Codex).await }
            });
            assert!(
                tokio::time::timeout(Duration::from_millis(30), &mut writer)
                    .await
                    .is_err()
            );
            harness.release.notify_one();
            drop(
                tokio::time::timeout(Duration::from_secs(1), writer)
                    .await
                    .unwrap()
                    .unwrap(),
            );
            registry.end_update(HarnessId::Codex);
        }
    }

    #[tokio::test]
    async fn queued_update_writer_precedes_later_dispatches() {
        let registry = Arc::new(HarnessRegistry::new());
        let running = registry.execution_lease(HarnessId::Codex).await;
        registry.begin_update(HarnessId::Codex);
        assert!(registry.update_pending(HarnessId::Codex));

        let writer_registry = registry.clone();
        let (writer_acquired_tx, writer_acquired_rx) = tokio::sync::oneshot::channel();
        let (release_writer_tx, release_writer_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _writer = writer_registry.update_lease(HarnessId::Codex).await;
            let _ = writer_acquired_tx.send(());
            let _ = release_writer_rx.await;
        });
        tokio::task::yield_now().await;

        let reader_registry = registry.clone();
        let (reader_acquired_tx, reader_acquired_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _reader = reader_registry.execution_lease(HarnessId::Codex).await;
            let _ = reader_acquired_tx.send(());
        });

        drop(running);
        writer_acquired_rx.await.unwrap();
        let mut reader_acquired_rx = Box::pin(reader_acquired_rx);
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(20),
                &mut reader_acquired_rx
            )
            .await
            .is_err(),
            "later reader jumped ahead of the queued update writer"
        );
        let _ = release_writer_tx.send(());
        registry.end_update(HarnessId::Codex);
        reader_acquired_rx.await.unwrap();
        assert!(!registry.update_pending(HarnessId::Codex));
    }

    #[test]
    fn update_marker_rejects_new_boundary_actions() {
        let registry = HarnessRegistry::new();
        assert_eq!(
            registry.while_update_clear(HarnessId::Codex, || 42),
            Some(42)
        );
        registry.begin_update(HarnessId::Codex);
        let mut called = false;
        assert_eq!(
            registry.while_update_clear(HarnessId::Codex, || called = true),
            None
        );
        assert!(!called);
        registry.end_update(HarnessId::Codex);
    }

    #[test]
    fn custom_prediction_apis_need_https_except_on_loopback() {
        let api = |base_url: &str, model: &str| CustomPredictionApi {
            base_url: base_url.into(),
            model: model.into(),
        };
        let ok = api(" https://openrouter.ai/api/v1/ ", " gpt-mini ")
            .normalized()
            .unwrap();
        assert_eq!(ok.base_url, "https://openrouter.ai/api/v1");
        assert_eq!(ok.model, "gpt-mini");
        assert_eq!(
            ok.endpoint(),
            "https://openrouter.ai/api/v1/chat/completions"
        );
        assert!(
            api("http://localhost:11434/v1", "llama")
                .normalized()
                .is_ok()
        );
        assert!(api("http://127.0.0.1:1234/v1", "qwen").normalized().is_ok());
        assert!(api("http://api.example.com/v1", "m").normalized().is_err());
        assert!(api("ftp://localhost/v1", "m").normalized().is_err());
        assert!(api("not a url", "m").normalized().is_err());
        assert!(
            api("https://api.example.com/v1", "  ")
                .normalized()
                .is_err()
        );
    }

    #[test]
    fn cloud_platform_registers_only_codex_and_claude() {
        let registry = registry_for_platform(zeron_proto::CLOUD_PLATFORM);
        let ids: Vec<HarnessId> = registry.descriptors().iter().map(|d| d.id).collect();
        assert_eq!(
            ids,
            vec![HarnessId::Mock, HarnessId::ClaudeCode, HarnessId::Codex]
        );
        for hidden in [
            HarnessId::Cursor,
            HarnessId::Devin,
            HarnessId::Grok,
            HarnessId::Hermes,
            HarnessId::Pi,
            HarnessId::Opencode,
            HarnessId::Antigravity,
            HarnessId::Muse,
        ] {
            assert!(!registry.is_registered(hidden), "{hidden:?}");
            // Never resolved, so never probed or spawned.
            assert!(registry.resolve(hidden).is_err(), "{hidden:?}");
        }
        let enabled = registry.enabled_set();
        assert!(
            enabled.iter().all(|id| CLOUD_HARNESS_IDS.contains(id)),
            "{enabled:?}"
        );
        // Laptops keep the full catalog.
        let laptop = registry_for_platform("macos");
        assert!(laptop.is_registered(HarnessId::Cursor));
        assert!(laptop.is_registered(HarnessId::Opencode));
    }

    #[test]
    fn cloud_harness_ids_mirror_the_proto_contract() {
        // `CLOUD_HARNESSES` uses the wire names, so serialize the typed ids.
        let names: Vec<String> = CLOUD_HARNESS_IDS
            .iter()
            .map(|id| {
                serde_json::to_value(id)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut expected: Vec<&str> = zeron_proto::CLOUD_HARNESSES.to_vec();
        let mut names = names;
        names.sort_unstable();
        expected.sort_unstable();
        assert_eq!(names, expected);
    }

    #[tokio::test]
    async fn run_environment_fills_env_only_when_installed() {
        struct Fill;
        #[async_trait::async_trait]
        impl RunEnvironment for Fill {
            async fn prepare(
                &self,
                harness: HarnessId,
                request: &mut zeron_proto::RunRequest,
            ) -> Result<(), String> {
                request.env.insert("HARNESS".into(), format!("{harness:?}"));
                Ok(())
            }
        }
        let mut request: zeron_proto::RunRequest = serde_json::from_str(
            r#"{"prompt":"p","model":null,"reasoning":null,"cwd":".","sandbox":"workspace-write","resume":null}"#,
        )
        .unwrap();
        let registry = HarnessRegistry::new();
        registry
            .prepare_run(HarnessId::Codex, &mut request)
            .await
            .unwrap();
        assert!(request.env.is_empty());
        registry.set_run_environment(Arc::new(Fill));
        registry
            .prepare_run(HarnessId::Codex, &mut request)
            .await
            .unwrap();
        assert_eq!(
            request.env.get("HARNESS").map(String::as_str),
            Some("Codex")
        );
    }

    /// Every agent spawn carries the commit-credit hooks while that's on.
    #[tokio::test]
    async fn runs_carry_the_commit_credit_hooks() {
        let data = tempfile::tempdir().unwrap();
        let mut request: zeron_proto::RunRequest = serde_json::from_str(
            r#"{"prompt":"p","model":null,"reasoning":null,"cwd":".","sandbox":"workspace-write","resume":null}"#,
        )
        .unwrap();
        let registry = HarnessRegistry::new();
        let credit = Arc::new(crate::commit_credit::CommitCredit::open(data.path(), false));
        credit.set(true).unwrap();
        registry.set_commit_credit(credit.clone());
        registry
            .prepare_run(HarnessId::ClaudeCode, &mut request)
            .await
            .unwrap();
        let hooks = data.path().join("git-hooks");
        assert_eq!(
            request.env.get("GIT_CONFIG_KEY_0").map(String::as_str),
            Some("core.hooksPath")
        );
        assert_eq!(
            request.env.get("GIT_CONFIG_VALUE_0").map(String::as_str),
            Some(hooks.to_string_lossy().as_ref())
        );
        assert!(hooks.join("commit-msg").exists());

        // Off on this device: nothing added.
        credit.set(false).unwrap();
        request.env.clear();
        registry
            .prepare_run(HarnessId::ClaudeCode, &mut request)
            .await
            .unwrap();
        assert!(request.env.is_empty());
    }
}
