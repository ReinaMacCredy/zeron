use gpui::{AnyElement, Context, SharedString, Stateful, div, prelude::*, px};
use zeron_proto::device_hub::{DeviceHostStatus, DeviceHubState, DevicePlatform};
use zeron_rpc::methods;

use super::{DevicePanel, DevicePanelEvent};
use crate::icons;
use crate::settings::widgets;
use crate::theme::Theme;

pub const DEVICE_HUB_DESCRIPTION: &str = "Enable this environment to open simulators and emulators, whether they run here or on a remote device host.";
pub const AGENT_DEVICE_DESCRIPTION: &str = "Allow new agent sessions in this environment to start and control local and remote devices, with required tools set up automatically.";
pub const STEPS: [&str; 3] = ["Device hub", "Simulators", "Agent access"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupPending {
    Hub,
    Check,
    Agent,
    Complete,
    Update,
}

fn platform_name(platform: DevicePlatform) -> &'static str {
    match platform {
        DevicePlatform::Ios => "iOS",
        DevicePlatform::Android => "Android",
    }
}

pub fn platform_setup_status(state: &DeviceHubState, platform: DevicePlatform) -> (bool, String) {
    let availability = state
        .hosts
        .iter()
        .flat_map(|host| host.platforms.iter())
        .find(|candidate| candidate.platform == platform);
    if !availability.is_some_and(|availability| availability.available) {
        return (
            false,
            availability
                .and_then(|availability| availability.reason.clone())
                .unwrap_or_else(|| {
                    format!("{} support was not detected.", platform_name(platform))
                }),
        );
    }
    if state.host_status == DeviceHostStatus::Ready
        && !state
            .devices
            .iter()
            .any(|device| device.platform == platform)
    {
        return (
            false,
            match platform {
                DevicePlatform::Ios => "Xcode is installed, but no iOS Simulator is available. Install a runtime in Xcode Settings → Components.",
                DevicePlatform::Android => "The Android SDK is installed, but no virtual device exists. Create one in Android Studio → Device Manager.",
            }
            .into(),
        );
    }
    (
        true,
        match platform {
            DevicePlatform::Ios => "Xcode and iOS Simulator are available.",
            DevicePlatform::Android => "The Android SDK and Emulator are available.",
        }
        .into(),
    )
}

pub fn busy(state: &DeviceHubState) -> bool {
    matches!(
        state.host_status,
        DeviceHostStatus::Installing | DeviceHostStatus::Starting
    )
}

pub fn local_platforms_unavailable(state: &DeviceHubState) -> bool {
    state.hosts.iter().any(|host| {
        host.kind == "local"
            && !host.platforms.is_empty()
            && !host.platforms.iter().any(|platform| platform.available)
    })
}

pub fn hub_setup_status(state: &DeviceHubState, pending: bool, compact: bool) -> Option<String> {
    if pending {
        return Some(
            match (state.host_status, compact) {
                (DeviceHostStatus::Installing, false) => "Installing device hub...",
                (DeviceHostStatus::Installing, true) => "Installing...",
                (DeviceHostStatus::Starting, false) => "Starting device hub...",
                (DeviceHostStatus::Starting, true) => "Starting...",
                (_, false) => "Updating device hub...",
                (_, true) => "Updating...",
            }
            .into(),
        );
    }
    (state.host_status == DeviceHostStatus::Ready).then(|| "Device hub is ready.".into())
}

pub fn agent_setup_status(state: &DeviceHubState, compact: bool) -> &'static str {
    match (state.host_status, compact) {
        (DeviceHostStatus::Installing, false) => "Installing agent tools...",
        (DeviceHostStatus::Installing, true) => "Installing...",
        (DeviceHostStatus::Starting, false) => "Starting agent tools...",
        (DeviceHostStatus::Starting, true) => "Starting...",
        (_, false) => "Updating agent access...",
        (_, true) => "Updating...",
    }
}

pub fn host_update(state: &DeviceHubState) -> Option<(String, String, bool)> {
    let failed = state.host_status == DeviceHostStatus::Failed;
    if !failed && !busy(state) {
        return None;
    }
    let host = state.hosts.first()?;
    let detail = state.host_status_detail.clone().unwrap_or_else(|| {
        match state.host_status {
            DeviceHostStatus::Failed => "Device support could not start.",
            DeviceHostStatus::Installing => "Installing device tools...",
            _ => "Starting device tools...",
        }
        .into()
    });
    Some((host.label.clone(), detail, failed))
}

