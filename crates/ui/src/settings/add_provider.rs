use std::collections::{BTreeMap, HashMap};

use gpui::{
    AnyElement, Context, Entity, EventEmitter, SharedString, Subscription, Task, div, prelude::*,
    px,
};
use zeron_engine::registry::HarnessDescriptor;
use zeron_proto::{AcpSource, HarnessId, HomeDir, InstanceConfig, ProviderInstance};

mod acp;

use acp::{AcpFlow, RegistryState, SelectedAgent, SignInState};

use crate::composer::{ComposerInput, ComposerInputEvent};
use crate::icons::{self, icon};
use crate::popover;
use crate::provider_instances::{self, ProviderInstanceWrite};
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::{Theme, ink};

pub const DRIVERS: [HarnessId; 9] = [
    HarnessId::ClaudeCode,
    HarnessId::Codex,
    HarnessId::Cursor,
    HarnessId::Devin,
    HarnessId::Grok,
    HarnessId::Hermes,
    HarnessId::Opencode,
    HarnessId::Pi,
    HarnessId::Antigravity,
];

pub fn driver_name(driver: HarnessId) -> &'static str {
    match driver {
        HarnessId::ClaudeCode => "Claude",
        HarnessId::Codex => "Codex",
        HarnessId::Cursor => "Cursor",
        HarnessId::Devin => "Devin",
        HarnessId::Grok => "Grok",
        HarnessId::Hermes => "Hermes",
        HarnessId::Opencode => "OpenCode",
        HarnessId::Pi => "Pi",
        HarnessId::Antigravity => "Antigravity",
        HarnessId::Acp => "ACP",
        HarnessId::Mock => "Mock",
    }
}

fn binary_placeholder(driver: HarnessId) -> &'static str {
    match driver {
        HarnessId::Antigravity => "antigravity-acp",
        _ => crate::settings::harnesses::cli_name(driver),
    }
}

fn home_field(driver: HarnessId) -> Option<(&'static str, &'static str)> {
    match driver {
        HarnessId::ClaudeCode => Some(("CLAUDE_CONFIG_DIR path", "~/.claude")),
        HarnessId::Codex => Some(("CODEX_HOME path", "~/.codex")),
        HarnessId::Pi => Some(("PI_CODING_AGENT_DIR path", "~/.pi/agent")),
        _ => None,
    }
}

