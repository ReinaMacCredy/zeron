use std::collections::HashMap;

use gpui::{
    AnyElement, Context, Entity, SharedString, Stateful, Subscription, Task, Window, div,
    prelude::*, px,
};
use zeron_proto::device_hub::{
    DeviceHostStatus, DeviceHostTest, DeviceHubState, DevicePlatform, SshDeviceHostConfig,
};
use zeron_rpc::methods;

use super::permission_guide::row;
use super::widgets;
use crate::composer::ComposerInput;
use crate::device_panel::setup::{self, SetupPending, activate, button, switch};
use crate::popover;
use crate::settings::{self, SavePolicy};
use crate::state::AppState;
use crate::theme::Theme;

pub const AUTO_FLOAT_DESCRIPTION: &str = "Show the floating preview when an agent opens a device.";
const UPDATE_FAILED: &str = "Update failed. Check this host's network connection and try again.";
pub const DEVICE_HOSTS_DESCRIPTION: &str = "Add remote machines with simulator or emulator runtimes installed, and the selected environments will connect over SSH and set up device tools automatically.";
const SAME_MACHINE: &str = "Hosts on the same machine are skipped.";

enum HostCheck {
    Pending,
    Done(DeviceHostTest),
    Failed(String),
}

struct HostEditor {
    id: String,
    name: Entity<ComposerInput>,
    target: Entity<ComposerInput>,
    identity: Entity<ComposerInput>,
    port: Entity<ComposerInput>,
    error: Option<SharedString>,
}

pub struct DeviceHubPage {
    state: Entity<AppState>,
    scroll: widgets::PageScroll,
    pending: Option<SetupPending>,
    update_error: Option<SharedString>,
    task: Option<Task<()>>,
    editor: Option<HostEditor>,
    checks: HashMap<String, HostCheck>,
    check_tasks: HashMap<String, Task<()>>,
    hosts_task: Option<Task<()>>,
    hosts_error: Option<SharedString>,
    _state: Subscription,
}

