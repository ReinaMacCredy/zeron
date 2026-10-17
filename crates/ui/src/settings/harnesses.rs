//! Settings → Providers: install and enable harnesses, one card row per agent.
//!
//! The state is PER-DEVICE and lives on the engine (`harness-prefs.json` in
//! its data dir): CLI installs are per-device, so enablement is too. The
//! page-header device switcher (the Accounts pattern) retargets both the
//! `ListHarnesses` probe and the `SetHarnessEnabled` writes at any registered
//! device over the relay-forwarded RPCs.
//!
//! Enablement follows DETECTION: every harness whose CLI probe passes is on
//! unless the user switched it off, so installing an agent is all it takes
//! for it to appear here and in the composer. A harness whose CLI is missing
//! on the target device renders dimmed with an install hint and is never
//! enabled (enabling an agent that can't run would only manufacture
//! NotInstalled errors at send time); an ENABLED agent can always be turned
//! OFF except the last one standing — the composer needs something to run.
//! Catalogs from engines predating the detection model can still stamp
//! enabled-but-uninstalled rows; the hint covers that too. The engine
//! enforces the same gates where the state lives, so a raced or stale toggle
//! self-corrects from the RPC reply.

use std::collections::HashMap;

use gpui::{
    AnyElement, Context, Entity, IntoElement, Render, SharedString, Subscription, Task, Window,
    div, prelude::*, px,
};

use zeron_engine::registry::{HarnessDescriptor, descriptor_enabled};

use zeron_proto::{
    HarnessId, HarnessUpdatePhase, HarnessUpdatePolicy, HarnessUpdateStatus, InstanceConfig,
    ProviderInstance,
};

use zeron_rpc::methods;

use crate::motion;
use crate::pickers::visible_harnesses;
use crate::popover::{self, Loadable};
use crate::provider_instances::{self, ProviderInstanceWrite};
use crate::settings::accounts::{self, AccountsPage};
use crate::settings::add_provider::{self, AddProviderDialog, AddProviderEvent};
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;

#[path = "completion.rs"]
mod completion;

/// Left inset of an expanded provider's details: the header trigger's
/// padding, brand tile and gap, so the details start on the title's edge.
const DETAILS_INSET: f32 = 4.0 + 36.0 + 12.0;

/// Update policies in menu order: the short label, and the note shown under
/// the row title for the chosen one.
const UPDATE_POLICIES: [(HarnessUpdatePolicy, &str, &str); 3] = [
    (
        HarnessUpdatePolicy::Notify,
        "Notify",
        "Install only when you choose Update.",
    ),
    (
        HarnessUpdatePolicy::AutoWhenIdle,
        "Auto when idle",
        "Install automatically after active runs finish.",
    ),
    (
        HarnessUpdatePolicy::Off,
        "Off",
        "Don't check for new versions.",
    ),
];

fn offers_install(harness: HarnessId, installed: bool, can_install: bool) -> bool {
    harness != HarnessId::Mock && !installed && can_install
}

fn install_hint(harness: HarnessId, enabled: bool, can_install: bool) -> String {
    if harness == HarnessId::Antigravity {
        return if can_install {
            "Install Antigravity to enable"
        } else {
            "Set ANTIGRAVITY_ACP_EXECUTABLE to enable Antigravity"
        }
        .into();
    }
    let hint = if enabled {
        format!(
            "{} CLI not installed — turn it off or install it",
            cli_name(harness)
        )
    } else {
        format!("Install the {} CLI to enable", cli_name(harness))
    };
    if !can_install && let Some(command) = zeron_harness::install::manual_command(harness) {
        format!("{hint}. Install with `{command}`")
    } else {
        hint
    }
}

fn install_label(name: &str) -> String {
    format!("Installing {name}…")
}

fn install_params(harness: HarnessId, target: &Option<String>) -> serde_json::Value {
    serde_json::json!({"harness": harness, "targetDeviceId": target})
}

/// The CLI named in the not-installed hint.
pub fn cli_name(harness: HarnessId) -> &'static str {
    match harness {
        HarnessId::ClaudeCode => "claude",
        HarnessId::Codex => "codex",
        HarnessId::Cursor => "cursor-agent",
        HarnessId::Devin => "devin",
        HarnessId::Grok => "grok",
        HarnessId::Hermes => "hermes",
        HarnessId::Pi => "pi",
        HarnessId::Opencode => "opencode",
        HarnessId::Antigravity => "Antigravity",
        HarnessId::Acp => "acp",
        HarnessId::Mock => "mock",
    }
}