fn sign_in_action(driver: HarnessId) -> Option<(&'static str, &'static str)> {
    match driver {
        HarnessId::Codex => Some(("Continue with ChatGPT", icons::OPENAI_MARK)),
        HarnessId::ClaudeCode => Some(("Sign in with Claude", icons::CLAUDE_MARK)),
        HarnessId::Pi => Some(("Continue with ChatGPT", icons::OPENAI_MARK)),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Step {
    Provider,
    Identity,
    Config,
    SignIn,
}

impl Step {
    pub fn from_knob(knob: &str) -> Option<(Step, bool)> {
        match knob {
            "provider" => Some((Step::Provider, false)),
            "identity" => Some((Step::Identity, false)),
            "identity-taken" => Some((Step::Identity, true)),
            "config" => Some((Step::Config, false)),
            _ => None,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Step::Provider => "Provider",
            Step::Identity => "Identity",
            Step::Config => "Config",
            Step::SignIn => "Sign in",
        }
    }
}

fn resolve_step(target: Step, editing: bool, id_error: bool) -> Step {
    let target = target.max(if editing {
        Step::Identity
    } else {
        Step::Provider
    });
    if target > Step::Identity && id_error {
        Step::Identity
    } else {
        target
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Identity {
    label: String,
    accent: Option<String>,
    id_override: Option<String>,
}

impl Identity {
    fn new(driver: HarnessId) -> Self {
        Self {
            label: driver_name(driver).to_string(),
            accent: None,
            id_override: None,
        }
    }

    fn untouched(&self, driver: HarnessId) -> bool {
        self.id_override.is_none() && self.label.trim() == driver_name(driver)
    }
}

struct EnvRow {
    key: Entity<ComposerInput>,
    value: Entity<ComposerInput>,
}

pub enum AddProviderEvent {
    Close,
    Saved {
        instances: Vec<ProviderInstance>,
        instance: ProviderInstance,
        sign_in: bool,
    },
    Created {
        instances: Vec<ProviderInstance>,
    },
}

enum AfterSave {
    Close { sign_in: bool },
    SignIn,
}

pub struct AddProviderDialog {
    state: Entity<AppState>,
    target_device: Option<String>,
    device_name: SharedString,
    harnesses: Vec<HarnessDescriptor>,
    instances: Vec<ProviderInstance>,
    editing: Option<ProviderInstance>,
    step: Step,
    driver: HarnessId,
    tile: HarnessId,
    acp: Option<AcpFlow>,
    selected: Option<SelectedAgent>,
    registry: RegistryState,
    sign_in: Option<SignInState>,
    pending_sign_in: Option<String>,
    acp_error: Option<SharedString>,
    identities: HashMap<HarnessId, Identity>,
    label_input: Entity<ComposerInput>,
    id_input: Entity<ComposerInput>,
    binary_input: Entity<ComposerInput>,
    home_input: Entity<ComposerInput>,
    args_input: Entity<ComposerInput>,
    env_rows: Vec<EnvRow>,
    search_input: Entity<ComposerInput>,
    manual_input: Entity<ComposerInput>,
    command_input: Entity<ComposerInput>,
    acp_args_input: Entity<ComposerInput>,
    saving: bool,
    error: Option<SharedString>,
    save_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<AddProviderEvent> for AddProviderDialog {}

impl AddProviderDialog {
    pub fn new(
        state: Entity<AppState>,
        target_device: Option<String>,
        device_name: SharedString,
        harnesses: Vec<HarnessDescriptor>,
        instances: Vec<ProviderInstance>,
        editing: Option<ProviderInstance>,
        cx: &mut Context<Self>,
    ) -> Self {
        let label_input = cx.new(|cx| ComposerInput::new("e.g. Work", cx).with_single_line());
        let id_input = cx.new(|cx| mono_input("", cx));
        let binary_input = cx.new(|cx| mono_input("", cx));
        let home_input = cx.new(|cx| mono_input("", cx));
        let args_input = cx.new(|cx| mono_input("--flag value", cx));
        let search_input = cx.new(|cx| ComposerInput::new("Search agents…", cx).with_single_line());
        let manual_input = cx.new(|cx| mono_input("devin", cx));
        let command_input = cx.new(|cx| mono_input("~/bin/my-agent", cx));
        let acp_args_input = cx.new(|cx| mono_input("--acp", cx));
        let subscriptions = vec![
            cx.subscribe(&search_input, |this: &mut Self, _, event, cx| {
                if matches!(event, ComposerInputEvent::Edited) {
                    this.search_edited(cx);
                }
            }),
            cx.subscribe(&command_input, |this: &mut Self, _, event, cx| {
                if matches!(event, ComposerInputEvent::Edited) && this.acp_error.is_some() {
                    this.acp_error = None;
                    cx.notify();
                }
            }),
            cx.subscribe(&manual_input, |this: &mut Self, _, event, cx| {
                if matches!(event, ComposerInputEvent::Edited) && this.acp_error.is_some() {
                    this.acp_error = None;
                    cx.notify();
                }
            }),
            cx.subscribe(&label_input, |this: &mut Self, _, event, cx| {
                if matches!(event, ComposerInputEvent::Edited) {
                    this.label_edited(cx);
                }
            }),
            cx.subscribe(&id_input, |this: &mut Self, _, event, cx| {
                if matches!(event, ComposerInputEvent::Edited) {
                    this.id_edited(cx);
                }
            }),
        ];
        let driver = editing.as_ref().map_or(HarnessId::Codex, |i| i.driver);
        let searching = editing.is_none();
        let mut dialog = Self {
            state,
            target_device,
            device_name,
            harnesses,
            instances,
            editing: None,
            step: Step::Provider,
            driver,
            tile: if driver == HarnessId::Acp {
                HarnessId::Codex
            } else {
                driver
            },
            acp: None,
            selected: None,
            registry: RegistryState::default(),
            sign_in: None,
            pending_sign_in: None,
            acp_error: None,
            identities: HashMap::new(),
            label_input,
            id_input,
            binary_input,
            home_input,
            args_input,
            env_rows: Vec::new(),
            search_input,
            manual_input,
            command_input,
            acp_args_input,
            saving: false,
            error: None,
            save_task: None,
            _subscriptions: subscriptions,
        };
        match editing {
            Some(instance) => dialog.load_instance(instance, cx),
            None => dialog.load_driver(cx),
        }
        if searching {
            dialog.search(false, cx);
        }
        dialog
    }

    fn load_instance(&mut self, instance: ProviderInstance, cx: &mut Context<Self>) {
        self.driver = instance.driver;
        self.step = Step::Identity;
        self.identities.insert(
            instance.driver,
            Identity {
                label: instance.label.clone(),
                accent: instance.accent.clone(),
                id_override: Some(instance.id.clone()),
            },
        );
        self.load_driver(cx);
        if let InstanceConfig::Native {
            executable,
            home_dir,
            launch_args,
            env,
        } = &instance.config
        {
            let home = match home_dir {
                Some(HomeDir::Custom { path }) => path.clone(),
                _ => String::new(),
            };
            self.binary_input.update(cx, |input, cx| {
                input.set_text(executable.clone().unwrap_or_default(), cx)
            });
            self.home_input
                .update(cx, |input, cx| input.set_text(home, cx));
            self.args_input
                .update(cx, |input, cx| input.set_text(launch_args.join(" "), cx));
            for (key, value) in env {
                self.add_env_row(key, value, cx);
            }
            if matches!(home_dir, Some(HomeDir::Managed)) {
                self.home_input.update(cx, |input, cx| {
                    input.set_placeholder("Managed by Zeron", cx)
                });
            }
        } else if let InstanceConfig::Acp { source, env, .. } = &instance.config {
            self.acp = Some(match source {
                AcpSource::Registry { .. } => AcpFlow::Registry,
                AcpSource::Local { .. } => AcpFlow::Local,
            });
            if let AcpSource::Local { command, args } = source {
                self.command_input
                    .update(cx, |input, cx| input.set_text(command.clone(), cx));
                self.acp_args_input
                    .update(cx, |input, cx| input.set_text(args.join(" "), cx));
            }
            for (key, value) in env {
                self.add_env_row(key, value, cx);
            }
        }
        self.editing = Some(instance);
    }

    pub fn set_instances(&mut self, instances: Vec<ProviderInstance>, cx: &mut Context<Self>) {
        self.instances = instances;
        if self.identity().id_override.is_none() {
            self.sync_id_input(cx);
        }
        self.try_pending_sign_in(cx);
        cx.notify();
    }

    pub fn set_harnesses(&mut self, harnesses: Vec<HarnessDescriptor>, cx: &mut Context<Self>) {
        self.harnesses = harnesses;
        cx.notify();
    }

    pub fn open_at(&mut self, step: Step, id_taken: bool, cx: &mut Context<Self>) {
        if id_taken && self.editing.is_none() {
            let base = serde_json::to_value(self.driver)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_default();
            let name = driver_name(self.driver);
            self.label_input
                .update(cx, |input, cx| input.set_text(name, cx));
            self.id_input
                .update(cx, |input, cx| input.set_text(base.clone(), cx));
            self.identity_mut().id_override = Some(base);
            self.step = step;
        } else {
            self.step = resolve_step(step, self.editing.is_some(), self.id_error().is_some());
        }
        cx.notify();
    }

    pub fn select_driver(&mut self, driver: HarnessId, cx: &mut Context<Self>) {
        if self.editing.is_some() || self.registry.preparing.is_some() {
            return;
        }
        self.tile = driver;
        if self.acp.take().is_none() && self.driver == driver {
            return;
        }
        self.selected = None;
        self.acp_error = None;
        self.driver = driver;
        self.load_driver(cx);
        self.error = None;
        cx.notify();
    }

    fn load_driver(&mut self, cx: &mut Context<Self>) {
        let identity = self.identity();
        let id = identity
            .id_override
            .clone()
            .unwrap_or_else(|| self.derived_id(&identity.label));
        self.label_input
            .update(cx, |input, cx| input.set_text(identity.label, cx));
        self.id_input.update(cx, |input, cx| input.set_text(id, cx));
        let binary = binary_placeholder(self.driver);
        self.binary_input.update(cx, |input, cx| {
            input.set_text("", cx);
            input.set_placeholder(binary, cx);
        });
        let home = home_field(self.driver).map_or("", |(_, placeholder)| placeholder);
        self.home_input.update(cx, |input, cx| {
            input.set_text("", cx);
            input.set_placeholder(home, cx);
        });
        self.args_input
            .update(cx, |input, cx| input.set_text("", cx));
        self.env_rows.clear();
    }

    fn identity(&self) -> Identity {
        self.identities
            .get(&self.driver)
            .cloned()
            .unwrap_or_else(|| Identity::new(self.driver))
    }

    fn identity_mut(&mut self) -> &mut Identity {
        let driver = self.driver;
        self.identities
            .entry(driver)
            .or_insert_with(|| Identity::new(driver))
    }

    fn taken(&self) -> impl Fn(&str) -> bool + use<> {
        let own = self.editing.as_ref().map(|i| i.id.clone());
        let ids: Vec<String> = self
            .instances
            .iter()
            .map(|i| i.id.clone())
            .filter(|id| Some(id) != own.as_ref())
            .collect();
        move |id: &str| ids.iter().any(|taken| taken.eq_ignore_ascii_case(id))
    }

    fn derived_id(&self, label: &str) -> String {
        if self.driver == HarnessId::Acp && label.trim().is_empty() {
            return zeron_proto::derive_instance_id(HarnessId::Acp, "custom", self.taken())
                .unwrap_or_default();
        }
        provider_instances::default_instance_id(
            self.driver,
            driver_name(self.driver),
            label,
            self.taken(),
        )
    }

    fn instance_id(&self) -> String {
        if let Some(instance) = &self.editing {
            return instance.id.clone();
        }
        let identity = self.identity();
        identity
            .id_override
            .unwrap_or_else(|| self.derived_id(&identity.label))
    }

    fn id_error(&self) -> Option<String> {
        if self.editing.is_some() {
            return None;
        }
        provider_instances::instance_id_error(&self.instance_id(), self.taken())
    }

    fn sync_id_input(&mut self, cx: &mut Context<Self>) {
        let derived = self.derived_id(&self.identity().label);
        if self.id_input.read(cx).text() != derived {
            self.id_input
                .update(cx, |input, cx| input.set_text(derived, cx));
        }
    }

    fn label_edited(&mut self, cx: &mut Context<Self>) {
        let label = self.label_input.read(cx).text().to_string();
        self.identity_mut().label = label;
        if self.editing.is_none() && self.identity().id_override.is_none() {
            self.sync_id_input(cx);
        }
        cx.notify();
    }

    fn id_edited(&mut self, cx: &mut Context<Self>) {
        if self.editing.is_some() {
            return;
        }
        let text = self.id_input.read(cx).text().to_string();
        let derived = self.derived_id(&self.identity().label);
        self.identity_mut().id_override = (text != derived).then_some(text);
        cx.notify();
    }

    fn set_accent(&mut self, accent: Option<String>, cx: &mut Context<Self>) {
        self.identity_mut().accent = accent;
        cx.notify();
    }

    fn add_env_row(&mut self, key: &str, value: &str, cx: &mut Context<Self>) {
        let (key, value) = (key.to_string(), value.to_string());
        let key = cx.new(|cx| {
            let mut input = mono_input("KEY", cx);
            input.set_text(key, cx);
            input
        });
        let value = cx.new(|cx| {
            let mut input = mono_input("value", cx);
            input.set_text(value, cx);
            input
        });
        self.env_rows.push(EnvRow { key, value });
        cx.notify();
    }

    pub fn go(&mut self, step: Step, cx: &mut Context<Self>) {
        self.step = resolve_step(step, self.editing.is_some(), self.id_error().is_some());
        cx.notify();
    }

    fn back(&mut self, cx: &mut Context<Self>) {
        match self.step {
            Step::Config => self.go(Step::Identity, cx),
            Step::Identity if self.editing.is_none() => self.go(Step::Provider, cx),
            _ => self.close(cx),
        }
    }

    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        self.cancel_login(cx);
        cx.emit(AddProviderEvent::Close);
    }

    fn steps(&self) -> &'static [Step] {
        match self.acp.filter(|_| self.editing.is_none()) {
            None => &[Step::Provider, Step::Identity, Step::Config],
            Some(AcpFlow::Local) => &[Step::Provider, Step::Identity],
            Some(_) => &[Step::Provider, Step::Identity, Step::SignIn],
        }
    }

    fn step_reachable(&self, step: Step, cx: &Context<Self>) -> bool {
        if self.busy() {
            return false;
        }
        if self.editing.is_some() {
            return step != Step::Provider;
        }
        if self.acp.is_none() {
            return true;
        }
        if self.sign_in.is_some() {
            return false;
        }
        match step {
            Step::Provider => true,
            Step::Identity => self.acp_selection_ok(cx),
            Step::Config | Step::SignIn => false,
        }
    }

    fn busy(&self) -> bool {
        self.saving || self.registry.preparing.is_some()
    }

    fn config(&self, cx: &Context<Self>) -> Option<InstanceConfig> {
        let text = |input: &Entity<ComposerInput>| input.read(cx).text().trim().to_string();
        let env: BTreeMap<String, String> = self
            .env_rows
            .iter()
            .map(|row| (text(&row.key), row.value.read(cx).text().to_string()))
            .filter(|(key, _)| !key.is_empty())
            .collect();
        if self.driver == HarnessId::Acp {
            return self.acp_config(env, cx);
        }
        if self.driver == HarnessId::Cursor {
            return Some(InstanceConfig::Native {
                executable: None,
                home_dir: None,
                launch_args: Vec::new(),
                env,
            });
        }
        let executable = Some(text(&self.binary_input)).filter(|path| !path.is_empty());
        let home = text(&self.home_input);
        let managed = self.editing.as_ref().is_some_and(|instance| {
            matches!(
                instance.config,
                InstanceConfig::Native {
                    home_dir: Some(HomeDir::Managed),
                    ..
                }
            )
        });
        let home_dir = match home_field(self.driver) {
            None => None,
            Some(_) if !home.is_empty() => Some(HomeDir::Custom { path: home }),
            Some(_) if managed => Some(HomeDir::Managed),
            Some(_) => None,
        };
        Some(InstanceConfig::Native {
            executable,
            home_dir,
            launch_args: text(&self.args_input)
                .split_whitespace()
                .map(str::to_owned)
                .collect(),
            env,
        })
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        if self.id_error().is_some() {
            self.go(Step::Identity, cx);
            return;
        }
        let Some(config) = self.config(cx) else {
            self.error = Some("Choose an ACP agent first.".into());
            cx.notify();
            return;
        };
        let identity = self.identity();
        let label = self.label_or_default(&identity);
        let instance = ProviderInstance {
            id: self.instance_id(),
            driver: self.driver,
            label,
            accent: identity.accent,
            config,
        };
        let after = if self.editing.is_none() && self.acp == Some(AcpFlow::Registry) {
            AfterSave::SignIn
        } else {
            AfterSave::Close { sign_in: false }
        };
        self.upsert(instance, after, cx);
    }

    fn label_or_default(&self, identity: &Identity) -> String {
        let fallback = match (&self.selected, self.acp) {
            (Some(selected), _) => selected.agent.name.clone(),
            (None, Some(AcpFlow::Local)) => "Local ACP".to_string(),
            _ => driver_name(self.driver).to_string(),
        };
        Some(identity.label.trim().to_string())
            .filter(|label| !label.is_empty())
            .unwrap_or(fallback)
    }

    /// D1: upsert before the login starts, since an upsert during a login kills it.
    fn sign_in(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let identity = self.identity();
        let (label, id) = if identity.untouched(self.driver) {
            provider_instances::next_numbered(self.driver, driver_name(self.driver), self.taken())
        } else {
            if self.id_error().is_some() {
                self.go(Step::Identity, cx);
                return;
            }
            (self.label_or_default(&identity), self.instance_id())
        };
        let instance = ProviderInstance {
            id,
            driver: self.driver,
            label,
            accent: identity.accent,
            config: InstanceConfig::Native {
                executable: None,
                home_dir: Some(HomeDir::Managed),
                launch_args: Vec::new(),
                env: BTreeMap::new(),
            },
        };
        self.upsert(instance, AfterSave::Close { sign_in: true }, cx);
    }

    fn upsert(&mut self, instance: ProviderInstance, after: AfterSave, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.error = Some("Engine not connected".into());
            cx.notify();
            return;
        };
        self.saving = true;
        self.error = None;
        let target = self.target_device.clone().map(serde_json::Value::String);
        self.save_task = Some(cx.spawn(async move |this, cx| {
            let result = provider_instances::write(
                &engine,
                ProviderInstanceWrite::Upsert(instance.clone()),
                target,
            )
            .await;
            this.update(cx, |dialog, cx| {
                dialog.saving = false;
                match (result, after) {
                    (Ok(instances), AfterSave::Close { sign_in }) => {
                        cx.emit(AddProviderEvent::Saved {
                            instances,
                            instance,
                            sign_in,
                        })
                    }
                    (Ok(instances), AfterSave::SignIn) => {
                        dialog.instances = instances.clone();
                        cx.emit(AddProviderEvent::Created { instances });
                        dialog.open_sign_in(instance, cx);
                    }
                    (Err(error), _) => dialog.error = Some(error.into()),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn render_steps(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let identity = self.identity();
        let provider_summary = match self.acp {
            None => Some(driver_name(self.driver).to_string()),
            Some(AcpFlow::Local) => Some("Local ACP command".to_string()),
            Some(_) => self
                .selected
                .as_ref()
                .map(|selected| selected.agent.name.clone())
                .or_else(|| {
                    self.sign_in
                        .as_ref()
                        .map(|sign_in| sign_in.instance.label.clone())
                })
                .or_else(|| {
                    Some(self.manual_input.read(cx).text().trim().to_string())
                        .filter(|id| !id.is_empty())
                }),
        };
        let identity_summary = match &self.sign_in {
            Some(sign_in) => Some(sign_in.instance.label.clone()),
            None => (self.step > Step::Identity || self.editing.is_some())
                .then(|| identity.label.trim().to_string())
                .filter(|label| !label.is_empty()),
        };
        let summaries = [provider_summary, identity_summary, None];
        let steps = self.steps();
        div()
            .mt(px(12.0))
            .p(px(3.0))
            .rounded(px(10.0))
            .bg(ink(0.04))
            .flex()
            .flex_row()
            .gap(px(4.0))
            .children(steps.iter().copied().zip(summaries).enumerate().map(
                |(ix, (step, summary))| {
                    let current = step == self.step;
                    let done = step < self.step;
                    let reachable = self.step_reachable(step, cx);
                    let number = div()
                        .flex_none()
                        .size(px(18.0))
                        .rounded_full()
                        .border_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(crate::typography::ui_rems(11.0))
                        .map(|n| {
                            if done {
                                n.bg(theme.accent).border_color(theme.accent).child(
                                    icon(icons::CHECK)
                                        .size(px(11.0))
                                        .text_color(theme.on_accent),
                                )
                            } else if current {
                                n.border_color(theme.accent)
                                    .text_color(theme.accent)
                                    .child(SharedString::from((ix + 1).to_string()))
                            } else {
                                n.border_color(theme.border_strong)
                                    .child(SharedString::from((ix + 1).to_string()))
                            }
                        });
                    div()
                        .id(("add-provider-step", ix))
                        .flex_1()
                        .min_w_0()
                        .px(px(8.0))
                        .py(px(6.0))
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(gpui::transparent_black())
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .text_size(crate::typography::ui_rems(13.0))
                        .text_color(if current {
                            theme.text
                        } else {
                            theme.text_muted
                        })
                        .when(current, |el| el.bg(ink(0.06)).border_color(theme.border))
                        .when(reachable && !current, |el| {
                            el.cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| this.go(step, cx)))
                        })
                        .child(number)
                        .child(
                            div()
                                .flex_none()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .child(step.title()),
                        )
                        .when_some(summary.filter(|_| !current), |el, summary| {
                            el.child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(crate::typography::ui_rems(12.0))
                                    .text_color(theme.text_faint)
                                    .child(SharedString::from(format!("· {summary}"))),
                            )
                        })
                },
            ))
            .into_any_element()
    }

    fn render_provider_step(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let tiles = DRIVERS.into_iter().enumerate().map(|(ix, driver)| {
            let selected = self.acp.is_none() && driver == self.driver;
            let not_installed = self
                .harnesses
                .iter()
                .any(|descriptor| descriptor.id == driver && !descriptor.installed);
            let (mark, tint) = crate::pickers::harness_brand_icon(driver);
            div()
                .id(("add-provider-tile", ix))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .px(px(12.0))
                .py(px(10.0))
                .rounded(px(9.0))
                .border_2()
                .cursor_pointer()
                .map(|tile| {
                    if selected {
                        tile.bg(theme.accent.opacity(0.15))
                            .border_color(theme.accent)
                    } else {
                        tile.bg(ink(0.03))
                            .border_color(ink(0.06))
                            .hover(|s| s.bg(ink(0.06)))
                    }
                })
                .on_click(cx.listener(move |this, _, _, cx| this.select_driver(driver, cx)))
                .child(
                    icon(mark)
                        .size(px(16.0))
                        .flex_none()
                        .text_color(tint.unwrap_or(theme.text)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(driver_name(driver)),
                )
                .when(not_installed, |tile| {
                    tile.child(
                        div()
                            .flex_none()
                            .text_size(crate::typography::ui_rems(12.0))
                            .text_color(theme.text_faint)
                            .child("Not installed"),
                    )
                })
                .when(selected, |tile| {
                    tile.child(
                        div()
                            .flex_none()
                            .size(px(18.0))
                            .rounded_full()
                            .bg(theme.accent)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                icon(icons::CHECK)
                                    .size(px(11.0))
                                    .text_color(theme.on_accent),
                            ),
                    )
                })
        });
        let rule = || div().flex_1().h(px(1.0)).bg(theme.border);
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .mb(px(8.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child("Provider"),
            )
            .child(div().grid().grid_cols(2).gap(px(8.0)).children(tiles))
            .child(
                div()
                    .mt(px(18.0))
                    .mb(px(12.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .child(rule())
                    .child("Or choose from ACP Registry")
                    .child(rule()),
            )
            .child(self.render_acp_section(theme, cx))
            .into_any_element()
    }

    fn render_identity_step(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let identity = self.identity();
        let id_error = self.id_error();
        let id_control = match &self.editing {
            Some(instance) => readonly_value(theme, instance.id.clone()),
            None => field(theme, &self.id_input, id_error.is_some()),
        };
        let swatch = |id: (&'static str, usize), on: bool| {
            div()
                .id(id)
                .size(px(24.0))
                .rounded_full()
                .border_2()
                .border_color(if on {
                    theme.text
                } else {
                    gpui::transparent_black()
                })
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
        };
        let swatches =
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(2.0))
                .child(
                    swatch(("add-provider-accent-none", 0), identity.accent.is_none())
                        .on_click(cx.listener(|this, _, _, cx| this.set_accent(None, cx)))
                        .child(
                            div()
                                .size(px(16.0))
                                .rounded_full()
                                .border_1()
                                .border_dashed()
                                .border_color(theme.border_strong),
                        ),
                )
                .children(provider_instances::ACCENTS.into_iter().enumerate().map(
                    |(ix, accent)| {
                        let on = identity.accent.as_deref() == Some(accent);
                        swatch(("add-provider-accent", ix), on)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_accent(Some(accent.to_string()), cx)
                            }))
                            .child(
                                div()
                                    .size(px(16.0))
                                    .rounded_full()
                                    .bg(provider_instances::accent_color(accent)
                                        .unwrap_or(theme.text_muted)),
                            )
                    },
                ));
        div()
            .flex()
            .flex_col()
            .children(self.render_picked(theme, cx))
            .child(setting_row(
                theme,
                "Label",
                "Shown in the provider list and model picker.".into(),
                None,
                field(theme, &self.label_input, false),
                true,
            ))
            .child(setting_row(
                theme,
                "Instance ID",
                "Letters, digits, '-', or '_'.".into(),
                id_error,
                id_control,
                true,
            ))
            .child(setting_row(
                theme,
                "Accent color",
                "Optional marker shown in the picker.".into(),
                None,
                swatches.into_any_element(),
                false,
            ))
            .into_any_element()
    }

    fn render_config_step(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        if self.driver == HarnessId::Acp {
            return self.render_acp_edit_config(theme, cx);
        }
        let name = driver_name(self.driver);
        let cursor = self.driver == HarnessId::Cursor;
        let mut rows = div().flex().flex_col();
        if cursor {
            rows = rows.child(
                div()
                    .py(px(12.0))
                    .border_b_1()
                    .border_color(theme.border)
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .child(
                        "Cursor instances run on Zeron's managed Cursor SDK and take only \
                         environment variables.",
                    ),
            );
        } else {
            rows = rows.child(setting_row(
                theme,
                "Binary path",
                format!("Path to the {name} binary used by this instance.").into(),
                None,
                field(theme, &self.binary_input, false),
                true,
            ));
            if let Some((label, _)) = home_field(self.driver) {
                rows = rows.child(setting_row(
                    theme,
                    label,
                    "Separate home so this instance keeps its own login and settings.".into(),
                    None,
                    field(theme, &self.home_input, false),
                    true,
                ));
            }
            rows = rows.child(setting_row(
                theme,
                "Launch arguments",
                "Additional CLI arguments passed on session start.".into(),
                None,
                field(theme, &self.args_input, false),
                true,
            ));
        }
        rows.child(self.env_row(theme, "Set for this instance only.", cx))
            .into_any_element()
    }

    fn env_row(
        &self,
        theme: &Theme,
        description: &'static str,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let env = div()
            .w(px(240.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .children(self.env_rows.iter().enumerate().map(|(ix, row)| {
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .w(px(90.0))
                            .flex_none()
                            .child(field(theme, &row.key, false)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(field(theme, &row.value, false)),
                    )
                    .child(
                        widgets::ghost_action(theme)
                            .id(("add-provider-env-remove", ix))
                            .flex_none()
                            .px(px(6.0))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if ix < this.env_rows.len() {
                                    this.env_rows.remove(ix);
                                }
                                cx.notify();
                            }))
                            .child(
                                icon(icons::CLOSE)
                                    .size(px(12.0))
                                    .text_color(theme.text_muted),
                            ),
                    )
            }))
            .child(
                div().flex().child(
                    widgets::ghost_action(theme)
                        .id("add-provider-env-add")
                        .text_size(crate::typography::ui_rems(12.0))
                        .on_click(cx.listener(|this, _, _, cx| this.add_env_row("", "", cx)))
                        .child("+ Add variable"),
                ),
            );
        div()
            .py(px(12.0))
            .flex()
            .flex_row()
            .items_start()
            .gap(px(16.0))
            .child(row_text(theme, "Environment", description.into(), None))
            .child(div().flex_none().child(env))
    }

    fn render_footer(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let button = |id: &'static str, tone: widgets::ActionTone| {
            widgets::action_button(theme, tone)
                .id(id)
                .flex_none()
                .tab_index(0)
                .role(gpui::Role::Button)
                .cursor_pointer()
                .focus_visible(|s| s.border_2().border_color(theme.accent))
        };
        let saving = self.saving;
        let mut footer = div()
            .px(px(20.0))
            .py(px(12.0))
            .border_t_1()
            .border_color(theme.border)
            .bg(ink(0.02))
            .flex()
            .flex_row()
            .items_center()
            .justify_end()
            .gap(px(8.0));
        let busy = self.busy();
        if self.step == Step::SignIn {
            let (active, signed_in) = self.sign_in.as_ref().map_or((false, false), |sign_in| {
                (sign_in.active(), sign_in.signed_in)
            });
            return footer
                .child(
                    button(
                        "add-provider-finish",
                        if signed_in {
                            widgets::ActionTone::Solid
                        } else {
                            widgets::ActionTone::Filled
                        },
                    )
                    .when(active, |el| el.opacity(0.5).cursor_default())
                    .when(!active, |el| {
                        el.on_click(cx.listener(|this, _, _, cx| this.close(cx)))
                    })
                    .child(if signed_in { "Done" } else { "Skip for now" }),
                )
                .into_any_element();
        }
        let first_step =
            self.step == Step::Provider || (self.step == Step::Identity && self.editing.is_some());
        footer = footer.child(
            button(
                "add-provider-back",
                if first_step {
                    widgets::ActionTone::Quiet
                } else {
                    widgets::ActionTone::Filled
                },
            )
            .when(busy, |el| el.opacity(0.5).cursor_default())
            .when(!busy, |el| {
                el.on_click(cx.listener(|this, _, _, cx| this.back(cx)))
            })
            .child(if first_step { "Cancel" } else { "Back" }),
        );
        let acp_new = self.acp.is_some() && self.editing.is_none();
        match self.step {
            Step::Provider if acp_new => {
                let preparing = self.registry.preparing.is_some();
                footer = footer.child(
                    button("add-provider-next", widgets::ActionTone::Solid)
                        .when(preparing, |el| el.opacity(0.5).cursor_default())
                        .when(!preparing, |el| {
                            el.on_click(cx.listener(|this, _, _, cx| this.acp_next(cx)))
                        })
                        .child(if preparing {
                            "Preparing…"
                        } else if self.manual_unverified_armed(cx) {
                            "Add anyway"
                        } else {
                            "Next"
                        }),
                );
            }
            Step::Provider => {
                footer = footer.child(
                    button("add-provider-manual", widgets::ActionTone::Filled)
                        .when(busy, |el| el.opacity(0.5).cursor_default())
                        .when(!busy, |el| {
                            el.on_click(cx.listener(|this, _, _, cx| this.go(Step::Identity, cx)))
                        })
                        .child("Configure manually"),
                );
                footer = footer.child(match sign_in_action(self.driver) {
                    Some((label, mark)) => {
                        button("add-provider-sign-in", widgets::ActionTone::Solid)
                            .when(busy, |el| el.opacity(0.5))
                            .when(!busy, |el| {
                                el.on_click(cx.listener(|this, _, _, cx| this.sign_in(cx)))
                            })
                            .child(icon(mark).size(px(14.0)).text_color(theme.on_solid))
                            .child(label)
                    }
                    None => button("add-provider-next", widgets::ActionTone::Solid)
                        .when(busy, |el| el.opacity(0.5))
                        .when(!busy, |el| {
                            el.on_click(cx.listener(|this, _, _, cx| this.go(Step::Identity, cx)))
                        })
                        .child("Next"),
                });
            }
            Step::Identity if !acp_new => {
                footer = footer.child(
                    button("add-provider-next", widgets::ActionTone::Solid)
                        .on_click(cx.listener(|this, _, _, cx| this.go(Step::Config, cx)))
                        .child("Next"),
                );
            }
            Step::Identity | Step::Config | Step::SignIn => {
                let label = match (self.editing.is_some(), saving) {
                    (true, true) => "Saving…",
                    (true, false) => "Save",
                    (false, true) => "Adding…",
                    (false, false) if acp_new && self.acp != Some(AcpFlow::Local) => {
                        "Continue to sign-in"
                    }
                    (false, false) => "Add instance",
                };
                footer = footer.child(
                    button("add-provider-save", widgets::ActionTone::Solid)
                        .when(saving, |el| el.opacity(0.5))
                        .on_click(cx.listener(|this, _, _, cx| this.save(cx)))
                        .child(label),
                );
            }
        }
        footer.into_any_element()
    }

    fn render_modal(
        &self,
        viewport: gpui::Size<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).for_popup();
        let title = if self.editing.is_some() {
            "Edit provider"
        } else {
            "Add provider"
        };
        let body = match self.step {
            Step::Provider => self.render_provider_step(&theme, cx),
            Step::Identity => self.render_identity_step(&theme, cx),
            Step::Config => self.render_config_step(&theme, cx),
            Step::SignIn => self.render_sign_in_step(&theme, cx),
        };
        let max_body = (f32::from(viewport.height) - 260.0).max(200.0);
        let header = div()
            .relative()
            .px(px(20.0))
            .pt(px(18.0))
            .pb(px(14.0))
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .text_size(crate::typography::ui_rems(18.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .mt(px(4.0))
                    .flex()
                    .flex_row()
                    .text_color(theme.text_muted)
                    .child("Add an account or configure a provider on\u{a0}")
                    .child(div().text_color(theme.text).child(self.device_name.clone()))
                    .child("."),
            )
            .child(
                div()
                    .id("add-provider-close")
                    .absolute()
                    .top(px(14.0))
                    .right(px(14.0))
                    .size(px(24.0))
                    .rounded(px(6.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| s.bg(ink(0.06)))
                    .role(gpui::Role::Button)
                    .aria_label("Close")
                    .on_click(cx.listener(|this, _, _, cx| this.close(cx)))
                    .child(
                        icon(icons::CLOSE)
                            .size(px(14.0))
                            .text_color(theme.text_muted),
                    ),
            )
            .child(self.render_steps(&theme, cx));
        let card = popover::dialog_card(&theme)
            .id("add-provider-card")
            .w(px(660.0))
            .max_w(viewport.width - px(48.0))
            .p_0()
            .overflow_hidden()
            .text_size(crate::typography::ui_rems(13.0))
            .role(gpui::Role::Dialog)
            .aria_label(title)
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.close(cx);
                    cx.stop_propagation();
                }
            }))
            .child(header)
            .child(
                div()
                    .id("add-provider-body")
                    .px(px(20.0))
                    .py(px(16.0))
                    .min_h(px(300.0))
                    .max_h(px(max_body))
                    .overflow_y_scroll()
                    .child(body),
            )
            .when_some(self.error.clone(), |el, error| {
                el.child(
                    div()
                        .px(px(20.0))
                        .pb(px(10.0))
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.danger)
                        .child(error),
                )
            })
            .child(self.render_footer(&theme, cx))
            .into_any_element();
        popover::modal("add-provider-dialog", viewport, card)
    }
}

