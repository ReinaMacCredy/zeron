use std::collections::BTreeMap;
use std::time::Duration;

use gpui::{AnyElement, Context, SharedString, Task, div, prelude::*, px};
use zeron_proto::{
    AcpAuthMethod, AcpDistribution, AcpPreparedAgent, AcpRegistryAgent, AcpSource, AgentLoginPoll,
    AgentLoginStart, AgentLoginStatus, HarnessId, InstanceConfig, ProviderInstance,
};
use zeron_rpc::methods;

use super::{AddProviderDialog, Identity, Step, field, readonly_value, setting_row};
use crate::icons::{self, icon};
use crate::popover::Loadable;
use crate::provider_instances::{self, InstanceIcon};
use crate::settings::accounts::open_login_url;
use crate::settings::widgets;
use crate::theme::{Theme, ink};

const SEARCH_DEBOUNCE: Duration = Duration::from_millis(300);
const ICON_REFRESH: Duration = Duration::from_millis(1500);
const LOGIN_POLL: Duration = Duration::from_millis(1500);
const NO_IN_APP_SIGN_IN: &str =
    "No in-app sign-in advertised. Follow the provider's docs to finish setup.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpFlow {
    Registry,
    Manual,
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RegistryKnob {
    Preparing,
    Prepare,
}

#[derive(Debug, Clone)]
pub(super) struct SelectedAgent {
    pub agent: AcpRegistryAgent,
    pub distribution: AcpDistribution,
}

#[derive(Default)]
pub(super) struct RegistryState {
    pub query: String,
    pub results: Loadable<Vec<AcpRegistryAgent>>,
    pub error: Option<String>,
    pub preparing: Option<String>,
    pub prepare_error: Option<String>,
    unverified_armed: Option<String>,
    pub knob: Option<RegistryKnob>,
    generation: u64,
    search_task: Option<Task<()>>,
    debounce: Option<Task<()>>,
    refresh_task: Option<Task<()>>,
    prepare_task: Option<Task<()>>,
}

struct Login {
    login_id: Option<String>,
    url: Option<String>,
    message: Option<String>,
}

pub(super) struct SignInState {
    pub instance: ProviderInstance,
    methods: Loadable<Vec<AcpAuthMethod>>,
    docs: Option<String>,
    login: Option<Login>,
    pub signed_in: bool,
    error: Option<String>,
    attempt: u64,
    methods_task: Option<Task<()>>,
    docs_task: Option<Task<()>>,
    login_task: Option<Task<()>>,
}

impl SignInState {
    pub fn active(&self) -> bool {
        self.login.is_some()
    }
}

fn distribution_label(agent: &AcpRegistryAgent) -> Option<&'static str> {
    agent
        .distributions
        .first()
        .map(|distribution| provider_instances::distribution_name(*distribution))
}

fn about_url(agent: &AcpRegistryAgent) -> Option<String> {
    agent.website.clone().or_else(|| agent.repository.clone())
}

impl AddProviderDialog {
    pub(super) fn target(&self) -> Option<serde_json::Value> {
        self.target_device.clone().map(serde_json::Value::String)
    }