/// A provider's version line: the installed version, plus only what the
/// Update button doesn't already say (progress, a failure), all in one muted
/// tone. Up-to-date providers, checks in flight and providers whose updates
/// aren't monitored show just the version, so rows don't flash through
/// states while checks run.
fn harness_update_label(status: &HarnessUpdateStatus, theme: &Theme) -> (SharedString, gpui::Hsla) {
    let installed = status
        .installed_version
        .as_deref()
        .map(|version| format!("v{version}"));
    let with = |detail: &str| match &installed {
        Some(version) => format!("{version} · {detail}"),
        None => detail.to_owned(),
    };
    let version_only = || installed.clone().unwrap_or_default();
    let label = match status.phase {
        HarnessUpdatePhase::Dormant
        | HarnessUpdatePhase::Checking
        | HarnessUpdatePhase::Current => version_only(),
        // The row's Update button already says it; one Zeron can only
        // describe (a command to run yourself) is just the version too.
        HarnessUpdatePhase::Available | HarnessUpdatePhase::ManualActionRequired
            if status.can_apply =>
        {
            version_only()
        }
        HarnessUpdatePhase::Available => match status.latest_version.as_deref() {
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
        HarnessUpdatePhase::Failed => with("Couldn't check for updates"),
    };
    let color = match status.phase {
        HarnessUpdatePhase::Failed => theme.danger_muted,
        _ => theme.text_muted.opacity(0.75),
    };
    (label.into(), color)
}

pub struct HarnessesPage {
    state: Entity<AppState>,
    scroll: widgets::PageScroll,
    harnesses: Loadable<Vec<HarnessDescriptor>>,
    updates: Loadable<Vec<HarnessUpdateStatus>>,
    /// Which device's harnesses are shown/edited; `None` = this device (no
    /// passthrough). Retargeted by the page-header device switcher.
    target_device: Option<String>,
    device_select: widgets::SelectState,
    policy_selects: std::collections::HashMap<HarnessId, widgets::SelectState>,
    /// Last refused/failed toggle (engine guards), shown in the error strip.
    error: Option<String>,
    load_task: Option<Task<()>>,
    toggle_task: Option<Task<()>>,
    /// A toggle the engine hasn't answered yet: the switch shows it at once.
    pending_toggle: Option<(HarnessId, bool)>,
    installing: Option<HarnessId>,
    install_task: Option<Task<()>>,

    expanded_harness: Option<HarnessId>,
    /// The expanded provider's Accounts section — one page, retargeted as
    /// providers expand, so every provider shares the same sign-in flow.
    accounts_page: Option<Entity<AccountsPage>>,
    update_task: Option<Task<()>>,
    update_action_task: Option<Task<()>>,
    /// "Check now" is running: the button says so, since rows no longer
    /// show "Checking…" while a check is in flight.
    checking_updates: bool,
    instances: Loadable<Vec<ProviderInstance>>,
    instances_task: Option<Task<()>>,
    instance_write_task: Option<Task<()>>,
    instance_accounts: HashMap<String, Entity<AccountsPage>>,
    confirm_delete: Option<String>,
    wizard: Option<(Entity<AddProviderDialog>, Subscription)>,
    scroll_end_knob: bool,
}

impl HarnessesPage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let mut page = Self {
            state,
            scroll: widgets::PageScroll::default(),
            harnesses: Loadable::Idle,
            updates: Loadable::Idle,
            target_device: None,
            device_select: widgets::SelectState::default(),
            policy_selects: Default::default(),
            error: None,
            load_task: None,
            toggle_task: None,
            pending_toggle: None,
            installing: None,
            install_task: None,

            expanded_harness: None,
            accounts_page: None,
            update_task: None,
            update_action_task: None,
            checking_updates: false,
            instances: Loadable::Idle,
            instances_task: None,
            instance_write_task: None,
            instance_accounts: HashMap::new(),
            confirm_delete: None,
            wizard: None,
            scroll_end_knob: std::env::var("ZERON_PROVIDERS_SCROLL").as_deref() == Ok("end"),
        };
        page.load(cx);
        if std::env::var("ZERON_OPEN_DIALOG").as_deref() == Ok("add-provider") {
            page.open_wizard(None, cx);
            let driver = std::env::var("ZERON_ADD_PROVIDER_DRIVER")
                .ok()
                .and_then(|driver| serde_json::from_value(serde_json::Value::String(driver)).ok());
            let step = std::env::var("ZERON_ADD_PROVIDER_STEP")
                .ok()
                .and_then(|step| add_provider::Step::from_knob(&step));
            let knob = std::env::var("ZERON_ADD_PROVIDER_STEP").unwrap_or_default();
            let query = std::env::var("ZERON_ADD_PROVIDER_QUERY").ok();
            let acp_instance = std::env::var("ZERON_ADD_PROVIDER_ACP_INSTANCE").ok();
            if let Some((wizard, _)) = &page.wizard {
                wizard.update(cx, |wizard, cx| {
                    if let Some(driver) = driver {
                        wizard.select_driver(driver, cx);
                    }
                    if let Some((step, taken)) = step {
                        wizard.open_at(step, taken, cx);
                    }
                    wizard.apply_acp_knob(&knob, query, acp_instance, cx);
                });
            }
        }
        page
    }

    fn device_name(&self, cx: &Context<Self>) -> SharedString {
        let state = self.state.read(cx);
        match &self.target_device {
            Some(id) if state.local_device_id.as_deref() != Some(id.as_str()) => state
                .devices
                .iter()
                .find(|device| &device.id == id)
                .map_or_else(|| "this device".into(), |device| device.name.clone().into()),
            _ => "This Mac".into(),
        }
    }

    fn open_wizard(&mut self, editing: Option<ProviderInstance>, cx: &mut Context<Self>) {
        let state = self.state.clone();
        let target = self.target_device.clone();
        let device = self.device_name(cx);
        let harnesses = self.harnesses.ready().cloned().unwrap_or_default();
        let instances = self.instances.ready().cloned().unwrap_or_default();
        let wizard = cx.new(|cx| {
            AddProviderDialog::new(state, target, device, harnesses, instances, editing, cx)
        });
        let events = cx.subscribe(&wizard, |page: &mut Self, _, event, cx| match event {
            AddProviderEvent::Close => {
                page.wizard = None;
                cx.notify();
            }
            AddProviderEvent::Saved {
                instances,
                instance,
                sign_in,
            } => {
                page.wizard = None;
                page.instance_accounts.remove(&instance.id);
                page.set_instances(Ok(instances.clone()), cx);
                provider_instances::bump_provider_instances(cx);
                if *sign_in && let Some(accounts) = page.instance_accounts.get(&instance.id) {
                    accounts.update(cx, |accounts, cx| accounts.sign_in(cx));
                }
            }
            AddProviderEvent::Created { instances } => {
                page.set_instances(Ok(instances.clone()), cx);
                provider_instances::bump_provider_instances(cx);
            }
        });
        self.confirm_delete = None;
        self.wizard = Some((wizard, events));
        cx.notify();
    }

    fn set_instances(
        &mut self,
        result: Result<Vec<ProviderInstance>, String>,
        cx: &mut Context<Self>,
    ) {
        self.instances = match result {
            Ok(instances) => Loadable::Ready(instances),
            Err(error) => Loadable::Error(error),
        };
        let own_login: Vec<ProviderInstance> = self
            .instances
            .ready()
            .into_iter()
            .flatten()
            .filter(|instance| provider_instances::has_own_login(instance))
            .cloned()
            .collect();
        self.instance_accounts
            .retain(|id, _| own_login.iter().any(|instance| &instance.id == id));
        for instance in own_login {
            if self.instance_accounts.contains_key(&instance.id) {
                continue;
            }
            let state = self.state.clone();
            let target = self.target_device.clone();
            let accounts = cx.new(|cx| {
                AccountsPage::new_instance(
                    state,
                    target,
                    instance.driver,
                    instance.id.clone(),
                    instance.label.clone(),
                    cx,
                )
            });
            cx.observe(&accounts, |_, _, cx| cx.notify()).detach();
            self.instance_accounts.insert(instance.id, accounts);
        }
        if let (Some((wizard, _)), Some(instances)) = (&self.wizard, self.instances.ready()) {
            let instances = instances.clone();
            wizard.update(cx, |wizard, cx| wizard.set_instances(instances, cx));
        }
        cx.notify();
    }

    fn delete_instance(&mut self, id: String, cx: &mut Context<Self>) {
        if self.confirm_delete.as_deref() != Some(id.as_str()) {
            self.confirm_delete = Some(id);
            cx.notify();
            return;
        }
        self.confirm_delete = None;
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let target = self.target_device.clone().map(serde_json::Value::String);
        self.instance_write_task = Some(cx.spawn(async move |this, cx| {
            let result = provider_instances::write(
                &engine,
                ProviderInstanceWrite::Delete { instance_id: id },
                target,
            )
            .await;
            this.update(cx, |page, cx| match result {
                Ok(instances) => {
                    page.set_instances(Ok(instances), cx);
                    provider_instances::bump_provider_instances(cx);
                }
                Err(error) => {
                    page.error = Some(error);
                    cx.notify();
                }
            })
            .ok();
        }));
        cx.notify();
    }

    fn sign_in_acp(&mut self, instance_id: String, cx: &mut Context<Self>) {
        self.open_wizard(None, cx);
        if let Some((wizard, _)) = &self.wizard {
            wizard.update(cx, |wizard, cx| wizard.sign_in_to(instance_id, cx));
        }
    }

    fn instance_row(
        &self,
        instance: &ProviderInstance,
        nested: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mark = provider_instances::InstanceIcon::of(instance);
        let id = instance.id.clone();
        let confirming = self.confirm_delete.as_deref() == Some(id.as_str());
        let accounts = self.instance_accounts.get(&id).cloned();
        let busy = accounts
            .as_ref()
            .is_some_and(|accounts| accounts.read(cx).signing_in());
        let native = matches!(instance.config, InstanceConfig::Native { .. });
        let editable = native || instance.driver == HarnessId::Acp;
        let status = match accounts {
            Some(accounts) => Some(
                div()
                    .min_w_0()
                    .max_w(px(240.0))
                    .child(accounts)
                    .into_any_element(),
            ),
            None if native && accounts::signs_in(instance.driver) => Some(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(format!(
                        "Uses the default {} login",
                        add_provider::driver_name(instance.driver)
                    )))
                    .into_any_element(),
            ),
            None if matches!(instance.config, InstanceConfig::Acp { .. }) => {
                let instance_id = id.clone();
                Some(
                    widgets::text_action(theme, widgets::ActionTone::Outlined, "Sign in")
                        .id(SharedString::from(format!(
                            "provider-instance-sign-in-{id}"
                        )))
                        .flex_none()
                        .tab_index(0)
                        .role(gpui::Role::Button)
                        .focus_visible(|s| s.border_2().border_color(theme.accent))
                        .on_click(cx.listener(move |page, _, _, cx| {
                            page.sign_in_acp(instance_id.clone(), cx)
                        }))
                        .into_any_element(),
                )
            }
            None => None,
        };
        let accent = provider_instances::instance_accent(instance);
        let editing = instance.clone();
        let (tile_size, tile_radius, mark_size) = if nested {
            (28.0, 8.0, 14.0)
        } else {
            (36.0, 10.0, 16.0)
        };
        let tile = div()
            .flex_none()
            .size(px(tile_size))
            .rounded(px(tile_radius))
            .bg(theme.wash(0.06))
            .flex()
            .items_center()
            .justify_center()
            .child(mark.svg(theme.text_muted).size(px(mark_size)));
        widgets::card_row(theme, false)
            .id(SharedString::from(format!("provider-instance-{id}")))
            .map(|row| {
                if nested {
                    row.ml(px(16.0 + DETAILS_INSET))
                } else {
                    row.pl(px(4.0))
                }
            })
            .min_h(px(52.0))
            .flex_nowrap()
            .gap(px(12.0))
            .child(tile)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.0))
                            .child(widgets::row_title(theme, instance.label.clone()).truncate())
                            .when_some(accent, |el, accent| {
                                el.child(div().flex_none().size(px(8.0)).rounded_full().bg(accent))
                            }),
                    )
                    .child(widgets::meta_line(
                        theme,
                        std::iter::once(
                            div()
                                .font_family(theme.font_mono.clone())
                                .child(SharedString::from(id.clone()))
                                .into_any_element(),
                        )
                        .chain(
                            provider_instances::acp_source_summary(instance).map(|summary| {
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .child(SharedString::from(summary))
                                    .into_any_element()
                            }),
                        )
                        .collect(),
                    )),
            )
            .children(status)
            .child(
                widgets::ghost_action(theme)
                    .id(SharedString::from(format!("provider-instance-edit-{id}")))
                    .flex_none()
                    .px(px(8.0))
                    .role(gpui::Role::Button)
                    .aria_label("Edit")
                    .tooltip(widgets::text_tooltip("Edit"))
                    .when(busy || !editable, |el| el.opacity(0.4))
                    .when(!busy && editable, |el| {
                        el.on_click(cx.listener(move |page, _, _, cx| {
                            page.open_wizard(Some(editing.clone()), cx)
                        }))
                    })
                    .child(
                        crate::icons::icon(crate::icons::PEN)
                            .size(px(14.0))
                            .text_color(theme.text_muted),
                    ),
            )
            .child(
                widgets::ghost_action(theme)
                    .id(SharedString::from(format!("provider-instance-delete-{id}")))
                    .flex_none()
                    .px(px(8.0))
                    .role(gpui::Role::Button)
                    .aria_label(if confirming {
                        "Confirm delete"
                    } else {
                        "Delete"
                    })
                    .when(busy, |el| el.opacity(0.4))
                    .when(!busy, |el| {
                        el.on_click(
                            cx.listener(move |page, _, _, cx| page.delete_instance(id.clone(), cx)),
                        )
                    })
                    .map(|el| {
                        if confirming {
                            el.text_color(theme.danger).child("Delete?")
                        } else {
                            el.tooltip(widgets::text_tooltip("Delete")).child(
                                crate::icons::icon(crate::icons::TRASH_BIN_MINIMALISTIC)
                                    .size(px(14.0))
                                    .text_color(theme.text_muted),
                            )
                        }
                    }),
            )
            .into_any_element()
    }

    fn toggle_agent_details(&mut self, harness: HarnessId, cx: &mut Context<Self>) {
        if !self.harnesses.ready().is_some_and(|items| {
            items
                .iter()
                .any(|item| item.id == harness && descriptor_enabled(item))
        }) {
            return;
        }
        if self.expanded_harness == Some(harness) {
            self.expanded_harness = None;
        } else {
            self.expanded_harness = Some(harness);
            if accounts::signs_in(harness) {
                if let Some(accounts) = &self.accounts_page {
                    accounts.update(cx, |page, cx| page.set_embedded_harness(harness, cx));
                } else {
                    let state = self.state.clone();
                    let target = self.target_device.clone();
                    self.accounts_page =
                        Some(cx.new(|cx| AccountsPage::new_embedded(state, target, harness, cx)));
                }
            }
        }
        cx.notify();
    }

    fn render_agent_details(
        &self,
        harness: HarnessId,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let accounts = accounts::signs_in(harness)
            .then(|| self.accounts_page.clone())
            .flatten()
            .map(|page| page.into_any_element());
        // No box of its own: the details continue the provider row on the
        // block's fill, indented to the row's title so they read as its
        // children, with switches and actions on the row's control edge.
        let content = div()
            .mx(px(16.0))
            .pl(px(DETAILS_INSET))
            .pb(px(16.0))
            .flex()
            .flex_col()
            .gap(px(20.0))
            .child(self.render_completion_for(harness, theme, cx))
            .children(self.render_updates_for(harness, theme, cx))
            .when_some(accounts, |details, accounts| details.child(accounts));
        if motion::reduced_motion(cx) {
            content.into_any_element()
        } else {
            motion::menu_in(format!("agent-details-{harness:?}"), content).into_any_element()
        }
    }

    /// The expanded provider's Updates section: the update policy, short
    /// labels in the menu and the chosen one explained under the title.
    fn render_updates_for(
        &self,
        harness: HarnessId,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let Loadable::Ready(statuses) = &self.updates else {
            return None;
        };
        let status = statuses.iter().find(|status| status.harness == harness)?;
        let selected = UPDATE_POLICIES
            .iter()
            .position(|(policy, ..)| *policy == status.policy)
            .unwrap_or(0);
        let closed = widgets::SelectState::default();
        let policy_select = widgets::select(
            format!("harness-update-policy-{harness:?}"),
            "Update policy",
            theme,
            move |page: &mut Self| page.policy_selects.entry(harness).or_default(),
        )
        .options(
            UPDATE_POLICIES
                .iter()
                .map(|(_, label, _)| widgets::SelectOption::new(*label)),
            selected,
        )
        // Fits the longest label, so switching never resizes the trigger.
        .width(136.0)
        .on_select(move |page, selected, _, cx| {
            if let Some((policy, ..)) = UPDATE_POLICIES.get(selected) {
                page.set_update_policy(harness, *policy, cx);
            }
        })
        .render(self.policy_selects.get(&harness).unwrap_or(&closed), cx);
        let row = div()
            .min_h(px(52.0))
            .py(px(10.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(16.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(widgets::row_title(theme, "Update policy"))
                    .child(widgets::meta_line(
                        theme,
                        vec![div().child(UPDATE_POLICIES[selected].2).into_any_element()],
                    )),
            )
            .child(policy_select);
        Some(
            div()
                .flex()
                .flex_col()
                .child(widgets::details_label(theme, "Updates"))
                .child(row)
                .into_any_element(),
        )
    }

    /// Params with the `targetDeviceId` passthrough merged in.
    fn with_target(&self, mut value: serde_json::Value) -> serde_json::Value {
        if let (Some(target), Some(object)) = (&self.target_device, value.as_object_mut()) {
            object.insert("targetDeviceId".into(), serde_json::json!(target));
        }
        value
    }

    /// Escape that reached Settings unclaimed goes to the expanded provider's
    /// accounts (an open login) first. Returns whether it was consumed.
    pub(crate) fn dismiss_on_escape(&mut self, cx: &mut Context<Self>) -> bool {
        if self.wizard.take().is_some() {
            cx.notify();
            return true;
        }
        let accounts: Vec<_> = self.instance_accounts.values().cloned().collect();
        if accounts
            .into_iter()
            .any(|accounts| accounts.update(cx, |page, cx| page.dismiss_on_escape(cx)))
        {
            return true;
        }
        self.expanded_harness.is_some()
            && self
                .accounts_page
                .as_ref()
                .is_some_and(|accounts| accounts.update(cx, |page, cx| page.dismiss_on_escape(cx)))
    }

    fn supports_updates(&self, cx: &Context<Self>) -> bool {
        let state = self.state.read(cx);
        let Some(engine) = state.engine() else {
            return false;
        };
        self.target_device.as_deref().map_or_else(
            || {
                engine
                    .engine_info()
                    .supports(zeron_proto::capabilities::HARNESS_UPDATES_V1)
            },
            |device| state.device_supports(device, zeron_proto::capabilities::HARNESS_UPDATES_V1),
        )
    }

    fn can_control_updates(&self, cx: &Context<Self>) -> bool {
        self.supports_updates(cx)
            && matches!(self.updates, Loadable::Ready(_))
            && self.target_device.as_ref().is_none_or(|device| {
                self.state
                    .read(cx)
                    .device_online(device, chrono::Utc::now())
            })
    }

    /// Retarget the page at another device: a different device is a different
    /// install/enablement world, so drop the rows and reload through it.

    pub(crate) fn set_target_device(&mut self, target: Option<String>, cx: &mut Context<Self>) {
        widgets::close_select(self, |page: &mut Self| &mut page.device_select, cx);

        if self.target_device == target {
            cx.notify();
            return;
        }
        self.installing = None;
        self.install_task = None;
        self.policy_selects.clear();
        self.target_device = target;
        if let Some(accounts) = &self.accounts_page {
            accounts.update(cx, |page, cx| {
                page.set_target_device(self.target_device.clone(), cx)
            });
        }
        self.error = None;
        self.wizard = None;
        self.confirm_delete = None;
        self.instance_accounts.clear();
        self.instances = Loadable::Idle;
        self.harnesses = Loadable::Idle;
        self.updates = Loadable::Idle;
        self.update_task = None;
        self.update_action_task = None;
        self.load(cx);
        cx.notify();
    }

    /// `ListHarnesses` against the target device (installed probe + enabled
    /// set both come from where the CLIs actually live).
    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let params = self.with_target(serde_json::json!({}));
        let update_params = params.clone();
        let supports_updates = self.supports_updates(cx);

        self.harnesses = Loadable::Loading;
        self.updates = if supports_updates {
            Loadable::Loading
        } else {
            Loadable::Ready(Vec::new())
        };
        self.update_task = supports_updates.then(|| {
            let update_engine = engine.clone();
            cx.spawn(async move |this, cx| {
                let mut retry = 1;
                loop {
                    let subscribed = update_engine
                        .client()
                        .subscribe_checked(methods::WATCH_HARNESS_UPDATES, update_params.clone())
                        .await;
                    match subscribed {
                        Ok(mut stream) => {
                            while let Some(value) = stream.recv().await {
                                retry = 1;
                                let parsed =
                                    serde_json::from_value::<Vec<HarnessUpdateStatus>>(value)
                                        .map_err(|error| error.to_string());
                                if this
                                    .update(cx, |page, cx| {
                                        page.updates = match parsed {
                                            Ok(statuses) => Loadable::Ready(statuses),
                                            Err(error) => Loadable::Error(error),
                                        };
                                        cx.notify();
                                    })
                                    .is_err()
                                {
                                    return;
                                }
                            }
                        }
                        Err(error) => {
                            this.update(cx, |page, cx| {
                                page.updates = Loadable::Error(error.to_string());
                                cx.notify();
                            })
                            .ok();
                        }
                    }
                    if this
                        .update(cx, |page, cx| {
                            if !matches!(page.updates, Loadable::Error(_)) {
                                page.updates = Loadable::Error(
                                    "Connection closed. Reconnecting to device…".into(),
                                );
                            }
                            cx.notify();
                        })
                        .is_err()
                    {
                        return;
                    }
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(retry))
                        .await;
                    retry = (retry * 2).min(15);
                }
            })
        });
        let instance_engine = engine.clone();
        let target = self.target_device.clone().map(serde_json::Value::String);
        self.instances_task = Some(cx.spawn(async move |this, cx| {
            let result = provider_instances::list(&instance_engine, target).await;
            this.update(cx, |page, cx| page.set_instances(result, cx))
                .ok();
        }));
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let result = engine.client().call(methods::LIST_HARNESSES, params).await;
            this.update(cx, |page, cx| {
                page.harnesses = match result {
                    Ok(value) => match serde_json::from_value::<Vec<HarnessDescriptor>>(value) {
                        Ok(list) => Loadable::Ready(list),
                        Err(err) => Loadable::Error(err.to_string()),
                    },
                    Err(err) => Loadable::Error(err.to_string()),
                };
                if let (Some((wizard, _)), Some(list)) = (&page.wizard, page.harnesses.ready()) {
                    let list = list.clone();
                    wizard.update(cx, |wizard, cx| wizard.set_harnesses(list, cx));
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Flip one harness on the target device. The reply carries the device's
    /// fresh catalog, so the rows repaint from the authoritative state in one
    /// round trip; refusals (engine guards) land in the error strip.
    fn toggle(&mut self, harness: HarnessId, enabled: bool, cx: &mut Context<Self>) {
        self.set_enabled(harness, enabled, cx);
    }

    fn cancel_install(&mut self, cx: &mut Context<Self>) {
        let Some(harness) = self.installing else {
            return;
        };
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let target = self.target_device.clone();
        let params = install_params(harness, &target);
        cx.spawn(async move |this, cx| {
            if let Err(error) = engine.client().call(methods::CANCEL_INSTALL, params).await {
                this.update(cx, |page, cx| {
                    if page.target_device == target && page.installing == Some(harness) {
                        page.error = Some(format!("Cancellation failed — {error}"));
                        cx.notify();
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    fn install(&mut self, harness: HarnessId, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        if self.installing.is_some() {
            return;
        }
        let params = install_params(harness, &self.target_device);
        let target = self.target_device.clone();
        self.installing = Some(harness);
        self.error = None;
        self.install_task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::INSTALL_HARNESS, params)
                .await
                .map_err(|error| error.to_string())
                .and_then(|value| {
                    serde_json::from_value::<Vec<HarnessDescriptor>>(value)
                        .map_err(|error| error.to_string())
                });
            this.update(cx, |page, cx| {
                if page.target_device != target {
                    return;
                }
                page.installing = None;
                match result {
                    Ok(list) => {
                        page.harnesses = Loadable::Ready(list);
                        crate::pickers::bump_harness_catalog(cx);
                    }
                    Err(error) => page.error = Some(format!("Installation failed — {error}")),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn set_enabled(&mut self, harness: HarnessId, enabled: bool, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let params = self.with_target(serde_json::json!({
            "harness": harness,
            "enabled": enabled,
        }));
        self.error = None;
        self.pending_toggle = Some((harness, enabled));
        cx.notify();
        self.toggle_task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::SET_HARNESS_ENABLED, params)
                .await;
            this.update(cx, |page, cx| {
                if page.pending_toggle == Some((harness, enabled)) {
                    page.pending_toggle = None;
                }
                match result {
                    Ok(value) => {
                        if let Ok(list) = serde_json::from_value::<Vec<HarnessDescriptor>>(value) {
                            page.harnesses = Loadable::Ready(list);
                        }
                        if !enabled && page.expanded_harness == Some(harness) {
                            page.expanded_harness = None;
                        }
                        // The composer caches its catalog per space — poke
                        // every Pickers to re-fetch, or the rail keeps the
                        // old set until restart.
                        crate::pickers::bump_harness_catalog(cx);
                    }
                    Err(err) => page.error = Some(err.to_string()),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn check_updates(&mut self, cx: &mut Context<Self>) {
        if !self.can_control_updates(cx) {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let params = self.with_target(serde_json::json!({}));
        self.error = None;
        self.checking_updates = true;
        cx.notify();
        self.update_action_task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::CHECK_HARNESS_UPDATES, params)
                .await;
            this.update(cx, |page, cx| {
                page.checking_updates = false;
                match result {
                    Ok(value) => match serde_json::from_value::<Vec<HarnessUpdateStatus>>(value) {
                        Ok(statuses) => page.updates = Loadable::Ready(statuses),
                        Err(error) => page.error = Some(error.to_string()),
                    },
                    Err(error) => page.error = Some(error.to_string()),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn apply_harness_update(&mut self, harness: HarnessId, cx: &mut Context<Self>) {
        if !self.can_control_updates(cx) {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let params = self.with_target(serde_json::json!({ "harness": harness }));
        self.error = None;
        self.update_action_task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::APPLY_HARNESS_UPDATE, params)
                .await;
            this.update(cx, |page, cx| {
                if let Err(error) = result {
                    page.error = Some(error.to_string());
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn cancel_harness_update(&mut self, harness: HarnessId, cx: &mut Context<Self>) {
        if !self.can_control_updates(cx) {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let params = self.with_target(serde_json::json!({ "harness": harness }));
        self.update_action_task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::CANCEL_HARNESS_UPDATE, params)
                .await;
            this.update(cx, |page, cx| {
                if let Err(error) = result {
                    page.error = Some(error.to_string());
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn render_limit_resume_card(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let state = self.state.read(cx);
        let device = self
            .target_device
            .clone()
            .or_else(|| state.local_device_id.clone())?;
        let enabled = state.limit_resumes.get(&device)?.auto_resume;
        Some(
            widgets::section_card(theme)
                .child(
                    widgets::card_row(theme, true)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(widgets::row_title(theme, "Auto-resume after usage limits"))
                                .child(widgets::meta_line(
                                    theme,
                                    vec![
                                        div()
                                            .child(
                                                "Continue a thread stopped by a usage limit once the limit resets.",
                                            )
                                            .into_any_element(),
                                    ],
                                )),
                        )
                        .child(
                            widgets::toggle_switch(theme, enabled, "auto-resume-limits-switch")
                                .id("auto-resume-limits")
                                .cursor_pointer()
                                .tab_index(0)
                                .role(gpui::Role::Switch)
                                .aria_label("Auto-resume after usage limits")
                                .aria_toggled(if enabled {
                                    gpui::Toggled::True
                                } else {
                                    gpui::Toggled::False
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.set_auto_resume_limits(!enabled, cx)
                                })),
                        ),
                )
                .into_any_element(),
        )
    }

    fn set_auto_resume_limits(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let params = self.with_target(serde_json::json!({ "enabled": enabled }));
        self.update_action_task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::SET_AUTO_RESUME_LIMITS, params)
                .await;
            this.update(cx, |page, cx| {
                if let Err(error) = result {
                    page.error = Some(error.to_string());
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn set_update_policy(
        &mut self,
        harness: HarnessId,
        policy: HarnessUpdatePolicy,
        cx: &mut Context<Self>,
    ) {
        if !self.can_control_updates(cx) {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let params = self.with_target(serde_json::json!({
            "harness": harness,
            "policy": policy,
        }));
        self.error = None;
        self.update_action_task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::SET_HARNESS_UPDATE_POLICY, params)
                .await;
            this.update(cx, |page, cx| {
                if let Err(error) = result {
                    page.error = Some(error.to_string());
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// The page-header device switcher (the Accounts pattern): platform glyph
    /// · name · presence dot · sort glyph, opening a dropdown of every
    /// registered device.
    fn render_device_switcher(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        use crate::icons::{self, icon};
        let (mut devices, local_id) = {
            let s = self.state.read(cx);
            (s.devices.clone(), s.local_device_id.clone())
        };
        devices.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        let effective = self.target_device.clone().or_else(|| local_id.clone());
        let platform_glyph = |platform: &str| match platform {
            "macos" | "darwin" => icons::LAPTOP,
            "ios" | "android" => icons::SMARTPHONE,
            _ => icons::MONITOR,
        };
        // Local device = no passthrough (calls stay direct).
        let mut targets: Vec<Option<String>> = Vec::new();
        let mut options = Vec::new();
        for device in &devices {
            let is_local = local_id.as_deref() == Some(device.id.as_str());
            let glyph = platform_glyph(&device.platform);
            let muted = theme.text_muted;
            let option = widgets::SelectOption::new(device.name.clone()).leading(move || {
                icon(glyph)
                    .size(px(16.0))
                    .flex_none()
                    .text_color(muted)
                    .into_any_element()
            });
            options.push(if is_local {
                option.detail("You")
            } else {
                option
            });
            targets.push((!is_local).then(|| device.id.clone()));
        }
        let selected = match devices
            .iter()
            .position(|d| Some(d.id.as_str()) == effective.as_deref())
        {
            Some(ix) => ix,
            // Not registered (yet): keep the current target reachable.
            None => {
                let muted = theme.text_muted;
                options.push(widgets::SelectOption::new("This device").leading(move || {
                    icon(icons::LAPTOP)
                        .size(px(16.0))
                        .flex_none()
                        .text_color(muted)
                        .into_any_element()
                }));
                targets.push(self.target_device.clone());
                options.len() - 1
            }
        };
        widgets::select(
            "harnesses-device-switcher",
            "Device",
            theme,
            |page: &mut Self| &mut page.device_select,
        )
        .options(options, selected)
        .menu_width(260.0)
        .heading("Devices")
        .on_select(move |page, ix, _, cx| {
            if let Some(target) = targets.get(ix) {
                page.set_target_device(target.clone(), cx);
            }
        })
        .render(&self.device_select, cx)
        .into_any_element()
    }

    fn rows(&self, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let theme = Theme::of(cx).for_settings_surface();
        let Loadable::Ready(list) = &self.harnesses else {
            return Vec::new();
        };
        let descriptors = visible_harnesses(list);
        let enabled_count = descriptors.iter().filter(|d| descriptor_enabled(d)).count();
        let instances = self.instances.ready().cloned().unwrap_or_default();
        let listed = |instance: &ProviderInstance| {
            matches!(instance.config, InstanceConfig::Native { .. })
                && descriptors.iter().any(|d| d.id == instance.driver)
        };
        let orphans: Vec<AnyElement> = instances
            .iter()
            .filter(|instance| !listed(instance))
            .map(|instance| self.instance_row(instance, false, &theme, cx))
            .collect();
        let mut rows: Vec<AnyElement> = descriptors
            .clone()
            .into_iter()
            .enumerate()
            .map(|(ix, descriptor)| {
                let harness = descriptor.id;
                let installed = descriptor.installed;
                let enabled = match self.pending_toggle {
                    Some((pending, on)) if pending == harness => on,
                    _ => descriptor_enabled(&descriptor),
                };
                // The one enabled harness left can't be switched off — the
                // composer needs something to run — but only when it could
                // actually run: an uninstalled last harness stays togglable
                // (its hint says to turn it off) and the composer handles the
                // resulting empty set (mirrors the engine guard).
                let last_enabled = enabled && enabled_count == 1 && installed;
                // Turning OFF never needs the CLI (a default-on agent the
                // user doesn't want must not be stuck on because it isn't
                // installed); turning ON still does.
                let interactive = !last_enabled && (enabled || installed);
                let (icon_path, tint) = crate::pickers::harness_brand_icon(harness);

                let update = match &self.updates {
                    Loadable::Ready(statuses) => {
                        statuses.iter().find(|status| status.harness == harness)
                    }
                    _ => None,
                };

                let mut meta: Vec<gpui::AnyElement> = Vec::new();
                // Installing REPLACES the not-installed hint in place, so the
                // row's text never shifts while the install runs.

                if self.installing == Some(harness) {
                    meta.push(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(crate::loaders::mini_mono_spinner(
                                format!("harness-install-spinner-{harness:?}"),
                                1.5,
                                theme.text_muted,
                                cx.entity_id(),
                                cx,
                            ))
                            .child(SharedString::from(install_label(&descriptor.name)))
                            .into_any_element(),
                    );
                } else if !installed {
                    meta.push(
                        div()
                            .text_color(theme.text_muted.opacity(0.75))
                            .child(SharedString::from(install_hint(
                                harness,
                                enabled,
                                descriptor.can_install,
                            )))
                            .into_any_element(),
                    );
                }
                // Not installed: the hint above says all there is.
                if let Some(status) = update.filter(|_| installed) {
                    let (text, color) = harness_update_label(status, &theme);
                    // Next to an Update button the version reads as the
                    // jump it would make: v1.2 → v1.3.
                    let next = status
                        .latest_version
                        .as_deref()
                        .filter(|_| {
                            status.can_apply
                                && matches!(
                                    status.phase,
                                    HarnessUpdatePhase::Available
                                        | HarnessUpdatePhase::ManualActionRequired
                                )
                        })
                        .filter(|latest| status.installed_version.as_deref() != Some(*latest));
                    if let Some(next) = next.filter(|_| !text.is_empty()) {
                        meta.push(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.0))
                                .text_color(color)
                                .child(text)
                                .child(
                                    crate::icons::icon(crate::icons::ARROW_RIGHT)
                                        .size(px(11.0))
                                        .text_color(color),
                                )
                                .child(format!("v{next}"))
                                .into_any_element(),
                        );
                    } else if !text.is_empty() {
                        meta.push(div().text_color(color).child(text).into_any_element());
                    }
                }
                // widgets::row_tile with the brand tint honored (the Claude
                // mark keeps its orange, like the picker rail).
                let tile = div()
                    .flex_none()
                    .size(px(36.0))
                    .rounded(px(10.0))
                    .bg(theme.wash(0.06))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        crate::icons::icon(icon_path)
                            .size(px(16.0))
                            .text_color(tint.unwrap_or(theme.text_muted)),
                    );
                let expanded = enabled && self.expanded_harness == Some(harness);
                // The row's one update action sits inside the trigger, before
                // the chevron, so it appearing never moves the chevron; the
                // update policy lives in the expanded details.
                let update_action = update.and_then(|status| {
                    let (id, label, primary) = match status.phase {
                        HarnessUpdatePhase::Available
                        | HarnessUpdatePhase::ManualActionRequired
                            if status.can_apply =>
                        {
                            ("harness-update", "Update", true)
                        }
                        HarnessUpdatePhase::WaitingForIdle
                        | HarnessUpdatePhase::Preparing
                        | HarnessUpdatePhase::Downloading => {
                            ("harness-update-cancel", "Cancel", false)
                        }
                        _ => return None,
                    };
                    Some(
                        div()
                            .id((id, ix))
                            .flex_none()
                            .px(px(9.0))
                            .py(px(5.0))
                            .rounded(px(6.0))
                            .text_size(crate::typography::ui_rems(11.0))
                            .cursor_pointer()
                            .map(|button| {
                                if primary {
                                    button
                                        .bg(theme.accent_wash)
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .text_color(theme.accent)
                                        .hover(|style| style.bg(theme.accent.opacity(0.16)))
                                } else {
                                    button
                                        .text_color(theme.text_muted)
                                        .hover(|style| style.bg(crate::theme::ink(0.05)))
                                }
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                // Inside the details trigger: act, don't expand.
                                cx.stop_propagation();
                                if primary {
                                    this.apply_harness_update(harness, cx)
                                } else {
                                    this.cancel_harness_update(harness, cx)
                                }
                            }))
                            .child(label),
                    )
                });
                let header = widgets::card_row(&theme, ix == 0)
                    .id(("harness-row", ix))
                    .when(!installed && self.installing != Some(harness), |el| {
                        el.opacity(0.55)
                    })
                    .child(
                        div()
                            .id(("harness-details-trigger", ix))
                            .group(format!("harness-details-{ix}"))
                            .flex_1()
                            .min_w(px(180.0))
                            .min_h(px(44.0))
                            .px(px(4.0))
                            .rounded(px(8.0))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(12.0))
                            .when(enabled, |el| {
                                el.role(gpui::Role::Button)
                                    .aria_label(format!("{} preferences", descriptor.name))
                                    .aria_expanded(expanded)
                                    .tab_index(0)
                                    .cursor_pointer()
                                    // No hover slab (it stopped short of the
                                    // toggle and boxed the row); the chevron
                                    // lifts instead, like a list disclosure.
                                    .focus_visible(|s| s.border_2().border_color(theme.accent))
                                    .on_click(cx.listener(move |page, _, _, cx| {
                                        page.toggle_agent_details(harness, cx)
                                    }))
                                    .on_key_down(cx.listener(
                                        move |page, event: &gpui::KeyDownEvent, _, cx| {
                                            if !event.is_held
                                                && matches!(
                                                    event.keystroke.key.as_str(),
                                                    "enter" | "space"
                                                )
                                            {
                                                page.toggle_agent_details(harness, cx);
                                                cx.stop_propagation();
                                            }
                                        },
                                    ))
                            })
                            .child(tile)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .child(widgets::row_title(&theme, descriptor.name.clone()))
                                    .when(!meta.is_empty(), |label| {
                                        label.child(widgets::meta_line(&theme, meta))
                                    }),
                            )
                            .children(update_action)
                            // Install / Cancel sit with the row's other
                            // action, before the caret, so carets line up.
                            .when(
                                offers_install(harness, installed, descriptor.can_install)
                                    && self.installing != Some(harness),
                                |el| {
                                    el.child(
                                        widgets::ghost_action(&theme)
                                            .id(("harness-install", ix))
                                            .flex_none()
                                            .when(self.installing.is_none(), |el| {
                                                el.on_click(cx.listener(move |this, _, _, cx| {
                                                    cx.stop_propagation();
                                                    this.install(harness, cx)
                                                }))
                                            })
                                            .child("Install"),
                                    )
                                },
                            )
                            .when(self.installing == Some(harness), |el| {
                                el.child(
                                    widgets::ghost_action(&theme)
                                        .id(("harness-cancel-install", ix))
                                        .flex_none()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.cancel_install(cx)
                                        }))
                                        .child("Cancel"),
                                )
                            })
                            // Always present so every row's controls hold
                            // their place; inert while the provider is off.
                            .child(
                                crate::icons::icon(if expanded {
                                    crate::icons::ALT_ARROW_DOWN
                                } else {
                                    crate::icons::ALT_ARROW_RIGHT
                                })
                                .size(px(14.0))
                                .flex_none()
                                .when(enabled, |caret| {
                                    caret
                                        .text_color(theme.text_muted)
                                        .group_hover(format!("harness-details-{ix}"), |s| {
                                            s.text_color(theme.text)
                                        })
                                })
                                .when(!enabled, |caret| {
                                    caret.text_color(theme.text_muted.opacity(0.35))
                                }),
                            ),
                    )
                    .child(
                        widgets::toggle_switch(
                            &theme,
                            enabled,
                            format!("harness-switch-{harness:?}"),
                        )
                        .id(("harness-toggle", ix))
                        .when(!interactive && !enabled, |el| el.opacity(0.55))
                        .when(interactive, |el| {
                            el.cursor_pointer()
                                .tab_index(0)
                                .role(gpui::Role::Switch)
                                .aria_label(descriptor.name.clone())
                                .aria_toggled(if enabled {
                                    gpui::Toggled::True
                                } else {
                                    gpui::Toggled::False
                                })
                                .focus_visible(|s| {
                                    s.border_2().border_color(theme.accent).opacity(1.0)
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.toggle(harness, !enabled, cx);
                                }))
                                .on_key_down(cx.listener(
                                    move |this, event: &gpui::KeyDownEvent, _, cx| {
                                        if !event.is_held
                                            && matches!(
                                                event.keystroke.key.as_str(),
                                                "enter" | "space"
                                            )
                                        {
                                            this.toggle(harness, !enabled, cx);
                                            cx.stop_propagation();
                                        }
                                    },
                                ))
                        }),
                    );
                let instance_rows: Vec<AnyElement> = instances
                    .iter()
                    .filter(|instance| instance.driver == harness && listed(instance))
                    .map(|instance| self.instance_row(instance, true, &theme, cx))
                    .collect();
                div()
                    .flex()
                    .flex_col()
                    .child(header)
                    .when(expanded, |row| {
                        row.child(self.render_agent_details(harness, &theme, cx))
                    })
                    .children(instance_rows)
                    .into_any_element()
            })
            .collect();
        rows.extend(orphans);
        rows
    }
    fn on_scroll_hovered(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.scroll.set_list_hovered(*hovered) {
            cx.notify();
        }
    }
}

impl popover::ScrollRailHost for HarnessesPage {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        self.scroll.rail_bar()
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.scroll.rail_scroll()
    }
}

impl Render for HarnessesPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_settings_surface();
        let supports_updates = self.supports_updates(cx);
        if self.scroll_end_knob
            && self.harnesses.ready().is_some()
            && self.instances.ready().is_some()
        {
            self.scroll_end_knob = false;
            self.scroll.scroll.scroll_to_bottom();
        }
        let add_provider = widgets::action_button(&theme, widgets::ActionTone::Outlined)
            .id("add-provider")
            .flex_none()
            .tab_index(0)
            .role(gpui::Role::Button)
            .focus_visible(|s| s.border_2().border_color(theme.accent))
            .on_click(cx.listener(|page, _, _, cx| page.open_wizard(None, cx)))
            .child(
                crate::icons::icon(crate::icons::PLUS)
                    .size(px(14.0))
                    .text_color(theme.text_muted),
            )
            .child("Add provider");
        let instances_error = match &self.instances {
            Loadable::Error(message) => Some(widgets::error_strip(
                &theme,
                format!("Provider instances: {message}"),
            )),
            _ => None,
        };
        let wizard = self.wizard.as_ref().map(|(wizard, _)| wizard.clone());

        let body: gpui::AnyElement = match &self.harnesses {
            Loadable::Idle | Loadable::Loading => widgets::section_card(&theme)
                .p(px(16.0))
                .child(popover::skeleton_rows(
                    "harnesses-skeleton",
                    &theme,
                    4,
                    cx.entity_id(),
                    cx,
                ))
                .into_any_element(),
            Loadable::Error(message) => {
                let message = message.clone();
                div()
                    .child(widgets::error_strip(&theme, message))
                    .child(
                        widgets::ghost_action(&theme)
                            .id("harnesses-retry")
                            .mt(px(8.0))
                            .tab_index(0)
                            .role(gpui::Role::Button)
                            .focus_visible(|s| s.border_2().border_color(theme.accent).opacity(1.0))
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.load(cx);
                                cx.notify();
                            }))
                            .child(SharedString::from("Retry")),
                    )
                    .into_any_element()
            }
            Loadable::Ready(_) => {
                let rows = self.rows(cx);
                widgets::section_card(&theme)
                    .children(rows)
                    .into_any_element()
            }
        };
        crate::toast::take_error(&mut self.error, cx);
        let update_error = match &self.updates {
            Loadable::Error(message) => Some(
                div()
                    .child(widgets::error_strip(
                        &theme,
                        format!("Agent updates: {message}"),
                    ))
                    .child(
                        widgets::ghost_action(&theme)
                            .id("harness-updates-retry")
                            .mt(px(8.0))
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.load(cx);
                                cx.notify();
                            }))
                            .child("Retry updates"),
                    )
                    .into_any_element(),
            ),
            _ => None,
        };
        let switcher = self.render_device_switcher(&theme, cx);
        let limits = self.render_limit_resume_card(&theme, cx);

        let can_control = self.can_control_updates(cx);
        let check = div()
            .id("check-harness-updates")
            .flex_none()
            .px(px(9.0))
            .py(px(5.0))
            .rounded(px(6.0))
            .text_size(crate::typography::ui_rems(11.0))
            .text_color(theme.text_muted)
            .cursor_pointer()
            .hover(|style| style.bg(crate::theme::ink(0.05)))
            .opacity(if can_control { 1.0 } else { 0.4 })
            .when(can_control, |button| {
                button.on_click(cx.listener(|this, _, _, cx| this.check_updates(cx)))
            })
            .child(if self.checking_updates {
                "Checking…"
            } else {
                "Check now"
            });

        let scrollbar = popover::rail(self, "harnesses-page-scrollbar", &theme, cx);

        div()
            .id("harnesses-page-host")
            .relative()
            .size_full()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .child(
                crate::edge_fade::edge_faded(
                    16.0,
                    true,
                    true,
                    div()
                        .id("harnesses-page")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll.scroll)
                        .child(
                            widgets::page_column()
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .justify_between()
                                        .child(widgets::page_header(&theme, "Providers", None))
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap(px(6.0))
                                                .when(supports_updates, |el| el.child(check))
                                                .child(add_provider)
                                                .child(switcher),
                                        ),
                                )
                                .children(update_error)
                                .children(instances_error)
                                .child(body)
                                .children(limits),
                        ),
                )
                .fade_overflow_y(&self.scroll.scroll),
            )
            .children(scrollbar)
            .children(wizard)
    }
}

#[cfg(test)]
mod tests {
    #[gpui::test]
    fn expanded_agent_preferences_render_inside_the_agent_row(cx: &mut gpui::TestAppContext) {
        use gpui::AppContext;
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            crate::settings::init(Default::default(), dir.path(), cx);
            gpui_base::init(cx);
            cx.set_global(crate::theme::Theme::default());
        });
        let window = cx.add_window(|_, cx| {
            let state = cx.new(|_| crate::state::AppState::new());
            super::HarnessesPage::new(state, cx)
        });
        let descriptor = |id, name: &str| zeron_engine::registry::HarnessDescriptor {
            id,
            name: name.into(),
            supports_steering: false,
            steering_mode: zeron_proto::SteeringMode::TurnBoundary,
            reasoning_levels: Vec::new(),
            installed: true,
            can_install: false,
            enabled: Some(true),
        };
        window
            .update(cx, |page, _, cx| {
                page.harnesses = super::Loadable::Ready(vec![
                    descriptor(zeron_proto::HarnessId::ClaudeCode, "Claude Code"),
                    descriptor(zeron_proto::HarnessId::Codex, "Codex"),
                ]);
                page.toggle_agent_details(zeron_proto::HarnessId::ClaudeCode, cx);
                assert_eq!(
                    page.expanded_harness,
                    Some(zeron_proto::HarnessId::ClaudeCode)
                );
                assert_eq!(
                    page.accounts_page
                        .as_ref()
                        .unwrap()
                        .read(cx)
                        .embedded_harness(),
                    Some(zeron_proto::HarnessId::ClaudeCode)
                );
            })
            .unwrap();
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
        window
            .update(cx, |page, _, cx| {
                page.toggle_agent_details(zeron_proto::HarnessId::Codex, cx);
                assert_eq!(page.expanded_harness, Some(zeron_proto::HarnessId::Codex));
                assert_eq!(
                    page.accounts_page
                        .as_ref()
                        .unwrap()
                        .read(cx)
                        .embedded_harness(),
                    Some(zeron_proto::HarnessId::Codex)
                );
            })
            .unwrap();
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
        // Antigravity signs in through the very same Accounts section.
        window
            .update(cx, |page, _, cx| {
                page.harnesses = super::Loadable::Ready(vec![descriptor(
                    zeron_proto::HarnessId::Antigravity,
                    "Antigravity",
                )]);
                page.toggle_agent_details(zeron_proto::HarnessId::Antigravity, cx);
                assert_eq!(
                    page.accounts_page
                        .as_ref()
                        .unwrap()
                        .read(cx)
                        .embedded_harness(),
                    Some(zeron_proto::HarnessId::Antigravity)
                );
            })
            .unwrap();
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
    }
}

#[cfg(test)]
#[test]
fn install_visibility_and_hint_follow_target_capabilities() {
    for id in [
        HarnessId::Antigravity,
        HarnessId::Codex,
        HarnessId::Opencode,
        HarnessId::ClaudeCode,
        HarnessId::Cursor,
        HarnessId::Pi,
        HarnessId::Grok,
        HarnessId::Hermes,
        HarnessId::Devin,
        HarnessId::Mock,
    ] {
        for installed in [false, true] {
            for available in [false, true] {
                assert_eq!(
                    offers_install(id, installed, available),
                    id != HarnessId::Mock && !installed && available
                );
            }
        }
    }
    assert_eq!(cli_name(HarnessId::Antigravity), "Antigravity");
    assert_eq!(
        install_hint(HarnessId::Antigravity, false, true),
        "Install Antigravity to enable"
    );
    assert!(
        install_hint(HarnessId::Antigravity, false, false).contains("ANTIGRAVITY_ACP_EXECUTABLE")
    );
}

#[cfg(test)]
#[test]
fn install_phase_copy_and_cancel_target_match_install() {
    assert_eq!(install_label("Claude Code"), "Installing Claude Code…");
    assert_eq!(install_label("Pi"), "Installing Pi…");
    for target in [None, Some("remote-device".to_string())] {
        let params = install_params(HarnessId::Pi, &target);
        assert_eq!(params["harness"], "pi");
        assert_eq!(
            params["targetDeviceId"],
            serde_json::to_value(&target).unwrap()
        );
    }
    assert_eq!(
        install_hint(HarnessId::Codex, false, false),
        "Install the codex CLI to enable. Install with `npm install -g @openai/codex`"
    );
    assert!(
        install_hint(HarnessId::Codex, true, false)
            .starts_with("codex CLI not installed — turn it off or install it.")
    );
}