impl Render for AddProviderDialog {
    fn render(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_modal(window.viewport_size(), cx)
    }
}

fn mono_input(placeholder: &'static str, cx: &mut Context<ComposerInput>) -> ComposerInput {
    ComposerInput::new(placeholder, cx)
        .with_single_line()
        .with_mono()
        .with_text_metrics(12.0, 16.0)
}

fn readonly_value(theme: &Theme, value: String) -> AnyElement {
    div()
        .w_full()
        .h(px(30.0))
        .px(px(10.0))
        .flex()
        .items_center()
        .justify_end()
        .font_family(theme.font_mono.clone())
        .text_size(crate::typography::ui_rems(12.0))
        .text_color(theme.text_muted)
        .child(SharedString::from(value))
        .into_any_element()
}

fn field(theme: &Theme, input: &Entity<ComposerInput>, error: bool) -> AnyElement {
    div()
        .w_full()
        .h(px(30.0))
        .px(px(10.0))
        .rounded(px(7.0))
        .border_1()
        .border_color(if error { theme.danger } else { theme.border })
        .bg(ink(0.02))
        .flex()
        .items_center()
        .overflow_hidden()
        .child(div().w_full().child(input.clone()))
        .into_any_element()
}

fn row_text(
    theme: &Theme,
    title: &'static str,
    description: SharedString,
    error: Option<String>,
) -> gpui::Div {
    div()
        .flex_1()
        .min_w_0()
        .child(div().font_weight(gpui::FontWeight::MEDIUM).child(title))
        .child(
            div()
                .mt(px(2.0))
                .text_size(crate::typography::ui_rems(12.0))
                .text_color(theme.text_muted)
                .child(description),
        )
        .when_some(error, |el, error| {
            el.child(
                div()
                    .mt(px(2.0))
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.danger)
                    .child(SharedString::from(error)),
            )
        })
}

