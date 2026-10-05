pub mod geometry;
mod setup;
pub mod stream_view;

use gpui::{
    AnyElement, App, Context, Entity, SharedString, Subscription, Task, Window, div, prelude::*, px,
};
use zeron_proto::device_hub::{
    DeviceHostStatus, DeviceHubState, DevicePlatform, DeviceSession, HubDevice,
};
use zeron_rpc::methods;

use crate::icons;
use crate::state::AppState;
use crate::theme::Theme;
use stream_view::DeviceStreamView;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelPhase {
    Setup,
    Loading(String),
    Failed(String),
    Stream,
    Picker,
}

pub fn panel_phase(
    hub: Option<&DeviceHubState>,
    chat_id: &str,
    target: Option<&(String, String)>,
    opening: Option<&str>,
) -> PanelPhase {
    let Some(hub) = hub else {
        return PanelPhase::Loading("Finding devices...".into());
    };
    if !hub.settings.enabled {
        return PanelPhase::Setup;
    }
    if let Some((host_id, device_id)) = target
        && hub.session(chat_id, host_id, device_id).is_some()
    {
        return PanelPhase::Stream;
    }
    if let Some(opening) = opening {
        let booting = hub
            .booting
            .iter()
            .find(|entry| entry.device.id == opening && entry.chat_id == chat_id);
        return PanelPhase::Loading(match booting {
            Some(entry) => format!("Starting {}... This can take a minute.", entry.device.name),
            None => "Opening device...".into(),
        });
    }
    match hub.host_status {
        DeviceHostStatus::Installing => PanelPhase::Loading("Installing device support...".into()),
        DeviceHostStatus::Starting => PanelPhase::Loading("Starting device hub...".into()),
        DeviceHostStatus::Failed => PanelPhase::Failed(
            hub.host_status_detail
                .clone()
                .unwrap_or_else(|| "The device hub failed to start.".into()),
        ),
        _ => PanelPhase::Picker,
    }
}

pub struct DevicePanel {
    state: Entity<AppState>,
    chat_id: String,
    target: Option<(String, String)>,
    opening: Option<String>,
    enabling: bool,
    listed: bool,
    error: Option<SharedString>,
    view: Option<Entity<DeviceStreamView>>,
    revision: Option<u64>,
    task: Option<Task<()>>,
    _state: Subscription,
}

impl DevicePanel {
    pub fn new(state: Entity<AppState>, chat_id: String, cx: &mut Context<Self>) -> Self {
        let observer = cx.observe(&state, |panel, _, cx| panel.sync(cx));
        let mut panel = Self {
            state,
            chat_id,
            target: None,
            opening: None,
            enabling: false,
            listed: false,
            error: None,
            view: None,
            revision: None,
            task: None,
            _state: observer,
        };
        panel.sync(cx);
        panel
    }

