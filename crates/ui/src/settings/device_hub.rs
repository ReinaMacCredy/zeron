use gpui::{
    AnyElement, Context, Entity, SharedString, Subscription, Task, Window, div, prelude::*, px,
};
use zeron_proto::device_hub::{DeviceHubState, DevicePlatform};
use zeron_rpc::methods;

use super::permission_guide::row;
use super::widgets;
use crate::device_panel::setup::{self, SetupPending, activate, button, switch};
use crate::popover;
use crate::settings::{self, SavePolicy};
use crate::state::AppState;
use crate::theme::Theme;

pub const AUTO_FLOAT_DESCRIPTION: &str =
    "Show the floating preview when an agent opens a device unless the agent says otherwise.";
const UPDATE_FAILED: &str = "Update failed. Check this host's network connection and try again.";

pub struct DeviceHubPage {
    state: Entity<AppState>,
    scroll: widgets::PageScroll,
    pending: Option<SetupPending>,
    update_error: Option<SharedString>,
    task: Option<Task<()>>,
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
}