fn setting_row(
    theme: &Theme,
    title: &'static str,
    description: SharedString,
    error: Option<String>,
    control: AnyElement,
    divider: bool,
) -> gpui::Div {
    div()
        .py(px(12.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(16.0))
        .when(divider, |el| el.border_b_1().border_color(theme.border))
        .child(row_text(theme, title, description, error))
        .child(
            div()
                .w(px(240.0))
                .flex_none()
                .flex()
                .justify_end()
                .child(control),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_invalid_id_holds_the_wizard_on_identity() {
        assert_eq!(resolve_step(Step::Config, false, true), Step::Identity);
        assert_eq!(resolve_step(Step::Config, false, false), Step::Config);
        assert_eq!(resolve_step(Step::Provider, false, true), Step::Provider);
        assert_eq!(resolve_step(Step::Provider, true, false), Step::Identity);
    }

    #[gpui::test]
    fn the_id_follows_the_label_until_the_user_edits_it(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            crate::settings::init(Default::default(), dir.path(), cx);
            gpui_base::init(cx);
            cx.set_global(Theme::default());
        });
        let window = cx.add_window(|_, cx| {
            let state = cx.new(|_| AppState::new());
            AddProviderDialog::new(
                state,
                None,
                "This Mac".into(),
                Vec::new(),
                Vec::new(),
                None,
                cx,
            )
        });
        let id = |cx: &mut gpui::TestAppContext| {
            window
                .update(cx, |dialog, _, cx| {
                    (
                        dialog.id_input.read(cx).text().to_string(),
                        dialog.instance_id(),
                    )
                })
                .unwrap()
        };
        let type_into = |cx: &mut gpui::TestAppContext, label: bool, text: &str| {
            let text = text.to_string();
            window
                .update(cx, |dialog, _, cx| {
                    let input = if label {
                        dialog.label_input.clone()
                    } else {
                        dialog.id_input.clone()
                    };
                    input.update(cx, |input, cx| input.set_text(text, cx));
                })
                .unwrap();
            cx.run_until_parked();
        };
        cx.run_until_parked();
        assert_eq!(id(cx), ("codex_2".into(), "codex_2".into()));

        type_into(cx, true, "Work");
        assert_eq!(id(cx), ("codex_work".into(), "codex_work".into()));

        type_into(cx, false, "team_a");
        type_into(cx, true, "Personal");
        assert_eq!(id(cx), ("team_a".into(), "team_a".into()));

        type_into(cx, false, "codex_personal");
        type_into(cx, true, "Home");
        assert_eq!(id(cx), ("codex_home".into(), "codex_home".into()));

        window
            .update(cx, |dialog, _, cx| {
                dialog.select_driver(HarnessId::ClaudeCode, cx);
                dialog.select_driver(HarnessId::Codex, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(id(cx), ("codex_home".into(), "codex_home".into()));
    }
}