impl DeviceHubPage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let observer = cx.observe(&state, |_, _, cx| cx.notify());
        let mut page = Self {
            state,
            scroll: widgets::PageScroll::default(),
            pending: None,
            update_error: None,
            task: None,
            editor: None,
            checks: HashMap::new(),
            check_tasks: HashMap::new(),
            hosts_task: None,
            hosts_error: None,
            _state: observer,
        };
        page.call(
            None,
            methods::LIST_HUB_DEVICES,
            serde_json::json!({ "inspectOnly": true }),
            cx,
        );
        page
    }

    fn hub<'a>(&self, cx: &'a gpui::App) -> Option<&'a DeviceHubState> {
        self.state.read(cx).device_hub.as_ref()
    }

    fn call(
        &mut self,
        pending: Option<SetupPending>,
        method: &'static str,
        params: serde_json::Value,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.pending = pending;
        self.task = Some(cx.spawn(async move |this, cx| {
            let reply = engine.client().call(method, params).await;
            this.update(cx, |page, cx| {
                page.pending = None;
                if pending == Some(SetupPending::Update) && reply.is_err() {
                    page.update_error = Some(UPDATE_FAILED.into());
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn configure(
        &mut self,
        pending: SetupPending,
        params: serde_json::Value,
        cx: &mut Context<Self>,
    ) {
        self.call(Some(pending), methods::CONFIGURE_DEVICE_HUB, params, cx);
    }

    fn list(&mut self, pending: SetupPending, inspect_only: bool, cx: &mut Context<Self>) {
        self.update_error = None;
        let params = if inspect_only {
            serde_json::json!({ "inspectOnly": true })
        } else {
            serde_json::json!({})
        };
        self.call(Some(pending), methods::LIST_HUB_DEVICES, params, cx);
    }

    fn saved_hosts(&self, cx: &gpui::App) -> Vec<SshDeviceHostConfig> {
        self.hub(cx)
            .map(|hub| hub.settings.device_hosts.clone())
            .unwrap_or_default()
    }

    fn open_editor(&mut self, cx: &mut Context<Self>) {
        let field = |placeholder: &str, cx: &mut Context<Self>| {
            let placeholder = SharedString::from(placeholder.to_string());
            cx.new(|cx| ComposerInput::new(placeholder, cx))
        };
        self.editor = Some(HostEditor {
            id: format!("host-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]),
            name: field("Mac mini", cx),
            target: field("user@host or SSH alias", cx),
            identity: field("SSH config default", cx),
            port: field("Default", cx),
            error: None,
        });
        cx.notify();
    }

    fn draft(editor: &HostEditor, cx: &gpui::App) -> Result<SshDeviceHostConfig, String> {
        let text = |input: &Entity<ComposerInput>| input.read(cx).text().trim().to_string();
        let (name, target, identity, port) = (
            text(&editor.name),
            text(&editor.target),
            text(&editor.identity),
            text(&editor.port),
        );
        let port = match port.as_str() {
            "" => None,
            port => Some(
                port.parse::<u16>()
                    .ok()
                    .filter(|port| *port > 0)
                    .ok_or("The SSH port must be between 1 and 65535.")?,
            ),
        };
        let config = SshDeviceHostConfig {
            id: editor.id.clone(),
            label: if name.is_empty() {
                target.clone()
            } else {
                name
            },
            target,
            identity_file: (!identity.is_empty()).then_some(identity),
            port,
        };
        config.validate()?;
        Ok(config)
    }

    fn test_host(&mut self, host: SshDeviceHostConfig, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let id = host.id.clone();
        self.checks.insert(id.clone(), HostCheck::Pending);
        let params = serde_json::to_value(&host).unwrap_or_default();
        let task = cx.spawn({
            let id = id.clone();
            async move |this, cx| {
                let reply = engine
                    .client()
                    .call(methods::TEST_DEVICE_HOST, params)
                    .await;
                let check = match reply.map_err(|e| e.to_string()).and_then(|value| {
                    serde_json::from_value::<DeviceHostTest>(value).map_err(|e| e.to_string())
                }) {
                    Ok(test) => HostCheck::Done(test),
                    Err(error) => HostCheck::Failed(error),
                };
                this.update(cx, |page, cx| {
                    page.checks.insert(id.clone(), check);
                    page.check_tasks.remove(&id);
                    cx.notify();
                })
                .ok();
            }
        });
        self.check_tasks.insert(id, task);
        cx.notify();
    }

    fn test_draft(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        match Self::draft(editor, cx) {
            Ok(host) => {
                editor.error = None;
                self.test_host(host, cx);
            }
            Err(error) => {
                editor.error = Some(error.into());
                cx.notify();
            }
        }
    }

    fn save_hosts(
        &mut self,
        hosts: Vec<SshDeviceHostConfig>,
        close_editor: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.hosts_error = None;
        let params = serde_json::json!({ "deviceHosts": hosts });
        self.hosts_task = Some(cx.spawn(async move |this, cx| {
            let reply = engine
                .client()
                .call(methods::CONFIGURE_DEVICE_HUB, params)
                .await;
            this.update(cx, |page, cx| {
                page.hosts_task = None;
                match reply {
                    Ok(_) if close_editor => page.editor = None,
                    Ok(_) => {}
                    Err(error) => page.hosts_error = Some(error.to_string().into()),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn save_draft(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        let host = match Self::draft(editor, cx) {
            Ok(host) => host,
            Err(error) => {
                editor.error = Some(error.into());
                cx.notify();
                return;
            }
        };
        let mut hosts = self.saved_hosts(cx);
        hosts.retain(|existing| existing.id != host.id);
        hosts.push(host);
        self.save_hosts(hosts, true, cx);
    }

    fn remove_host(&mut self, id: &str, cx: &mut Context<Self>) {
        let mut hosts = self.saved_hosts(cx);
        hosts.retain(|host| host.id != id);
        self.checks.remove(id);
        self.save_hosts(hosts, false, cx);
    }

    fn render_check(theme: &Theme, check: &HostCheck) -> AnyElement {
        let line = |ok: bool, text: String| {
            div()
                .flex()
                .items_start()
                .gap(px(6.0))
                .text_size(crate::typography::ui_rems(12.0))
                .text_color(if ok { theme.text_muted } else { theme.danger })
                .child(
                    crate::icons::icon(if ok {
                        crate::icons::CHECK
                    } else {
                        crate::icons::CLOSE
                    })
                    .size(px(12.0))
                    .flex_none()
                    .mt(px(2.0)),
                )
                .child(div().min_w_0().child(SharedString::from(text)))
        };
        let body = div().flex().flex_col().gap(px(4.0));
        match check {
            HostCheck::Pending => body.child(
                div()
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .child("Checking connection..."),
            ),
            HostCheck::Failed(error) => body.child(line(false, error.clone())),
            HostCheck::Done(test) if test.local => {
                body.child(line(true, "Already available locally".into()))
            }
            HostCheck::Done(test) => body.children(test.checks.iter().map(|check| {
                let text = match &check.detail {
                    Some(detail) => format!("{}: {detail}", check.name),
                    None => check.name.clone(),
                };
                line(check.ok, text)
            })),
        }
        .id("device-host-check")
        .debug_selector(|| "device-host-check".into())
        .role(gpui::Role::Status)
        .into_any_element()
    }

    fn host_button(theme: &Theme, id: String, label: &'static str) -> Stateful<gpui::Div> {
        let selector = id.clone();
        super::permission_guide::outline_button(theme)
            .id(SharedString::from(id))
            .debug_selector(move || selector.clone())
            .tab_index(0)
            .role(gpui::Role::Button)
            .aria_label(label)
            .child(label)
    }

    fn render_hosts(
        &mut self,
        theme: &Theme,
        hub: &DeviceHubState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let saving = self.hosts_task.is_some();
        let small = |text: String, color| {
            div()
                .text_size(crate::typography::ui_rems(12.0))
                .text_color(color)
                .child(SharedString::from(text))
        };
        let mut list = div().flex().flex_col().gap(px(10.0));
        if hub.settings.device_hosts.is_empty() && self.editor.is_none() {
            list = list.child(small("No device hosts.".into(), theme.text_muted));
        }
        for host in &hub.settings.device_hosts {
            let summary = hub.hosts.iter().find(|entry| entry.id == host.id);
            let marks: Vec<&str> = summary
                .map(|entry| {
                    entry
                        .platforms
                        .iter()
                        .filter(|platform| platform.available)
                        .map(|platform| match platform.platform {
                            DevicePlatform::Ios => "iOS",
                            DevicePlatform::Android => "Android",
                        })
                        .collect()
                })
                .unwrap_or_default();
            let status = hub.host_statuses.get(&host.id);
            let checking = matches!(self.checks.get(&host.id), Some(HostCheck::Pending));
            let progress = match status.map(|status| status.status) {
                _ if checking => Some("Checking connection...".to_string()),
                Some(DeviceHostStatus::Installing) => Some("Installing device support...".into()),
                Some(DeviceHostStatus::Starting) => Some(
                    status
                        .and_then(|status| status.detail.clone())
                        .unwrap_or_else(|| "Connecting...".into()),
                ),
                _ => None,
            };
            let failure = status
                .filter(|status| status.status == DeviceHostStatus::Failed)
                .and_then(|status| status.detail.clone());
            let id = host.id.clone();
            let remove_id = host.id.clone();
            let test_host = host.clone();
            let header = div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .text_size(crate::typography::ui_rems(13.0))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(theme.text)
                                .child(SharedString::from(host.label.clone()))
                                .children(marks.into_iter().map(|mark| {
                                    div()
                                        .text_size(crate::typography::ui_rems(11.0))
                                        .text_color(theme.text_muted)
                                        .child(format!("{mark} available"))
                                })),
                        )
                        .child(
                            div()
                                .text_size(crate::typography::ui_rems(12.0))
                                .text_color(theme.text_muted)
                                .truncate()
                                .child(SharedString::from(host.target.clone())),
                        ),
                )
                .children(progress.map(|text| small(text, theme.text_muted)))
                .child(activate(
                    Self::host_button(
                        theme,
                        format!("settings-device-host-test-{id}"),
                        "Test connection",
                    ),
                    !saving && !checking,
                    cx,
                    move |page: &mut Self, cx| page.test_host(test_host.clone(), cx),
                ))
                .child(activate(
                    Self::host_button(theme, format!("settings-device-host-remove-{id}"), "Remove"),
                    !saving,
                    cx,
                    move |page: &mut Self, cx| page.remove_host(&remove_id, cx),
                ));
            list =
                list.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(header)
                        .children(failure.map(|detail| {
                            small(format!("Connection failed: {detail}"), theme.danger)
                        }))
                        .children(
                            self.checks
                                .get(&host.id)
                                .filter(|check| !matches!(check, HostCheck::Pending))
                                .map(|check| Self::render_check(theme, check)),
                        ),
                );
        }
        if let Some(editor) = &self.editor {
            let field = |label: &'static str, input: &Entity<ComposerInput>| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(small(label.into(), theme.text_muted))
                    .child(crate::popover::dialog_field(
                        input.clone().into_any_element(),
                    ))
            };
            let check = self.checks.get(&editor.id);
            let checking = matches!(check, Some(HostCheck::Pending));
            list = list.child(
                div()
                    .id("settings-device-host-editor")
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .p(px(12.0))
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(theme.border)
                    .child(small(SAME_MACHINE.into(), theme.text_muted))
                    .child(field("Name", &editor.name))
                    .child(field("SSH target", &editor.target))
                    .child(
                        div()
                            .flex()
                            .gap(px(12.0))
                            .child(
                                div()
                                    .flex_1()
                                    .child(field("Identity file", &editor.identity)),
                            )
                            .child(div().w(px(110.0)).child(field("Port", &editor.port))),
                    )
                    .children(
                        editor
                            .error
                            .clone()
                            .map(|error| small(error.to_string(), theme.danger)),
                    )
                    .children(check.map(|check| Self::render_check(theme, check)))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.0))
                            .child(activate(
                                button(theme, "settings-device-host-cancel", "Cancel"),
                                !saving,
                                cx,
                                |page: &mut Self, cx| {
                                    page.editor = None;
                                    cx.notify();
                                },
                            ))
                            .child(activate(
                                button(
                                    theme,
                                    "settings-device-host-editor-test",
                                    if checking {
                                        "Checking..."
                                    } else {
                                        "Test connection"
                                    },
                                ),
                                !saving && !checking,
                                cx,
                                |page: &mut Self, cx| page.test_draft(cx),
                            ))
                            .child(activate(
                                button(
                                    theme,
                                    "settings-device-host-save",
                                    if saving { "Saving..." } else { "Save host" },
                                ),
                                !saving && !checking,
                                cx,
                                |page: &mut Self, cx| page.save_draft(cx),
                            )),
                    ),
            );
        }
        list.children(
            self.hosts_error
                .clone()
                .map(|error| small(error.to_string(), theme.danger)),
        )
        .into_any_element()
    }

    fn on_scroll_hovered(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.scroll.set_list_hovered(*hovered) {
            cx.notify();
        }
    }

    fn version_rows(hub: &DeviceHubState) -> Option<[(&'static str, String); 3]> {
        let tool = &hub
            .hosts
            .iter()
            .find(|host| host.kind == "local")?
            .tools
            .as_ref()?
            .hub;
        let installed = if tool.installed.is_empty() {
            "None".to_string()
        } else {
            tool.installed.join(", ")
        };
        Some([
            (
                "Running",
                tool.running.clone().unwrap_or_else(|| "Not running".into()),
            ),
            ("Required", tool.required.clone()),
            ("Installed", installed),
        ])
    }

    fn needs_update(hub: &DeviceHubState) -> Option<String> {
        let tool = &hub
            .hosts
            .iter()
            .find(|host| host.kind == "local")?
            .tools
            .as_ref()?
            .hub;
        (!tool.installed.contains(&tool.required)).then(|| tool.required.clone())
    }
}

impl popover::ScrollRailHost for DeviceHubPage {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        self.scroll.rail_bar()
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.scroll.rail_scroll()
    }
}

impl Render for DeviceHubPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_settings_surface();
        let loaded = self.hub(cx).is_some();
        let hub = self.hub(cx).cloned().unwrap_or_default();
        let enabled = hub.settings.enabled;
        let busy = setup::busy(&hub);
        let idle = self.pending.is_none();
        let owner = cx.entity_id();

        let mut hub_controls = div().flex().items_center().gap(px(8.0));
        if self.pending == Some(SetupPending::Hub)
            && let Some(text) = setup::hub_setup_status(&hub, true, true)
        {
            hub_controls = hub_controls.child(setup::status_line(&theme, owner, text, true, cx));
        }
        hub_controls = hub_controls.child(activate(
            switch(&theme, "settings-device-hub", "Device hub", enabled),
            loaded && !busy && idle,
            cx,
            move |page: &mut Self, cx| {
                let params = if enabled {
                    serde_json::json!({ "enabled": false, "agentAccessEnabled": false })
                } else {
                    serde_json::json!({ "enabled": true, "onboardingCompleted": true })
                };
                page.configure(SetupPending::Hub, params, cx);
            },
        ));
        let checking = self.pending == Some(SetupPending::Check);
        let updating = self.pending == Some(SetupPending::Update);
        let versions = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(match Self::version_rows(&hub) {
                Some(rows) => div()
                    .max_w(px(280.0))
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .text_size(crate::typography::ui_rems(12.0))
                    .children(rows.into_iter().map(|(label, value)| {
                        div()
                            .flex()
                            .justify_between()
                            .gap(px(24.0))
                            .child(div().text_color(theme.text_muted).child(label))
                            .child(
                                div()
                                    .min_w_0()
                                    .font_family(theme.font_mono.clone())
                                    .text_color(theme.text)
                                    .child(SharedString::from(value)),
                            )
                    })),
                None => div()
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .child("Versions have not been checked."),
            })
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .when_some(Self::needs_update(&hub), |el, version| {
                        el.child(activate(
                            button(
                                &theme,
                                "settings-device-hub-update",
                                if updating {
                                    "Updating...".to_string()
                                } else {
                                    format!("Update to v{version}")
                                },
                            ),
                            enabled && idle && !busy,
                            cx,
                            |page: &mut Self, cx| page.list(SetupPending::Update, false, cx),
                        ))
                    })
                    .child(activate(
                        button(
                            &theme,
                            "settings-device-hub-check",
                            if checking {
                                "Checking..."
                            } else {
                                "Check versions"
                            },
                        ),
                        idle && !busy,
                        cx,
                        |page: &mut Self, cx| page.list(SetupPending::Check, true, cx),
                    )),
            )
            .when_some(self.update_error.clone(), |el, error| {
                el.child(
                    div()
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.danger)
                        .child(error),
                )
            })
            .into_any_element();

        let mut card = widgets::section_card(&theme).mt_0().child(row(
            &theme,
            true,
            "Device hub",
            setup::DEVICE_HUB_DESCRIPTION,
            None,
            None,
            hub_controls.into_any_element(),
            Some(
                div()
                    .px(px(20.0))
                    .pb(px(14.0))
                    .child(versions)
                    .into_any_element(),
            ),
        ));
        if enabled {
            let platforms = div()
                .flex()
                .flex_wrap()
                .gap(px(20.0))
                .child(setup::platform_card(
                    &theme,
                    DevicePlatform::Ios,
                    setup::platform_setup_status(&hub, DevicePlatform::Ios),
                    true,
                ))
                .child(setup::platform_card(
                    &theme,
                    DevicePlatform::Android,
                    setup::platform_setup_status(&hub, DevicePlatform::Android),
                    true,
                ))
                .into_any_element();
            card = card.child(row(
                &theme,
                false,
                "Platform support",
                "",
                None,
                None,
                activate(
                    button(
                        &theme,
                        "settings-device-platforms-refresh",
                        if self.pending == Some(SetupPending::Check) {
                            "Checking..."
                        } else {
                            "Refresh"
                        },
                    ),
                    enabled && !busy && idle,
                    cx,
                    |page: &mut Self, cx| page.list(SetupPending::Check, false, cx),
                )
                .into_any_element(),
                Some(
                    div()
                        .px(px(20.0))
                        .pb(px(14.0))
                        .child(platforms)
                        .into_any_element(),
                ),
            ));
        }
        let agent = hub.settings.agent_access_enabled;
        let mut agent_controls = div().flex().items_center().gap(px(8.0));
        if self.pending == Some(SetupPending::Agent) {
            agent_controls = agent_controls.child(setup::status_line(
                &theme,
                owner,
                setup::agent_setup_status(&hub, true).into(),
                true,
                cx,
            ));
        }
        agent_controls = agent_controls.child(activate(
            switch(
                &theme,
                "settings-device-agent-access",
                "Agent device access",
                agent,
            ),
            loaded && enabled && !busy && idle,
            cx,
            move |page: &mut Self, cx| {
                page.configure(
                    SetupPending::Agent,
                    serde_json::json!({ "agentAccessEnabled": !agent }),
                    cx,
                );
            },
        ));
        card = card.child(row(
            &theme,
            false,
            "Agent device access",
            setup::AGENT_DEVICE_DESCRIPTION,
            setup::agent_failure(&hub).map(SharedString::from),
            None,
            agent_controls.into_any_element(),
            None,
        ));
        let auto_float = settings::current(cx).device_auto_float;
        card = card.child(row(
            &theme,
            false,
            "Auto-show floating preview",
            AUTO_FLOAT_DESCRIPTION,
            None,
            None,
            activate(
                switch(
                    &theme,
                    "settings-device-auto-float",
                    "Auto-show floating preview",
                    auto_float,
                ),
                true,
                cx,
                move |_: &mut Self, cx| {
                    settings::update(SavePolicy::Immediate, cx, |settings| {
                        settings.device_auto_float = !auto_float;
                    });
                    cx.notify();
                },
            )
            .into_any_element(),
            None,
        ));
        let hosts = self.render_hosts(&theme, &hub, cx);
        let editing = self.editor.is_some();
        card = card.child(row(
            &theme,
            false,
            "Device hosts",
            DEVICE_HOSTS_DESCRIPTION,
            None,
            None,
            activate(
                button(&theme, "settings-device-host-add", "Add host"),
                loaded && !editing && self.hosts_task.is_none(),
                cx,
                |page: &mut Self, cx| page.open_editor(cx),
            )
            .into_any_element(),
            Some(
                div()
                    .px(px(20.0))
                    .pb(px(14.0))
                    .child(hosts)
                    .into_any_element(),
            ),
        ));
        let update_banner: Option<AnyElement> =
            setup::host_update(&hub).map(|(label, detail, failed)| {
                setup::host_update_banner(&theme, label, detail, failed, |el| {
                    activate(el, enabled && idle, cx, |page: &mut Self, cx| {
                        page.list(SetupPending::Check, false, cx)
                    })
                })
            });

        let scrollbar = popover::rail(self, "device-hub-page-scrollbar", &theme, cx);
        div()
            .id("device-hub-page-host")
            .relative()
            .size_full()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .child(
                crate::edge_fade::edge_faded(
                    16.0,
                    true,
                    true,
                    div()
                        .id("device-hub-page")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll.scroll)
                        .child(
                            widgets::page_column()
                                .child(widgets::page_header(&theme, "Device hub", None))
                                .child(div().mt(px(24.0)).child(card))
                                .children(
                                    update_banner.map(|banner| div().mt(px(12.0)).child(banner)),
                                ),
                        ),
                )
                .fade_overflow_y(&self.scroll.scroll),
            )
            .children(scrollbar)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device_panel::tests::{app_state, calls_of, click, hub, wait};
    use gpui::TestAppContext;
    use zeron_proto::device_hub::{DeviceToolVersion, DeviceToolVersions};

    fn with_tools(mut state: DeviceHubState, installed: &[&str], revision: u64) -> DeviceHubState {
        state.revision = revision;
        state.hosts[0].tools = Some(DeviceToolVersions {
            hub: DeviceToolVersion {
                required: "0.12.0".into(),
                installed: installed.iter().map(|v| v.to_string()).collect(),
                running: None,
            },
        });
        state
    }

    #[test]
    fn the_version_rows_use_the_t3code_labels() {
        assert_eq!(
            DeviceHubPage::version_rows(&DeviceHubState::default()),
            None
        );
        assert_eq!(
            DeviceHubPage::version_rows(&with_tools(hub(true), &[], 0)),
            Some([
                ("Running", "Not running".to_string()),
                ("Required", "0.12.0".to_string()),
                ("Installed", "None".to_string()),
            ])
        );
        assert_eq!(
            DeviceHubPage::version_rows(&with_tools(hub(true), &["0.11.0", "0.12.0"], 0))
                .map(|rows| rows[2].1.clone()),
            Some("0.11.0, 0.12.0".to_string())
        );
    }

    #[gpui::test]
    fn the_page_inspects_on_open_and_enabling_also_completes_onboarding(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (state, calls) = app_state(cx, &runtime, with_tools(hub(false), &[], 0));
        let (_page, cx) = cx.add_window_view(|_, cx| DeviceHubPage::new(state.clone(), cx));
        wait(cx, |_| {
            !calls_of(&calls, methods::LIST_HUB_DEVICES).is_empty()
        });
        assert_eq!(
            calls_of(&calls, methods::LIST_HUB_DEVICES),
            vec![serde_json::json!({ "inspectOnly": true })]
        );
        assert!(cx.debug_bounds("settings-device-hub-update").is_some());
        click(cx, "settings-device-hub");
        wait(cx, |_| {
            !calls_of(&calls, methods::CONFIGURE_DEVICE_HUB).is_empty()
        });
        assert_eq!(
            calls_of(&calls, methods::CONFIGURE_DEVICE_HUB),
            vec![serde_json::json!({ "enabled": true, "onboardingCompleted": true })]
        );
        state.update(cx, |state, cx| {
            let mut enabled = with_tools(hub(true), &["0.12.0"], 1);
            enabled.host_status = zeron_proto::device_hub::DeviceHostStatus::Idle;
            state.device_hub = Some(enabled);
            cx.notify();
        });
        wait(cx, |cx| {
            cx.debug_bounds("settings-device-hub-update").is_none()
        });
        wait(cx, |cx| {
            cx.debug_bounds("settings-device-platforms-refresh")
                .is_some()
        });
        assert!(cx.debug_bounds("settings-device-agent-access").is_some());
        assert!(cx.debug_bounds("settings-device-auto-float").is_some());
    }

    #[gpui::test]
    fn a_long_platform_reason_wraps_inside_the_card(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let mut state = hub(true);
        state.hosts[0].platforms = vec![
            zeron_proto::device_hub::DevicePlatformAvailability {
                platform: DevicePlatform::Ios,
                available: true,
                reason: None,
            },
            zeron_proto::device_hub::DevicePlatformAvailability {
                platform: DevicePlatform::Android,
                available: false,
                reason: Some(
                    "The Android SDK was not found. Install Android Studio, or set ANDROID_HOME \
                     to an existing SDK directory, then check again."
                        .into(),
                ),
            },
        ];
        let (state, _calls) = app_state(cx, &runtime, state);
        let (_page, cx) = cx.add_window_view(|_, cx| DeviceHubPage::new(state, cx));
        cx.simulate_resize(gpui::size(gpui::px(480.0), gpui::px(900.0)));
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
        });
        wait(cx, |cx| {
            cx.debug_bounds("device-platform-android").is_some()
        });
        let refresh = cx
            .debug_bounds("settings-device-platforms-refresh")
            .unwrap();
        let android = cx.debug_bounds("device-platform-android").unwrap();
        assert!(
            android.right() <= refresh.right(),
            "the reason runs past the card: {android:?} vs {refresh:?}"
        );
    }

    fn with_hosts(mut state: DeviceHubState) -> DeviceHubState {
        use zeron_proto::device_hub::{DeviceHostState, DeviceHubHost, DevicePlatformAvailability};
        state.settings.device_hosts = vec![
            SshDeviceHostConfig {
                id: "mini".into(),
                label: "Mac mini".into(),
                target: "me@mini.local".into(),
                ..Default::default()
            },
            SshDeviceHostConfig {
                id: "studio".into(),
                label: "Studio".into(),
                target: "studio".into(),
                port: Some(2222),
                ..Default::default()
            },
        ];
        state.hosts.push(DeviceHubHost {
            id: "mini".into(),
            kind: "ssh".into(),
            label: "Mac mini".into(),
            platforms: vec![DevicePlatformAvailability {
                platform: DevicePlatform::Ios,
                available: true,
                reason: None,
            }],
            ..Default::default()
        });
        state.host_statuses.insert(
            "studio".into(),
            DeviceHostState {
                status: DeviceHostStatus::Failed,
                detail: Some("Host key verification failed.".into()),
            },
        );
        state
    }

    #[gpui::test]
    fn host_rows_test_each_host_and_remove_saves_the_rest(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (state, calls) = crate::device_panel::tests::app_state_with(
            cx,
            &runtime,
            with_hosts(hub(true)),
            std::sync::Arc::new(|method| match method {
                methods::TEST_DEVICE_HOST => serde_json::json!({"local": true}),
                _ => serde_json::json!({}),
            }),
        );
        let (page, cx) = cx.add_window_view(|_, cx| DeviceHubPage::new(state, cx));
        cx.simulate_resize(gpui::size(gpui::px(900.0), gpui::px(2400.0)));
        wait(cx, |cx| {
            cx.debug_bounds("settings-device-host-test-mini").is_some()
        });
        assert!(
            cx.debug_bounds("settings-device-host-remove-studio")
                .is_some()
        );
        click(cx, "settings-device-host-test-mini");
        wait(cx, |_| {
            !calls_of(&calls, methods::TEST_DEVICE_HOST).is_empty()
        });
        assert_eq!(
            calls_of(&calls, methods::TEST_DEVICE_HOST),
            vec![serde_json::json!({"id": "mini", "label": "Mac mini", "target": "me@mini.local"})]
        );
        wait(cx, |cx| {
            page.read_with(cx, |page, _| {
                matches!(page.checks.get("mini"), Some(HostCheck::Done(test)) if test.local)
            })
        });
        wait(cx, |cx| cx.debug_bounds("device-host-check").is_some());
        click(cx, "settings-device-host-remove-mini");
        wait(cx, |_| {
            !calls_of(&calls, methods::CONFIGURE_DEVICE_HUB).is_empty()
        });
        assert_eq!(
            calls_of(&calls, methods::CONFIGURE_DEVICE_HUB),
            vec![serde_json::json!({"deviceHosts": [
                {"id": "studio", "label": "Studio", "target": "studio", "port": 2222}
            ]})]
        );
    }

    #[gpui::test]
    fn the_editor_validates_tests_and_saves_a_new_host(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (state, calls) = crate::device_panel::tests::app_state_with(
            cx,
            &runtime,
            with_hosts(hub(true)),
            std::sync::Arc::new(|method| match method {
                methods::TEST_DEVICE_HOST => serde_json::json!({"checks": [
                    {"name": "SSH", "ok": true},
                    {"name": "Node 22 or newer", "ok": false, "detail": "Node 20.11.0 is installed."}
                ]}),
                _ => serde_json::json!({}),
            }),
        );
        let (page, cx) = cx.add_window_view(|_, cx| DeviceHubPage::new(state, cx));
        cx.simulate_resize(gpui::size(gpui::px(900.0), gpui::px(2400.0)));
        click(cx, "settings-device-host-add");
        wait(cx, |cx| {
            cx.debug_bounds("settings-device-host-save").is_some()
        });
        let type_into = |cx: &mut gpui::VisualTestContext,
                         field: fn(&HostEditor) -> &Entity<ComposerInput>,
                         text: &str| {
            let text = text.to_string();
            page.update(cx, |page, cx| {
                let input = field(page.editor.as_ref().unwrap()).clone();
                input.update(cx, |input, cx| input.set_text(text, cx));
            });
        };
        type_into(cx, |editor| &editor.target, "-oProxyCommand=touch /tmp/x");
        click(cx, "settings-device-host-editor-test");
        cx.run_until_parked();
        assert!(calls_of(&calls, methods::TEST_DEVICE_HOST).is_empty());
        assert!(page.read_with(cx, |page, _| page.editor.as_ref().unwrap().error.is_some()));

        type_into(cx, |editor| &editor.name, "Lab Mac");
        type_into(cx, |editor| &editor.target, "me@lab.local");
        type_into(cx, |editor| &editor.identity, "/Users/me/.ssh/id_ed25519");
        type_into(cx, |editor| &editor.port, "2200");
        click(cx, "settings-device-host-editor-test");
        wait(cx, |_| {
            !calls_of(&calls, methods::TEST_DEVICE_HOST).is_empty()
        });
        let id = page.read_with(cx, |page, _| page.editor.as_ref().unwrap().id.clone());
        assert!(id.starts_with("host-"));
        assert_eq!(
            calls_of(&calls, methods::TEST_DEVICE_HOST),
            vec![
                serde_json::json!({"id": id, "label": "Lab Mac", "target": "me@lab.local",
                "identityFile": "/Users/me/.ssh/id_ed25519", "port": 2200})
            ]
        );
        wait(cx, |cx| {
            page.read_with(cx, |page, _| {
                matches!(page.checks.get(&id), Some(HostCheck::Done(test)) if !test.passed())
            })
        });
        click(cx, "settings-device-host-save");
        wait(cx, |_| {
            !calls_of(&calls, methods::CONFIGURE_DEVICE_HUB).is_empty()
        });
        let saved = &calls_of(&calls, methods::CONFIGURE_DEVICE_HUB)[0]["deviceHosts"];
        assert_eq!(saved.as_array().unwrap().len(), 3);
        assert_eq!(saved[2]["target"], "me@lab.local");
        wait(cx, |cx| page.read_with(cx, |page, _| page.editor.is_none()));
    }
}