    pub(super) fn search_edited(&mut self, cx: &mut Context<Self>) {
        let query = self.search_input.read(cx).text().trim().to_string();
        self.registry.prepare_error = None;
        self.registry.unverified_armed = None;
        self.registry.debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DEBOUNCE).await;
            this.update(cx, |dialog, cx| {
                if dialog.registry.query != query {
                    dialog.registry.query = query;
                    dialog.search(false, cx);
                }
            })
            .ok();
        }));
    }

    pub(super) fn search(&mut self, quiet: bool, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let target = self.target();
        let registry = &mut self.registry;
        registry.generation += 1;
        let generation = registry.generation;
        if !quiet {
            registry.refresh_task = None;
            if registry.results.ready().is_none() {
                registry.results = Loadable::Loading;
            }
        }
        let query = registry.query.clone();
        registry.search_task = Some(cx.spawn(async move |this, cx| {
            let result = provider_instances::search_registry(&engine, &query, target).await;
            this.update(cx, |dialog, cx| {
                dialog.searched(generation, quiet, result, cx)
            })
            .ok();
        }));
        cx.notify();
    }

    fn searched(
        &mut self,
        generation: u64,
        quiet: bool,
        result: Result<Vec<AcpRegistryAgent>, String>,
        cx: &mut Context<Self>,
    ) {
        if self.registry.generation != generation {
            return;
        }
        match result {
            Ok(rows) => {
                let icons_pending = rows
                    .iter()
                    .any(|row| row.icon_url.is_some() && row.icon_path.is_none());
                self.registry.error = None;
                self.registry.results = Loadable::Ready(rows);
                if icons_pending && !quiet {
                    self.registry.refresh_task = Some(cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(ICON_REFRESH).await;
                        this.update(cx, |dialog, cx| dialog.search(true, cx)).ok();
                    }));
                }
                self.apply_registry_knob(cx);
            }
            Err(error) if quiet => tracing::debug!(error, "ACP Registry icon refresh failed"),
            Err(error) => {
                if self.registry.results.ready().is_none() {
                    self.registry.results = Loadable::Error(error.clone());
                }
                self.registry.error = Some(error);
            }
        }
        cx.notify();
    }

    fn apply_registry_knob(&mut self, cx: &mut Context<Self>) {
        let Some(knob) = self.registry.knob.take() else {
            return;
        };
        let Some(first) = self
            .registry
            .results
            .ready()
            .and_then(|rows| rows.first())
            .cloned()
        else {
            return;
        };
        match knob {
            RegistryKnob::Preparing => self.registry.preparing = Some(first.id),
            RegistryKnob::Prepare => self.prepare(first, false, cx),
        }
    }

    pub fn apply_acp_knob(
        &mut self,
        step: &str,
        query: Option<String>,
        instance: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(query) = query {
            self.search_input
                .update(cx, |input, cx| input.set_text(query.clone(), cx));
            self.registry.query = query.trim().to_string();
            self.search(false, cx);
        }
        match step {
            "registry-preparing" => self.registry.knob = Some(RegistryKnob::Preparing),
            "acp-identity" => self.registry.knob = Some(RegistryKnob::Prepare),
            "local" => self.enter_local(cx),
            "manual" => self.enter_manual(cx),
            "signin" => {
                if let Some(id) = instance {
                    self.sign_in_to(id, cx);
                }
            }
            _ => {}
        }
        if self.registry.knob.is_some() && self.registry.results.ready().is_some() {
            self.apply_registry_knob(cx);
        }
        cx.notify();
    }

    pub(super) fn prepare(
        &mut self,
        agent: AcpRegistryAgent,
        allow_unverified: bool,
        cx: &mut Context<Self>,
    ) {
        if self.registry.preparing.is_some() {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.registry.prepare_error = Some("Engine not connected".into());
            cx.notify();
            return;
        };
        self.registry.preparing = Some(agent.id.clone());
        self.registry.prepare_error = None;
        self.registry.unverified_armed = None;
        self.acp_error = None;
        let target = self.target();
        self.registry.prepare_task = Some(cx.spawn(async move |this, cx| {
            let result =
                provider_instances::prepare_agent(&engine, &agent.id, allow_unverified, target)
                    .await;
            this.update(cx, |dialog, cx| {
                dialog.registry.preparing = None;
                match result {
                    Ok(prepared) => dialog.select_prepared(agent, prepared, cx),
                    Err(error) => {
                        dialog.registry.prepare_error = Some(if error.trim().is_empty() {
                            "The ACP could not be prepared.".into()
                        } else {
                            error
                        })
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn select_prepared(
        &mut self,
        mut agent: AcpRegistryAgent,
        prepared: AcpPreparedAgent,
        cx: &mut Context<Self>,
    ) {
        agent.id = prepared.agent_id;
        agent.version = prepared.version;
        agent.name = prepared.display_name.clone();
        agent.icon_path = prepared.icon_path.or(agent.icon_path);
        self.enter_acp(AcpFlow::Registry, &prepared.display_name, cx);
        self.selected = Some(SelectedAgent {
            agent,
            distribution: prepared.distribution,
        });
        self.step = Step::Identity;
    }

    fn enter_acp(&mut self, flow: AcpFlow, label: &str, cx: &mut Context<Self>) {
        self.acp = Some(flow);
        self.selected = None;
        self.driver = HarnessId::Acp;
        let accent = self
            .identities
            .get(&HarnessId::Acp)
            .and_then(|identity| identity.accent.clone());
        self.identities.insert(
            HarnessId::Acp,
            Identity {
                label: label.to_string(),
                accent,
                id_override: None,
            },
        );
        self.load_driver(cx);
        self.acp_error = None;
        self.error = None;
        cx.notify();
    }

    pub(super) fn enter_manual(&mut self, cx: &mut Context<Self>) {
        self.enter_acp(AcpFlow::Manual, "", cx);
        self.manual_input
            .update(cx, |input, cx| input.set_text("", cx));
    }

    pub(super) fn enter_local(&mut self, cx: &mut Context<Self>) {
        self.enter_acp(AcpFlow::Local, "Local ACP", cx);
        self.command_input
            .update(cx, |input, cx| input.set_text("", cx));
        self.acp_args_input
            .update(cx, |input, cx| input.set_text("", cx));
    }

    pub(super) fn back_to_search(&mut self, cx: &mut Context<Self>) {
        self.registry.prepare_task = None;
        self.registry.preparing = None;
        self.acp = None;
        self.selected = None;
        self.driver = self.tile;
        self.load_driver(cx);
        self.acp_error = None;
        cx.notify();
    }

    pub(super) fn acp_selection_ok(&self, cx: &Context<Self>) -> bool {
        match self.acp {
            Some(AcpFlow::Registry) => self.selected.is_some(),
            Some(AcpFlow::Local) => !self.command_input.read(cx).text().trim().is_empty(),
            Some(AcpFlow::Manual) | None => false,
        }
    }

    pub(super) fn acp_next(&mut self, cx: &mut Context<Self>) {
        match self.acp {
            Some(AcpFlow::Manual) => {
                let id = self.manual_input.read(cx).text().trim().to_string();
                if id.is_empty() {
                    self.acp_error = Some("Select an ACP or configure one manually.".into());
                    cx.notify();
                    return;
                }
                let agent = self
                    .registry
                    .results
                    .ready()
                    .and_then(|rows| rows.iter().find(|row| row.id == id))
                    .cloned()
                    .unwrap_or_else(|| AcpRegistryAgent {
                        id: id.clone(),
                        name: id,
                        version: String::new(),
                        description: String::new(),
                        website: None,
                        repository: None,
                        distributions: Vec::new(),
                        icon_url: None,
                        icon_path: None,
                        installed: false,
                        unverified: false,
                    });
                self.prepare(agent, false, cx);
            }
            Some(AcpFlow::Local) if !self.acp_selection_ok(cx) => {
                self.acp_error = Some("Executable is required.".into());
                cx.notify();
            }
            _ if self.acp_selection_ok(cx) => self.go(Step::Identity, cx),
            _ => {}
        }
    }

    pub(super) fn acp_config(
        &self,
        env: BTreeMap<String, String>,
        cx: &Context<Self>,
    ) -> Option<InstanceConfig> {
        let local = || AcpSource::Local {
            command: self.command_input.read(cx).text().trim().to_string(),
            args: self
                .acp_args_input
                .read(cx)
                .text()
                .split_whitespace()
                .map(str::to_owned)
                .collect(),
        };
        let (source, icon_path) = match &self.editing {
            Some(ProviderInstance {
                config:
                    InstanceConfig::Acp {
                        source, icon_path, ..
                    },
                ..
            }) => match source {
                AcpSource::Registry { .. } => (source.clone(), icon_path.clone()),
                AcpSource::Local { .. } => (local(), icon_path.clone()),
            },
            Some(_) => return None,
            None => match (self.acp, &self.selected) {
                (Some(AcpFlow::Local), _) => (local(), None),
                (Some(AcpFlow::Registry), Some(selected)) => (
                    AcpSource::Registry {
                        agent_id: selected.agent.id.clone(),
                        version: selected.agent.version.clone(),
                        distribution: selected.distribution,
                    },
                    selected.agent.icon_path.clone(),
                ),
                _ => return None,
            },
        };
        Some(InstanceConfig::Acp {
            source,
            env,
            icon_path,
        })
    }

    pub fn sign_in_to(&mut self, instance_id: String, cx: &mut Context<Self>) {
        self.pending_sign_in = Some(instance_id);
        self.try_pending_sign_in(cx);
    }

    pub(super) fn try_pending_sign_in(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.pending_sign_in.clone() else {
            return;
        };
        let Some(instance) = self.instances.iter().find(|i| i.id == id).cloned() else {
            return;
        };
        self.pending_sign_in = None;
        if self.acp.is_none() {
            self.acp = Some(AcpFlow::Registry);
            self.driver = HarnessId::Acp;
        }
        self.open_sign_in(instance, cx);
    }

    pub(super) fn open_sign_in(&mut self, instance: ProviderInstance, cx: &mut Context<Self>) {
        let docs = self
            .selected
            .as_ref()
            .and_then(|selected| about_url(&selected.agent));
        let lookup = docs.is_none();
        self.sign_in = Some(SignInState {
            instance,
            methods: Loadable::Loading,
            docs,
            login: None,
            signed_in: false,
            error: None,
            attempt: 0,
            methods_task: None,
            docs_task: None,
            login_task: None,
        });
        self.step = Step::SignIn;
        self.load_auth_methods(cx);
        if lookup {
            self.lookup_docs(cx);
        }
        cx.notify();
    }

    fn load_auth_methods(&mut self, cx: &mut Context<Self>) {
        let engine = self.state.read(cx).engine().cloned();
        let target = self.target();
        let Some(sign_in) = self.sign_in.as_mut() else {
            return;
        };
        let Some(engine) = engine else {
            sign_in.methods = Loadable::Error("Engine not connected".into());
            return;
        };
        let id = sign_in.instance.id.clone();
        sign_in.methods_task = Some(cx.spawn(async move |this, cx| {
            let result = provider_instances::auth_methods(&engine, &id, target).await;
            this.update(cx, |dialog, cx| {
                if let Some(sign_in) = dialog.sign_in.as_mut().filter(|s| s.instance.id == id) {
                    sign_in.methods = match result {
                        Ok(methods) => Loadable::Ready(methods),
                        Err(error) => Loadable::Error(error),
                    };
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn lookup_docs(&mut self, cx: &mut Context<Self>) {
        let engine = self.state.read(cx).engine().cloned();
        let target = self.target();
        let Some(sign_in) = self.sign_in.as_mut() else {
            return;
        };
        let (
            Some(engine),
            InstanceConfig::Acp {
                source: AcpSource::Registry { agent_id, .. },
                ..
            },
        ) = (engine, &sign_in.instance.config)
        else {
            return;
        };
        let agent_id = agent_id.clone();
        sign_in.docs_task = Some(cx.spawn(async move |this, cx| {
            let Ok(rows) = provider_instances::search_registry(&engine, &agent_id, target).await
            else {
                return;
            };
            let docs = rows
                .iter()
                .find(|row| row.id == agent_id)
                .and_then(about_url);
            this.update(cx, |dialog, cx| {
                if let Some(sign_in) = dialog.sign_in.as_mut() {
                    sign_in.docs = docs;
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn start_login(&mut self, method_id: String, cx: &mut Context<Self>) {
        let engine = self.state.read(cx).engine().cloned();
        let target = self.target();
        let Some(sign_in) = self.sign_in.as_mut().filter(|s| s.login.is_none()) else {
            return;
        };
        let Some(engine) = engine else {
            sign_in.error = Some("Engine not connected".into());
            cx.notify();
            return;
        };
        sign_in.attempt += 1;
        let attempt = sign_in.attempt;
        sign_in.error = None;
        sign_in.login = Some(Login {
            login_id: None,
            url: None,
            message: None,
        });
        let instance = sign_in.instance.clone();
        let with_target = move |mut params: serde_json::Value| {
            if let Some(target) = &target {
                params["targetDeviceId"] = target.clone();
            }
            params
        };
        let start_params = with_target(serde_json::json!({
            "harness": instance.driver,
            "instanceId": instance.id,
            "methodId": method_id,
        }));
        sign_in.login_task = Some(cx.spawn(async move |this, cx| {
            let start = engine
                .client()
                .call_as::<AgentLoginStart>(methods::START_AGENT_LOGIN, start_params)
                .await
                .map_err(|error| error.to_string());
            let Ok(Some(login_id)) =
                this.update(cx, |dialog, cx| dialog.login_started(attempt, start, cx))
            else {
                return;
            };
            let poll_params =
                with_target(serde_json::json!({ "loginId": login_id, "instanceId": instance.id }));
            loop {
                cx.background_executor().timer(LOGIN_POLL).await;
                let poll = engine
                    .client()
                    .call_as::<AgentLoginPoll>(methods::POLL_AGENT_LOGIN, poll_params.clone())
                    .await
                    .map_err(|error| error.to_string());
                match this.update(cx, |dialog, cx| dialog.login_polled(attempt, poll, cx)) {
                    Ok(false) => {}
                    Ok(true) | Err(_) => break,
                }
            }
        }));
        cx.notify();
    }

    fn login_started(
        &mut self,
        attempt: u64,
        start: Result<AgentLoginStart, String>,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let sign_in = self.sign_in.as_mut().filter(|s| s.attempt == attempt)?;
        let login = sign_in.login.as_mut()?;
        let started = match start {
            Ok(start) => {
                if !start.url.is_empty() {
                    open_login_url(&start.url, cx);
                    login.url = Some(start.url);
                }
                login.login_id = Some(start.login_id.clone());
                Some(start.login_id)
            }
            Err(error) => {
                sign_in.login = None;
                sign_in.error = Some(format!("Couldn't start the sign-in: {error}"));
                None
            }
        };
        cx.notify();
        started
    }

    fn login_polled(
        &mut self,
        attempt: u64,
        poll: Result<AgentLoginPoll, String>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(sign_in) = self.sign_in.as_mut().filter(|s| s.attempt == attempt) else {
            return true;
        };
        let Some(login) = sign_in.login.as_mut() else {
            return true;
        };
        let finished = match poll {
            Ok(poll) => match poll.status {
                AgentLoginStatus::Done => {
                    sign_in.login = None;
                    sign_in.signed_in = true;
                    true
                }
                AgentLoginStatus::Error => {
                    sign_in.login = None;
                    sign_in.error = Some(
                        poll.message
                            .unwrap_or_else(|| "The sign-in failed.".to_string()),
                    );
                    true
                }
                AgentLoginStatus::Pending => {
                    if let Some(url) = poll.url.filter(|url| login.url.as_ref() != Some(url)) {
                        open_login_url(&url, cx);
                        login.url = Some(url);
                    }
                    login.message = poll.message;
                    false
                }
            },
            Err(error) => {
                sign_in.login = None;
                sign_in.error = Some(format!("Lost track of the sign-in: {error}"));
                true
            }
        };
        cx.notify();
        finished
    }

    pub(super) fn cancel_login(&mut self, cx: &mut Context<Self>) {
        let engine = self.state.read(cx).engine().cloned();
        let target = self.target();
        let Some(sign_in) = self.sign_in.as_mut() else {
            return;
        };
        let Some(login) = sign_in.login.take() else {
            return;
        };
        sign_in.login_task = None;
        if let (Some(login_id), Some(engine)) = (login.login_id, engine) {
            let mut params =
                serde_json::json!({ "loginId": login_id, "instanceId": sign_in.instance.id });
            if let Some(target) = target {
                params["targetDeviceId"] = target;
            }
            cx.spawn(async move |_, _| {
                if let Err(error) = engine
                    .client()
                    .call(methods::CANCEL_AGENT_LOGIN, params)
                    .await
                {
                    tracing::debug!(%error, "CancelAgentLogin failed (best-effort)");
                }
            })
            .detach();
        }
        cx.notify();
    }

    pub(super) fn render_acp_section(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let body = match self.acp {
            Some(AcpFlow::Manual) => self.render_manual_form(theme, cx),
            Some(AcpFlow::Local) => self.render_local_form(theme, cx),
            _ => self.render_registry_search(theme, cx),
        };
        div()
            .flex()
            .flex_col()
            .child(body)
            .when_some(self.acp_error.clone(), |el, error| {
                el.child(
                    div()
                        .mt(px(8.0))
                        .text_size(crate::typography::ui_rems(11.0))
                        .text_color(theme.danger)
                        .child(error),
                )
            })
            .into_any_element()
    }

    fn small_button(theme: &Theme, tone: widgets::ActionTone, enabled: bool) -> gpui::Div {
        widgets::action_button(theme, tone)
            .flex_none()
            .min_h(px(28.0))
            .py(px(3.0))
            .when(!enabled, |el| el.opacity(0.5).cursor_default())
    }

    fn form_header(
        &self,
        theme: &Theme,
        title: &'static str,
        description: SharedString,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .mb(px(4.0))
            .flex()
            .flex_row()
            .items_start()
            .justify_between()
            .gap(px(12.0))
            .child(
                div()
                    .min_w_0()
                    .child(div().font_weight(gpui::FontWeight::MEDIUM).child(title))
                    .child(
                        div()
                            .mt(px(2.0))
                            .text_size(crate::typography::ui_rems(12.0))
                            .text_color(theme.text_muted)
                            .child(description),
                    ),
            )
            .child(
                Self::small_button(theme, widgets::ActionTone::Quiet, true)
                    .id("acp-search-registry")
                    .on_click(cx.listener(|this, _, _, cx| this.back_to_search(cx)))
                    .child("Search registry"),
            )
            .into_any_element()
    }

    fn render_manual_form(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .child(self.form_header(
                theme,
                "Enter manually",
                "Enter an official ACP Registry agent ID.".into(),
                cx,
            ))
            .child(setting_row(
                theme,
                "Registry ID",
                "Agent identifier from the official ACP Registry, for example 'devin'.".into(),
                None,
                field(theme, &self.manual_input, false),
                false,
            ))
            .when_some(self.registry.prepare_error.clone(), |el, error| {
                el.child(error_alert(theme, error))
            })
            .into_any_element()
    }

    fn render_local_form(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let description = format!("Run an installed ACP executable on {}.", self.device_name);
        div()
            .flex()
            .flex_col()
            .child(self.form_header(theme, "Local ACP command", description.into(), cx))
            .child(setting_row(
                theme,
                "Executable",
                "Absolute path or a command on PATH.".into(),
                None,
                field(theme, &self.command_input, false),
                true,
            ))
            .child(setting_row(
                theme,
                "Arguments",
                "Passed after the executable.".into(),
                None,
                field(theme, &self.acp_args_input, false),
                true,
            ))
            .child(self.env_row(theme, "Set for this agent only.", cx))
            .into_any_element()
    }

    fn render_registry_search(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.registry.preparing.is_some();
        let search = div()
            .flex_1()
            .min_w(px(160.0))
            .h(px(30.0))
            .px(px(10.0))
            .rounded(px(7.0))
            .border_1()
            .border_color(theme.border)
            .bg(ink(0.02))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .overflow_hidden()
            .when(busy, |el| el.opacity(0.5))
            .child(
                icon(icons::MAGNIFER)
                    .size(px(13.0))
                    .text_color(theme.text_muted),
            )
            .child(div().flex_1().min_w_0().child(self.search_input.clone()));
        let controls = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .child(search)
            .child(
                Self::small_button(theme, widgets::ActionTone::Quiet, !busy)
                    .id("acp-enter-manually")
                    .when(!busy, |el| {
                        el.on_click(cx.listener(|this, _, _, cx| this.enter_manual(cx)))
                    })
                    .child("Enter manually"),
            )
            .child(
                Self::small_button(theme, widgets::ActionTone::Outlined, !busy)
                    .id("acp-local-command")
                    .when(!busy, |el| {
                        el.on_click(cx.listener(|this, _, _, cx| this.enter_local(cx)))
                    })
                    .child("Local ACP command"),
            );
        let error = self
            .registry
            .prepare_error
            .clone()
            .or_else(|| self.registry.error.clone());
        let list = match &self.registry.results {
            Loadable::Idle | Loadable::Error(_) => None,
            Loadable::Loading => Some(
                div()
                    .min_h(px(80.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.text_muted)
                    .child("Searching the registry...")
                    .into_any_element(),
            ),
            Loadable::Ready(rows) if rows.is_empty() => Some(
                div()
                    .min_h(px(112.0))
                    .px(px(16.0))
                    .rounded(px(16.0))
                    .border_1()
                    .border_dashed()
                    .border_color(theme.border_strong)
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child("No compatible agents found"),
                    )
                    .child(
                        div()
                            .mt(px(4.0))
                            .text_size(crate::typography::ui_rems(12.0))
                            .text_color(theme.text_muted)
                            .child("Try a broader search."),
                    )
                    .into_any_element(),
            ),
            Loadable::Ready(rows) => Some(
                div()
                    .id("acp-registry-list")
                    .max_h(px(256.0))
                    .overflow_y_scroll()
                    .pr(px(10.0))
                    .children(
                        rows.iter()
                            .enumerate()
                            .map(|(ix, agent)| self.registry_row(ix, agent, theme, cx)),
                    )
                    .into_any_element(),
            ),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(controls)
            .when_some(error, |el, error| el.child(error_alert(theme, error)))
            .children(list)
            .into_any_element()
    }

    fn registry_row(
        &self,
        ix: usize,
        agent: &AcpRegistryAgent,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let added = provider_instances::uses_registry_agent(&self.instances, &agent.id);
        let preparing = self.registry.preparing.as_deref() == Some(agent.id.as_str());
        let can_add = self.registry.preparing.is_none() && !added;
        let distribution = distribution_label(agent);
        let armed = self.registry.unverified_armed.as_deref() == Some(agent.id.as_str());
        let label = if added {
            "Added"
        } else if armed {
            "Add anyway"
        } else if preparing {
            if distribution == Some("binary") {
                "Downloading"
            } else {
                "Preparing"
            }
        } else {
            "Add"
        };
        let tile = div()
            .flex_none()
            .size(px(24.0))
            .rounded(px(6.0))
            .bg(ink(0.06))
            .flex()
            .items_center()
            .justify_center()
            .child(
                InstanceIcon::registry(agent.icon_path.as_deref())
                    .svg(theme.text_muted)
                    .size(px(16.0)),
            );
        let badge = |text: SharedString| {
            div()
                .flex_none()
                .px(px(6.0))
                .rounded(px(5.0))
                .bg(ink(0.06))
                .text_size(crate::typography::ui_rems(11.0))
                .text_color(theme.text_muted)
                .child(text)
        };
        let name = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .min_w_0()
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(SharedString::from(agent.name.clone())),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(crate::typography::ui_rems(11.0))
                    .text_color(theme.text_faint)
                    .child(SharedString::from(format!("v{}", agent.version))),
            )
            .children(distribution.map(|d| badge(d.into())))
            .when(agent.unverified, |el| {
                el.child(
                    div()
                        .id(("acp-unverified", ix))
                        .child(badge("unverified".into()))
                        .tooltip(widgets::text_tooltip(
                            "The ACP Registry publishes no checksum for this download",
                        )),
                )
            })
            .when(agent.installed && !added, |el| {
                el.child(
                    div()
                        .flex_none()
                        .text_size(crate::typography::ui_rems(11.0))
                        .text_color(theme.text_faint)
                        .child("Installed"),
                )
            });
        let about = about_url(agent).map(|url| {
            let tooltip = format!("About {}", agent.name);
            widgets::ghost_action(theme)
                .id(("acp-about", ix))
                .flex_none()
                .size(px(26.0))
                .min_h(px(26.0))
                .p_0()
                .justify_center()
                .role(gpui::Role::Link)
                .aria_label(SharedString::from(tooltip.clone()))
                .tooltip(widgets::text_tooltip(tooltip))
                .on_click(cx.listener(move |_, _, _, cx| open_login_url(&url, cx)))
                .child(
                    icon(icons::LUCIDE_EXTERNAL_LINK)
                        .size(px(13.0))
                        .text_color(theme.text_muted),
                )
        });
        let add = widgets::action_button(
            theme,
            if preparing {
                widgets::ActionTone::Filled
            } else {
                widgets::ActionTone::Outlined
            },
        )
        .id(("acp-add", ix))
        .flex_none()
        .min_h(px(26.0))
        .py(px(2.0))
        .text_size(crate::typography::ui_rems(12.0))
        .role(gpui::Role::Button)
        .aria_label(SharedString::from(format!("{label} {}", agent.name)))
        .when(!can_add && !preparing, |el| {
            el.opacity(0.5).cursor_default()
        })
        .when(can_add, |el| {
            let agent = agent.clone();
            el.on_click(cx.listener(move |this, _, _, cx| {
                if agent.unverified && !armed {
                    this.registry.unverified_armed = Some(agent.id.clone());
                    cx.notify();
                } else {
                    this.prepare(agent.clone(), agent.unverified, cx);
                }
            }))
        })
        .child(label);
        div()
            .py(px(10.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.0))
            .child(tile)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(name)
                    .when(!agent.description.is_empty(), |el| {
                        el.child(
                            div()
                                .mt(px(2.0))
                                .truncate()
                                .text_size(crate::typography::ui_rems(12.0))
                                .text_color(theme.text_muted)
                                .child(SharedString::from(agent.description.clone())),
                        )
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .children(about)
                    .child(add),
            )
            .into_any_element()
    }

    pub(super) fn render_picked(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let selected = self.selected.as_ref().filter(|_| self.editing.is_none())?;
        let agent = &selected.agent;
        let link = |id: &'static str, text: &'static str, url: Option<String>| {
            url.map(|url| {
                div()
                    .id(id)
                    .cursor_pointer()
                    .role(gpui::Role::Link)
                    .text_color(theme.text_muted)
                    .hover(|s| s.text_color(theme.text))
                    .on_click(cx.listener(move |_, _, _, cx| open_login_url(&url, cx)))
                    .child(text)
            })
        };
        Some(
            div()
                .mb(px(4.0))
                .pb(px(12.0))
                .border_b_1()
                .border_color(theme.border)
                .flex()
                .flex_row()
                .items_start()
                .justify_between()
                .gap(px(12.0))
                .child(
                    div()
                        .min_w_0()
                        .child(
                            div()
                                .truncate()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .child(SharedString::from(agent.name.clone())),
                        )
                        .child(
                            div()
                                .mt(px(2.0))
                                .text_size(crate::typography::ui_rems(12.0))
                                .text_color(theme.text_muted)
                                .child(SharedString::from(format!(
                                    "v{} · {}",
                                    agent.version,
                                    provider_instances::distribution_name(selected.distribution)
                                ))),
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .flex_row()
                        .gap(px(10.0))
                        .text_size(crate::typography::ui_rems(12.0))
                        .children(link("acp-picked-docs", "Docs", agent.website.clone()))
                        .children(link(
                            "acp-picked-source",
                            "Source",
                            agent.repository.clone(),
                        )),
                )
                .into_any_element(),
        )
    }

    pub(super) fn render_acp_edit_config(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut rows = div().flex().flex_col();
        match self.editing.as_ref().map(|instance| &instance.config) {
            Some(InstanceConfig::Acp {
                source:
                    AcpSource::Registry {
                        agent_id,
                        version,
                        distribution,
                    },
                ..
            }) => {
                rows = rows
                    .child(setting_row(
                        theme,
                        "Registry agent",
                        "Agent identifier from the ACP Registry.".into(),
                        None,
                        readonly_value(theme, agent_id.clone()),
                        true,
                    ))
                    .child(setting_row(
                        theme,
                        "Version",
                        "The prepared release this instance runs.".into(),
                        None,
                        readonly_value(theme, version.clone()),
                        true,
                    ))
                    .child(setting_row(
                        theme,
                        "Distribution",
                        "How the agent was installed.".into(),
                        None,
                        readonly_value(
                            theme,
                            provider_instances::distribution_name(*distribution).to_string(),
                        ),
                        true,
                    ));
            }
            _ => {
                rows = rows
                    .child(setting_row(
                        theme,
                        "Executable",
                        "Absolute path or a command on PATH.".into(),
                        None,
                        field(theme, &self.command_input, false),
                        true,
                    ))
                    .child(setting_row(
                        theme,
                        "Arguments",
                        "Passed after the executable.".into(),
                        None,
                        field(theme, &self.acp_args_input, false),
                        true,
                    ));
            }
        }
        rows.child(self.env_row(theme, "Set for this agent only.", cx))
            .into_any_element()
    }

    pub(super) fn render_sign_in_step(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(sign_in) = &self.sign_in else {
            return div().into_any_element();
        };
        let in_app: Vec<AcpAuthMethod> = sign_in
            .methods
            .ready()
            .map(|methods| {
                methods
                    .iter()
                    .filter(|method| method.signs_in_app())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let active = sign_in.active();
        let discovering = sign_in.methods.is_loading();
        let methods_error = sign_in.methods.error().map(str::to_owned);
        let external = !discovering && (methods_error.is_some() || in_app.is_empty());
        let description: SharedString = if active {
            sign_in
                .login
                .as_ref()
                .and_then(|login| login.message.clone())
                .unwrap_or_else(|| "Finish signing in in your browser.".into())
                .into()
        } else if sign_in.signed_in {
            "Signed in.".into()
        } else if discovering {
            "Discovering sign-in methods…".into()
        } else if let Some(error) = &methods_error {
            error.clone().into()
        } else if in_app.is_empty() {
            NO_IN_APP_SIGN_IN.into()
        } else {
            format!("{} advertises browser sign-in.", sign_in.instance.label).into()
        };
        let retry = sign_in.error.is_some();
        let sign_in_button = |id: (&'static str, usize), method_id: Option<String>| {
            Self::small_button(theme, widgets::ActionTone::Outlined, method_id.is_some())
                .id(id)
                .when_some(method_id, |el, method_id| {
                    el.on_click(
                        cx.listener(move |this, _, _, cx| this.start_login(method_id.clone(), cx)),
                    )
                })
                .child(if retry { "Retry sign-in" } else { "Sign in" })
        };
        let control: Option<AnyElement> = if discovering {
            Some(sign_in_button(("acp-sign-in", 0), None).into_any_element())
        } else if active {
            Some(
                Self::small_button(theme, widgets::ActionTone::Quiet, true)
                    .id("acp-cancel-sign-in")
                    .aria_label("Cancel sign-in")
                    .on_click(cx.listener(|this, _, _, cx| this.cancel_login(cx)))
                    .child("Cancel")
                    .into_any_element(),
            )
        } else if sign_in.signed_in {
            Some(
                div()
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.success)
                    .child("Ready")
                    .into_any_element(),
            )
        } else if external {
            sign_in.docs.clone().map(|url| {
                Self::small_button(theme, widgets::ActionTone::Outlined, true)
                    .id("acp-open-docs")
                    .role(gpui::Role::Link)
                    .on_click(cx.listener(move |_, _, _, cx| open_login_url(&url, cx)))
                    .child("Open docs")
                    .into_any_element()
            })
        } else if let [method] = in_app.as_slice() {
            Some(sign_in_button(("acp-sign-in", 0), Some(method.id.clone())).into_any_element())
        } else {
            None
        };
        let method_rows = (in_app.len() > 1 && !active && !sign_in.signed_in).then(|| {
            div()
                .mt(px(12.0))
                .pt(px(4.0))
                .border_t_1()
                .border_color(theme.border)
                .flex()
                .flex_col()
                .children(in_app.iter().enumerate().map(|(ix, method)| {
                    div()
                        .py(px(8.0))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(12.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .child(SharedString::from(method.name.clone())),
                                )
                                .children(method.description.clone().map(|text| {
                                    div()
                                        .mt(px(2.0))
                                        .truncate()
                                        .text_size(crate::typography::ui_rems(12.0))
                                        .text_color(theme.text_muted)
                                        .child(SharedString::from(text))
                                })),
                        )
                        .child(sign_in_button(
                            ("acp-sign-in-method", ix),
                            Some(method.id.clone()),
                        ))
                }))
        });
        let avatar = div()
            .flex_none()
            .size(px(26.0))
            .rounded(px(7.0))
            .bg(ink(0.06))
            .flex()
            .items_center()
            .justify_center()
            .child(
                InstanceIcon::of(&sign_in.instance)
                    .svg(theme.text_muted)
                    .size(px(16.0)),
            );
        let card = div()
            .p(px(14.0))
            .rounded(px(10.0))
            .bg(ink(0.03))
            .border_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(12.0))
                    .child(avatar)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(div().font_weight(gpui::FontWeight::MEDIUM).child("Account"))
                            .child(
                                div()
                                    .mt(px(2.0))
                                    .text_size(crate::typography::ui_rems(12.0))
                                    .text_color(theme.text_muted)
                                    .child(description),
                            ),
                    )
                    .children(control),
            )
            .children(method_rows)
            .when_some(sign_in.error.clone(), |el, error| {
                el.child(
                    div()
                        .mt(px(8.0))
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.danger)
                        .child(SharedString::from(error)),
                )
            });
        div()
            .flex()
            .flex_col()
            .child(card)
            .when(!sign_in.signed_in, |el| {
                el.child(
                    div()
                        .mt(px(10.0))
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.text_faint)
                        .child("You can sign in later from Settings › Providers."),
                )
            })
            .into_any_element()
    }
}

fn error_alert(theme: &Theme, message: String) -> AnyElement {
    div()
        .px(px(12.0))
        .py(px(8.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(theme.danger.opacity(0.3))
        .bg(theme.danger.opacity(0.08))
        .text_size(crate::typography::ui_rems(12.0))
        .text_color(theme.danger)
        .child(SharedString::from(message))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;

    #[gpui::test]
    fn registry_and_local_picks_upsert_their_own_acp_source(cx: &mut gpui::TestAppContext) {
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
        cx.run_until_parked();
        window
            .update(cx, |dialog, _, cx| {
                let listed = AcpRegistryAgent {
                    id: "kimchi".into(),
                    name: "kimchi".into(),
                    version: "1.15.0".into(),
                    description: String::new(),
                    website: None,
                    repository: None,
                    distributions: Vec::new(),
                    icon_url: None,
                    icon_path: Some("/cache/kimchi.svg".into()),
                    installed: false,
                    unverified: false,
                };
                let prepared = AcpPreparedAgent {
                    agent_id: "kimchi".into(),
                    version: "1.16.0".into(),
                    distribution: AcpDistribution::Binary,
                    icon_path: None,
                    display_name: "Kimchi".into(),
                };
                dialog.select_prepared(listed, prepared, cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |dialog, _, cx| {
                assert_eq!(dialog.instance_id(), "acp_kimchi");
                assert_eq!(
                    dialog.config(cx),
                    Some(InstanceConfig::Acp {
                        source: AcpSource::Registry {
                            agent_id: "kimchi".into(),
                            version: "1.16.0".into(),
                            distribution: AcpDistribution::Binary,
                        },
                        env: BTreeMap::new(),
                        icon_path: Some("/cache/kimchi.svg".into()),
                    })
                );

                dialog.enter_local(cx);
                dialog
                    .command_input
                    .update(cx, |input, cx| input.set_text("~/bin/agent", cx));
                dialog
                    .acp_args_input
                    .update(cx, |input, cx| input.set_text(" --acp  --verbose ", cx));
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |dialog, _, cx| {
                assert_eq!(dialog.instance_id(), "acp_local_acp");
                assert_eq!(
                    dialog.config(cx),
                    Some(InstanceConfig::Acp {
                        source: AcpSource::Local {
                            command: "~/bin/agent".into(),
                            args: vec!["--acp".into(), "--verbose".into()],
                        },
                        env: BTreeMap::new(),
                        icon_path: None,
                    })
                );
            })
            .unwrap();
    }
}