    fn hub<'a>(&self, cx: &'a App) -> Option<&'a DeviceHubState> {
        self.state.read(cx).device_hub.as_ref()
    }

    pub fn title(&self, cx: &App) -> SharedString {
        self.target
            .as_ref()
            .and_then(|(host, device)| self.hub(cx)?.device(host, device))
            .map(|device| SharedString::from(device.name.clone()))
            .unwrap_or_else(|| "Device".into())
    }

    pub fn stream_view(&self) -> Option<&Entity<DeviceStreamView>> {
        self.view.as_ref()
    }

    fn sync(&mut self, cx: &mut Context<Self>) {
        let Some(hub) = self.hub(cx).cloned() else {
            return;
        };
        if self.revision == Some(hub.revision) {
            return;
        }
        self.revision = Some(hub.revision);
        if hub.settings.enabled && !self.listed {
            self.listed = true;
            self.refresh(cx);
        }
        let live = self
            .target
            .as_ref()
            .and_then(|(host, device)| hub.session(&self.chat_id, host, device))
            .map(|session| session.device_id.clone());
        match live {
            Some(device_id) => {
                if self.view.is_none()
                    && let Some(engine) = self.state.read(cx).engine().cloned()
                {
                    let id = device_id.clone();
                    self.view = Some(cx.new(|cx| DeviceStreamView::new(engine, id, cx)));
                }
                if let Some(view) = &self.view {
                    let screen = hub.screens.get(&device_id).cloned();
                    view.update(cx, |view, cx| view.set_screen(screen, cx));
                }
            }
            None => self.view = None,
        }
        cx.notify();
    }

    fn call(
        &mut self,
        method: &'static str,
        params: serde_json::Value,
        cx: &mut Context<Self>,
        done: impl FnOnce(&mut Self, Result<serde_json::Value, String>, &mut Context<Self>) + 'static,
    ) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.task = Some(cx.spawn(async move |this, cx| {
            let reply = engine
                .client()
                .call(method, params)
                .await
                .map_err(|error| error.to_string());
            this.update(cx, |panel, cx| {
                done(panel, reply, cx);
                cx.notify();
            })
            .ok();
        }));
    }

    fn enable(&mut self, cx: &mut Context<Self>) {
        self.enabling = true;
        self.error = None;
        let params = serde_json::json!({ "enabled": true, "onboardingCompleted": true });
        self.call(
            methods::CONFIGURE_DEVICE_HUB,
            params,
            cx,
            |panel, reply, _| {
                panel.enabling = false;
                if let Err(error) = reply {
                    panel.error = Some(error.into());
                }
            },
        );
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.call(
            methods::LIST_HUB_DEVICES,
            serde_json::json!({}),
            cx,
            |panel, reply, _| {
                if let Err(error) = reply {
                    panel.error = Some(error.into());
                }
            },
        );
    }

    fn open_device(&mut self, device: HubDevice, cx: &mut Context<Self>) {
        if self.opening.is_some() {
            return;
        }
        self.opening = Some(device.id.clone());
        self.error = None;
        let params = serde_json::json!({
            "chatId": self.chat_id,
            "hostId": device.host_id,
            "deviceId": device.id,
            "platform": device.platform,
        });
        self.call(methods::OPEN_HUB_DEVICE, params, cx, |panel, reply, cx| {
            panel.opening = None;
            match reply.and_then(|value| {
                serde_json::from_value::<DeviceSession>(value).map_err(|e| e.to_string())
            }) {
                Ok(session) => {
                    panel.target = Some((session.host_id, session.device_id));
                    panel.revision = None;
                    panel.sync(cx);
                }
                Err(error) => panel.error = Some(error.into()),
            }
        });
    }

    fn render_message(
        &self,
        theme: &Theme,
        text: String,
        loading: bool,
        cx: &mut App,
    ) -> AnyElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(10.0))
            .p(px(16.0))
            .when(loading, |el| {
                el.child(crate::loaders::mini_mono_spinner(
                    "device-panel-spinner",
                    2.0,
                    theme.text_muted,
                    self.state.entity_id(),
                    cx,
                ))
            })
            .child(
                div()
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(text)),
            )
            .into_any_element()
    }

    fn render_picker(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let hub = self.hub(cx).cloned().unwrap_or_default();
        let mut devices: Vec<HubDevice> = hub
            .devices
            .iter()
            .filter(|device| device.platform == DevicePlatform::Ios)
            .cloned()
            .collect();
        devices.sort_by(|a, b| b.booted.cmp(&a.booted).then_with(|| a.name.cmp(&b.name)));
        let ios_reason = hub
            .hosts
            .iter()
            .flat_map(|host| host.platforms.iter())
            .find(|platform| platform.platform == DevicePlatform::Ios && !platform.available)
            .and_then(|platform| platform.reason.clone());
        let empty = match &ios_reason {
            Some(reason) => reason.clone(),
            None => "Xcode is installed, but no iOS Simulator is available. Install a runtime in \
                     Xcode Settings → Components."
                .into(),
        };
        let rows = devices.into_iter().map(|device| {
            let opening = self.opening.as_deref() == Some(device.id.as_str());
            let status = if device.booted { "Running" } else { "Stopped" };
            let action = if device.booted { "Open" } else { "Start" };
            let detail = format!("{} · {status}", device.version);
            let row_id = SharedString::from(format!("device-row-{}", device.id));
            let clicked = device.clone();
            div()
                .id(row_id)
                .w_full()
                .px(px(12.0))
                .py(px(8.0))
                .rounded(px(10.0))
                .border_1()
                .border_color(theme.border)
                .bg(crate::theme::ink(0.02))
                .flex()
                .items_center()
                .gap(px(10.0))
                .cursor_pointer()
                .hover(|s| s.bg(crate::theme::ink(0.05)))
                .child(
                    icons::icon(icons::SMARTPHONE)
                        .size(px(15.0))
                        .flex_none()
                        .text_color(theme.text_muted),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(crate::typography::ui_rems(13.0))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(theme.text)
                                .truncate()
                                .child(SharedString::from(device.name.clone())),
                        )
                        .child(
                            div()
                                .text_size(crate::typography::ui_rems(11.0))
                                .text_color(theme.text_muted)
                                .truncate()
                                .child(SharedString::from(detail)),
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.text)
                        .child(if opening { "Opening..." } else { action }),
                )
                .on_click(cx.listener(move |panel, _, _, cx| {
                    panel.open_device(clicked.clone(), cx);
                }))
        });
        let rows: Vec<_> = rows.collect();
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(16.0))
            .child(
                div()
                    .text_size(crate::typography::ui_rems(11.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text_muted)
                    .child("iOS Simulators"),
            )
            .when(rows.is_empty(), |el| {
                el.child(
                    div()
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(empty)),
                )
            })
            .children(rows)
            .child(
                div()
                    .id("device-refresh")
                    .mt(px(4.0))
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .cursor_pointer()
                    .hover(|s| s.text_color(theme.text))
                    .child("Refresh devices")
                    .on_click(cx.listener(|panel, _, _, cx| panel.refresh(cx))),
            )
            .into_any_element()
    }
}