pub fn switch(
    theme: &Theme,
    id: &'static str,
    label: &'static str,
    on: bool,
) -> Stateful<gpui::Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .flex_none()
        .tab_index(0)
        .role(gpui::Role::Switch)
        .aria_label(label)
        .aria_toggled(if on {
            gpui::Toggled::True
        } else {
            gpui::Toggled::False
        })
        .child(widgets::toggle_switch(theme, on, id))
}

pub fn button(
    theme: &Theme,
    id: &'static str,
    label: impl Into<SharedString>,
) -> Stateful<gpui::Div> {
    let label: SharedString = label.into();
    super::super::settings::permission_guide::outline_button(theme)
        .id(id)
        .debug_selector(move || id.into())
        .tab_index(0)
        .role(gpui::Role::Button)
        .aria_label(label.clone())
        .child(label)
}

pub fn activate<V: 'static>(
    element: Stateful<gpui::Div>,
    enabled: bool,
    cx: &mut Context<V>,
    run: impl Fn(&mut V, &mut Context<V>) + Clone + 'static,
) -> Stateful<gpui::Div> {
    if !enabled {
        return element
            .opacity(0.5)
            .aria_description("Unavailable right now");
    }
    let on_key = run.clone();
    element
        .cursor_pointer()
        .on_click(cx.listener(move |view, _, _, cx| run(view, cx)))
        .on_key_down(cx.listener(move |view, event: &gpui::KeyDownEvent, _, cx| {
            if !event.is_held && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                cx.stop_propagation();
                on_key(view, cx);
            }
        }))
}

pub fn status_line(
    theme: &Theme,
    owner: gpui::EntityId,
    text: String,
    spinner: bool,
    cx: &mut gpui::App,
) -> AnyElement {
    div()
        .id("device-setup-status")
        .flex()
        .items_center()
        .gap(px(8.0))
        .role(gpui::Role::Status)
        .text_size(crate::typography::ui_rems(12.0))
        .text_color(theme.text_muted)
        .map(|el| {
            if spinner {
                el.child(crate::loaders::mini_mono_spinner(
                    "device-setup-spinner",
                    2.0,
                    theme.text_muted,
                    owner,
                    cx,
                ))
            } else {
                el.child(
                    icons::icon(icons::CHECK)
                        .size(px(12.0))
                        .text_color(theme.success),
                )
            }
        })
        .child(SharedString::from(text))
        .into_any_element()
}

pub fn platform_card(
    theme: &Theme,
    platform: DevicePlatform,
    status: (bool, String),
    compact: bool,
) -> AnyElement {
    let (ready, message) = status;
    div()
        .flex()
        .gap(px(8.0))
        .when(!compact, |el| {
            el.rounded(px(6.0))
                .border_1()
                .border_color(theme.border)
                .px(px(12.0))
                .py(px(8.0))
        })
        .child(
            icons::icon(if ready {
                icons::CHECK
            } else {
                icons::INFO_CIRCLE
            })
            .flex_none()
            .mt(px(2.0))
            .size(px(16.0))
            .text_color(if ready {
                theme.success
            } else {
                theme.text_muted
            }),
        )
        .child(
            div()
                .flex()
                .when(compact && ready, |el| el.items_center().gap(px(8.0)))
                .when(!(compact && ready), |el| el.flex_col())
                .child(
                    div()
                        .text_size(crate::typography::ui_rems(13.0))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(theme.text)
                        .child(platform_name(platform)),
                )
                .child(
                    div()
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(if compact && ready {
                            "Ready".to_string()
                        } else {
                            message
                        })),
                ),
        )
        .into_any_element()
}