impl Render for DevicePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let phase = panel_phase(
            self.hub(cx),
            &self.chat_id,
            self.target.as_ref(),
            self.opening.as_deref(),
        );
        let body = match phase {
            PanelPhase::Setup => setup::render_gate(
                &theme,
                self.enabling,
                cx.listener(|panel, _, _, cx| panel.enable(cx)),
            ),
            PanelPhase::Loading(text) => self.render_message(&theme, text, true, cx),
            PanelPhase::Failed(text) => self.render_message(&theme, text, false, cx),
            PanelPhase::Stream => match &self.view {
                Some(view) => view.clone().into_any_element(),
                None => self.render_message(&theme, "Opening device...".into(), true, cx),
            },
            PanelPhase::Picker => self.render_picker(&theme, cx),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .when_some(self.error.clone(), |el, error| {
                el.child(
                    div()
                        .id("device-panel-error")
                        .flex_none()
                        .mx(px(12.0))
                        .mt(px(10.0))
                        .px(px(10.0))
                        .py(px(6.0))
                        .rounded(px(8.0))
                        .bg(theme.danger_muted)
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.danger)
                        .cursor_pointer()
                        .child(error)
                        .on_click(cx.listener(|panel, _, _, cx| {
                            panel.error = None;
                            cx.notify();
                        })),
                )
            })
            .child(div().flex_1().min_h_0().child(body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeron_proto::device_hub::{DeviceBooting, DeviceHubSettings};

    fn hub(enabled: bool) -> DeviceHubState {
        DeviceHubState {
            settings: DeviceHubSettings {
                enabled,
                ..Default::default()
            },
            host_status: DeviceHostStatus::Ready,
            devices: vec![HubDevice {
                host_id: "local".into(),
                id: "SIM".into(),
                name: "iPhone 17".into(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn the_gate_shows_until_the_hub_is_enabled() {
        assert_eq!(
            panel_phase(Some(&hub(false)), "c", None, None),
            PanelPhase::Setup
        );
        assert_eq!(
            panel_phase(Some(&hub(true)), "c", None, None),
            PanelPhase::Picker
        );
        assert!(matches!(
            panel_phase(None, "c", None, None),
            PanelPhase::Loading(_)
        ));
    }

    #[test]
    fn a_session_of_this_chat_shows_the_stream_and_boots_show_their_copy() {
        let mut state = hub(true);
        let target = ("local".to_string(), "SIM".to_string());
        assert_eq!(
            panel_phase(Some(&state), "c", Some(&target), None),
            PanelPhase::Picker
        );
        state.booting.push(DeviceBooting {
            device: state.devices[0].clone(),
            chat_id: "c".into(),
        });
        assert_eq!(
            panel_phase(Some(&state), "c", None, Some("SIM")),
            PanelPhase::Loading("Starting iPhone 17... This can take a minute.".into())
        );
        assert_eq!(
            panel_phase(Some(&state), "other", None, Some("SIM")),
            PanelPhase::Loading("Opening device...".into())
        );
        state.sessions.push(DeviceSession {
            chat_id: "c".into(),
            host_id: "local".into(),
            device_id: "SIM".into(),
            ..Default::default()
        });
        assert_eq!(
            panel_phase(Some(&state), "c", Some(&target), None),
            PanelPhase::Stream
        );
        assert_eq!(
            panel_phase(Some(&state), "other", Some(&target), None),
            PanelPhase::Picker
        );
        state.host_status = DeviceHostStatus::Failed;
        state.host_status_detail = Some("npm missing".into());
        assert_eq!(
            panel_phase(Some(&state), "other", None, None),
            PanelPhase::Failed("npm missing".into())
        );
    }
}