fn heading(theme: &Theme, text: &'static str) -> gpui::Div {
    div()
        .text_size(crate::typography::ui_rems(13.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(theme.text)
        .child(text)
}

fn muted(theme: &Theme, text: &'static str) -> gpui::Div {
    div()
        .text_size(crate::typography::ui_rems(12.0))
        .text_color(theme.text_muted)
        .child(text)
}

impl DevicePanel {
    pub(super) fn configure(
        &mut self,
        pending: SetupPending,
        params: serde_json::Value,
        cx: &mut Context<Self>,
    ) {
        self.pending = Some(pending);
        self.error = None;
        self.configure_task = self.call(
            methods::CONFIGURE_DEVICE_HUB,
            params,
            cx,
            |panel, reply, _| {
                panel.pending = None;
                if let Err(error) = reply {
                    panel.error = Some(error.into());
                }
            },
        );
        if self.configure_task.is_none() {
            self.pending = None;
        }
    }

    fn check_platforms(&mut self, cx: &mut Context<Self>) {
        self.pending = Some(SetupPending::Check);
        self.list_task = self.call(
            methods::LIST_HUB_DEVICES,
            serde_json::json!({}),
            cx,
            |panel, _, _| {
                panel.pending = None;
            },
        );
    }

    pub(super) fn render_setup(
        &self,
        hub: &DeviceHubState,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let step = self.setup_step;
        let pending = self.pending;
        let enabled = hub.host_status != DeviceHostStatus::Disabled;
        let busy = busy(hub);
        let idle = pending.is_none();
        let owner = cx.entity_id();
        let steps: Vec<_> = STEPS
            .iter()
            .enumerate()
            .map(|(index, label)| {
                let current = index == step;
                let reachable = !busy && idle && index < step;
                let id = SharedString::from(format!("device-setup-step-{index}"));
                div()
                    .id(id)
                    .role(gpui::Role::Tab)
                    .aria_label(*label)
                    .aria_selected(current)
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(div().h(px(3.0)).rounded_full().bg(if index <= step {
                        theme.text.opacity(0.6)
                    } else {
                        crate::theme::ink(0.08)
                    }))
                    .child(
                        div()
                            .text_size(crate::typography::ui_rems(11.0))
                            .text_color(if current {
                                theme.text
                            } else {
                                theme.text_muted
                            })
                            .child(*label),
                    )
                    .when(reachable, |el| {
                        el.cursor_pointer()
                            .on_click(cx.listener(move |panel, _, _, cx| {
                                panel.setup_step = index;
                                cx.notify();
                            }))
                    })
            })
            .collect();
        let mut body = div().flex().flex_col().gap(px(12.0));
        if let Some((label, detail, failed)) = host_update(hub) {
            body = body.child(self.render_host_update(theme, label, detail, failed, cx));
        }
        if step == 0 {
            let hub_switch = activate(
                switch(theme, "device-hub-enable", "Enable device hub", enabled),
                !busy && idle,
                cx,
                move |panel: &mut Self, cx| {
                    let params = if enabled {
                        serde_json::json!({ "enabled": false, "agentAccessEnabled": false })
                    } else {
                        serde_json::json!({ "enabled": true })
                    };
                    panel.configure(SetupPending::Hub, params, cx);
                },
            );
            let status = hub_setup_status(
                hub,
                pending == Some(SetupPending::Hub)
                    || (busy && pending != Some(SetupPending::Agent)),
                false,
            );
            let spinning = pending == Some(SetupPending::Hub) || busy;
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .child(heading(theme, "Enable the device hub"))
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .justify_between()
                            .gap(px(16.0))
                            .child(muted(theme, DEVICE_HUB_DESCRIPTION).flex_1())
                            .child(hub_switch),
                    )
                    .children(status.map(|text| status_line(theme, owner, text, spinning, cx))),
            );
        }
        if step == 1 || (step == 0 && enabled && local_platforms_unavailable(hub)) {
            let checking = pending == Some(SetupPending::Check);
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .when(step == 0, |el| el.mt(px(4.0)))
                    .child(heading(theme, "Check simulator support"))
                    .child(platform_card(theme, DevicePlatform::Ios, platform_setup_status(hub, DevicePlatform::Ios), false))
                    .child(platform_card(theme, DevicePlatform::Android, platform_setup_status(hub, DevicePlatform::Android), false))
                    .child(muted(
                        theme,
                        "You can use either platform. Fixing a missing platform does not block the other one.",
                    ))
                    .child(
                        div().flex().child(activate(
                            button(theme, "device-setup-check", if checking { "Checking..." } else { "Check again" }),
                            enabled && !busy && idle,
                            cx,
                            |panel: &mut Self, cx| panel.check_platforms(cx),
                        )),
                    ),
            );
        }
        if step == 2 {
            let agent = hub.settings.agent_access_enabled;
            let agent_switch = activate(
                switch(
                    theme,
                    "device-agent-access",
                    "Allow agents to control devices",
                    agent,
                ),
                enabled && !busy && idle,
                cx,
                move |panel: &mut Self, cx| {
                    panel.configure(
                        SetupPending::Agent,
                        serde_json::json!({ "agentAccessEnabled": !agent }),
                        cx,
                    );
                },
            );
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .child(heading(theme, "Allow agent control"))
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .justify_between()
                            .gap(px(16.0))
                            .child(muted(theme, AGENT_DEVICE_DESCRIPTION).flex_1())
                            .child(agent_switch),
                    )
                    .when(pending == Some(SetupPending::Agent), |el| {
                        el.child(status_line(theme, owner, agent_setup_status(hub, false).into(), true, cx))
                    })
                    .child(muted(
                        theme,
                        "Leave this off to keep manual device controls without giving agents access.",
                    )),
            );
        }
        if hub.host_status == DeviceHostStatus::Failed
            && let Some(detail) = hub.host_status_detail.clone()
        {
            body = body.child(
                div()
                    .id("device-setup-failed")
                    .role(gpui::Role::Alert)
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.danger)
                    .child(SharedString::from(detail)),
            );
        }
        let ready = hub.host_status == DeviceHostStatus::Ready;
        let back = if step == 0 {
            activate(
                button(theme, "device-setup-cancel", "Cancel"),
                true,
                cx,
                |_: &mut Self, cx| cx.emit(DevicePanelEvent::Close),
            )
        } else {
            activate(
                button(theme, "device-setup-back", "Back"),
                !busy && idle,
                cx,
                |panel: &mut Self, cx| {
                    panel.setup_step -= 1;
                    cx.notify();
                },
            )
        };
        let next = if step < 2 {
            activate(
                button(theme, "device-setup-continue", "Continue"),
                ready && idle,
                cx,
                |panel: &mut Self, cx| {
                    panel.setup_step += 1;
                    cx.notify();
                },
            )
        } else {
            let label = if pending == Some(SetupPending::Complete) {
                "Saving..."
            } else {
                "Done"
            };
            activate(
                button(theme, "device-setup-done", label),
                ready && idle,
                cx,
                |panel: &mut Self, cx| {
                    panel.configure(
                        SetupPending::Complete,
                        serde_json::json!({ "onboardingCompleted": true }),
                        cx,
                    );
                },
            )
        };
        div()
            .id("device-setup")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .items_center()
            .p(px(16.0))
            .child(
                div()
                    .my_auto()
                    .w_full()
                    .max_w(px(440.0))
                    .rounded(px(12.0))
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.surface)
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .p(px(16.0))
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .text_size(crate::typography::ui_rems(14.0))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(theme.text)
                                    .child("Set up devices"),
                            )
                            .child(muted(
                                theme,
                                "Review what runs on this environment before using simulators and emulators.",
                            ))
                            .child(
                                div()
                                    .id("device-setup-steps")
                                    .mt(px(8.0))
                                    .role(gpui::Role::TabList)
                                    .aria_label("Setup steps")
                                    .flex()
                                    .gap(px(8.0))
                                    .children(steps),
                            ),
                    )
                    .child(div().px(px(16.0)).pb(px(16.0)).child(body))
                    .child(
                        div()
                            .border_t_1()
                            .border_color(theme.border)
                            .px(px(16.0))
                            .py(px(12.0))
                            .flex()
                            .justify_end()
                            .gap(px(8.0))
                            .child(back)
                            .child(next),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_host_update(
        &self,
        theme: &Theme,
        label: String,
        detail: String,
        failed: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        host_update_banner(theme, label, detail, failed, |el| {
            activate(el, true, cx, |panel: &mut Self, cx| panel.refresh(cx))
        })
    }
}

pub fn host_update_banner(
    theme: &Theme,
    label: String,
    detail: String,
    failed: bool,
    retry: impl FnOnce(Stateful<gpui::Div>) -> Stateful<gpui::Div>,
) -> AnyElement {
    div()
        .id("device-host-update")
        .role(if failed {
            gpui::Role::Alert
        } else {
            gpui::Role::Status
        })
        .flex()
        .items_start()
        .gap(px(12.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(theme.border)
        .px(px(12.0))
        .py(px(8.0))
        .text_size(crate::typography::ui_rems(12.0))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(theme.text)
                        .child(SharedString::from(label)),
                )
                .child(div().text_color(theme.text_muted).child(SharedString::from(detail)))
                .when(failed, |el| {
                    el.child(div().mt(px(4.0)).text_color(theme.text_muted).child(
                        "Check the host connection and network access, then retry. Your device settings are saved.",
                    ))
                }),
        )
        .when(failed, |el| el.child(retry(button(theme, "device-retry", "Retry"))))
        .into_any_element()
}
