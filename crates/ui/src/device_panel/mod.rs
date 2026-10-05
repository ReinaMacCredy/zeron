pub mod geometry;
pub mod loading;
mod rail;
pub mod setup;
pub mod stream_view;

use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, SharedString, Subscription, Task, Window, div,
    prelude::*, px,
};
use zeron_proto::device_hub::{
    DeviceHostStatus, DeviceHubState, DevicePlatform, DeviceSession, HubDevice,
};
use zeron_rpc::methods;

use crate::icons;
use crate::state::AppState;
use crate::theme::Theme;
use loading::{LoadingStage, LoadingView};
use setup::SetupPending;
use stream_view::DeviceStreamView;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelPhase {
    Setup,
    Loading {
        name: String,
        description: String,
        message: String,
    },
    Stream,
    Picker,
}

pub fn device_description(hub: &DeviceHubState, device: &HubDevice) -> String {
    let label = hub
        .hosts
        .iter()
        .find(|host| host.id == device.host_id)
        .map_or("Device host", |host| host.label.as_str());
    format!("{label} · {}", device.version)
}

pub fn booting_banner(hub: &DeviceHubState, chat_id: &str) -> Option<String> {
    let names: Vec<&str> = hub
        .booting
        .iter()
        .filter(|entry| entry.chat_id == chat_id)
        .map(|entry| entry.device.name.as_str())
        .collect();
    (!names.is_empty()).then(|| format!("Starting {}... This can take a minute.", names.join(", ")))
}

pub fn panel_phase(
    hub: Option<&DeviceHubState>,
    chat_id: &str,
    target: Option<&(String, String)>,
    opening: Option<&str>,
) -> PanelPhase {
    let loading = |name: &str, description: String, message: &str| PanelPhase::Loading {
        name: name.into(),
        description,
        message: message.into(),
    };
    let Some(hub) = hub else {
        return loading("Devices", String::new(), "Finding devices...");
    };
    if !hub.settings.onboarding_completed || hub.host_status == DeviceHostStatus::Disabled {
        return PanelPhase::Setup;
    }
    if let Some((host_id, device_id)) = target
        && hub.session(chat_id, host_id, device_id).is_some()
    {
        return PanelPhase::Stream;
    }
    if let Some(opening) = opening {
        return match hub.devices.iter().find(|device| device.id == opening) {
            Some(device) => loading(
                &device.name,
                device_description(hub, device),
                if device.booted {
                    "Opening device..."
                } else {
                    "Starting device..."
                },
            ),
            None => loading("Devices", String::new(), "Opening device..."),
        };
    }
    match hub.host_status {
        DeviceHostStatus::Installing => loading(
            "Devices",
            String::new(),
            hub.host_status_detail
                .as_deref()
                .unwrap_or("Installing device support..."),
        ),
        DeviceHostStatus::Starting => loading("Devices", String::new(), "Finding devices..."),
        _ => PanelPhase::Picker,
    }
}

pub enum DevicePanelEvent {
    Close,
}

pub struct DevicePanel {
    state: Entity<AppState>,
    chat_id: String,
    target: Option<(String, String)>,
    opening: Option<String>,
    pending: Option<SetupPending>,
    setup_step: usize,
    powering_off: bool,
    listed: bool,
    error: Option<SharedString>,
    view: Option<Entity<DeviceStreamView>>,
    revision: Option<u64>,
    configure_task: Option<Task<()>>,
    list_task: Option<Task<()>>,
    open_task: Option<Task<()>>,
    close_task: Option<Task<()>>,
    more: crate::popover::Popup<()>,
    _state: Subscription,
}

impl EventEmitter<DevicePanelEvent> for DevicePanel {}

impl DevicePanel {
    pub fn new(state: Entity<AppState>, chat_id: String, cx: &mut Context<Self>) -> Self {
        let observer = cx.observe(&state, |panel, _, cx| panel.sync(cx));
        let mut panel = Self {
            state,
            chat_id,
            target: None,
            opening: None,
            pending: None,
            setup_step: 0,
            powering_off: false,
            listed: false,
            error: None,
            view: None,
            revision: None,
            configure_task: None,
            list_task: None,
            open_task: None,
            close_task: None,
            more: Default::default(),
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
                    let link = hub.streams.get(&device_id).cloned();
                    let label = self
                        .target
                        .as_ref()
                        .and_then(|(host, device)| hub.device(host, device))
                        .map(|device| (device.name.clone(), device_description(&hub, device)));
                    view.update(cx, |view, cx| {
                        view.set_screen(screen, cx);
                        view.set_link(link, cx);
                        if let Some((name, description)) = label {
                            view.set_label(name, description, cx);
                        }
                    });
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
    ) -> Option<Task<()>> {
        let engine = self.state.read(cx).engine().cloned()?;
        Some(cx.spawn(async move |this, cx| {
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
        }))
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.list_task = self.call(
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
        self.open_task = self.call(methods::OPEN_HUB_DEVICE, params, cx, |panel, reply, cx| {
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

    fn power_off(&mut self, cx: &mut Context<Self>) {
        let Some((host_id, device_id)) = self.target.clone() else {
            return;
        };
        if self.powering_off {
            return;
        }
        self.error = None;
        let params = serde_json::json!({
            "chatId": self.chat_id,
            "hostId": host_id,
            "deviceId": device_id,
            "shutdown": true,
        });
        self.close_task = self.call(methods::CLOSE_HUB_DEVICE, params, cx, |panel, reply, cx| {
            panel.powering_off = false;
            match reply {
                Ok(_) => {
                    panel.target = None;
                    panel.view = None;
                    panel.revision = None;
                    panel.sync(cx);
                }
                Err(error) => panel.error = Some(error.into()),
            }
        });
        self.powering_off = self.close_task.is_some();
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
        let ready = hub.host_status == DeviceHostStatus::Ready;
        let mut devices: Vec<HubDevice> = hub
            .devices
            .iter()
            .filter(|device| device.platform == DevicePlatform::Ios)
            .cloned()
            .collect();
        devices.sort_by(|a, b| b.booted.cmp(&a.booted).then_with(|| a.name.cmp(&b.name)));
        let empty = if hub.host_status == DeviceHostStatus::Failed {
            hub.host_status_detail
                .clone()
                .unwrap_or_else(|| "The device hub failed to start.".into())
        } else {
            "No simulators or emulators were found on this environment.".into()
        };
        let android_hint = ready
            && !hub
                .devices
                .iter()
                .any(|device| device.platform == DevicePlatform::Android)
            && !hub
                .hosts
                .iter()
                .flat_map(|host| host.platforms.iter())
                .any(|platform| {
                    platform.platform == DevicePlatform::Android && !platform.available
                });
        let rows: Vec<_> = devices
            .into_iter()
            .filter(|_| ready)
            .map(|device| {
                let opening = self.opening.as_deref() == Some(device.id.as_str());
                let status = if device.booted { "Running" } else { "Stopped" };
                let action = if device.booted { "Open" } else { "Start" };
                let detail = format!("{} · {status}", device_description(&hub, &device));
                let row_id = SharedString::from(format!("device-row-{}", device.id));
                let label = SharedString::from(format!("{action} {}", device.name));
                let clicked = device.clone();
                div()
                    .id(row_id)
                    .role(gpui::Role::Button)
                    .aria_label(label)
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
            })
            .collect();
        let small = |text: String| {
            div()
                .text_size(crate::typography::ui_rems(12.0))
                .text_color(theme.text_muted)
                .child(SharedString::from(text))
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(16.0))
            .when(!rows.is_empty(), |el| {
                el.child(
                    div()
                        .text_size(crate::typography::ui_rems(11.0))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(theme.text_muted)
                        .child("iOS Simulators"),
                )
            })
            .when(rows.is_empty(), |el| el.child(small(empty)))
            .children(rows)
            .when(android_hint, |el| {
                el.child(small(
                    "No Android virtual devices found. Create one in Android Studio's Device \
                     Manager, then refresh."
                        .into(),
                ))
            })
            .child(
                div()
                    .id("device-refresh")
                    .role(gpui::Role::Button)
                    .aria_label("Refresh devices")
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

    fn render_banners(
        &self,
        hub: &DeviceHubState,
        theme: &Theme,
        stream: bool,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let banner = |id: &'static str| {
            div()
                .id(id)
                .role(gpui::Role::Status)
                .flex_none()
                .border_b_1()
                .border_color(theme.border)
                .px(px(12.0))
                .py(px(8.0))
                .text_size(crate::typography::ui_rems(12.0))
                .text_color(theme.text_muted)
        };
        let mut banners = Vec::new();
        if hub.host_status == DeviceHostStatus::Ready
            && !stream
            && let Some(detail) = hub.host_status_detail.clone()
        {
            banners.push(
                banner("device-host-detail")
                    .child(SharedString::from(detail))
                    .into_any_element(),
            );
        }
        if let Some((label, detail, failed)) = setup::host_update(hub) {
            banners.push(
                div()
                    .flex_none()
                    .px(px(12.0))
                    .pt(px(8.0))
                    .child(self.render_host_update(theme, label, detail, failed, cx))
                    .into_any_element(),
            );
        }
        if let Some(text) = booting_banner(hub, &self.chat_id) {
            banners.push(
                banner("device-booting")
                    .child(SharedString::from(text))
                    .into_any_element(),
            );
        }
        banners
    }
}

impl Render for DevicePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let hub = self.hub(cx).cloned();
        let phase = panel_phase(
            hub.as_ref(),
            &self.chat_id,
            self.target.as_ref(),
            self.opening.as_deref(),
        );
        let banners = match &hub {
            Some(hub) if phase != PanelPhase::Setup => {
                self.render_banners(hub, &theme, phase == PanelPhase::Stream, cx)
            }
            _ => Vec::new(),
        };
        let body = match phase {
            _ if self.powering_off => {
                self.render_message(&theme, "Powering off device...".into(), true, cx)
            }
            PanelPhase::Setup => self.render_setup(&hub.unwrap_or_default(), &theme, cx),
            PanelPhase::Loading {
                name,
                description,
                message,
            } => loading::render(
                LoadingView {
                    name: name.into(),
                    description: description.into(),
                    message: message.into(),
                    stage: LoadingStage::Opening,
                    error: false,
                },
                &theme,
                cx.entity_id(),
                None,
                cx,
            ),
            PanelPhase::Stream => match self.view.clone() {
                Some(view) => div()
                    .size_full()
                    .relative()
                    .child(view.clone())
                    .child(self.render_rail(&view, &theme, cx))
                    .into_any_element(),
                None => self.render_message(&theme, "Opening device...".into(), true, cx),
            },
            PanelPhase::Picker => self.render_picker(&theme, cx),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .children(banners)
            .when_some(self.error.clone(), |el, error| {
                el.child(
                    div()
                        .id("device-panel-error")
                        .role(gpui::Role::Alert)
                        .flex_none()
                        .flex()
                        .items_start()
                        .gap(px(8.0))
                        .border_b_1()
                        .border_color(theme.border)
                        .px(px(12.0))
                        .py(px(8.0))
                        .bg(theme.danger_muted)
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.danger)
                        .child(div().flex_1().min_w_0().child(error))
                        .child(
                            div()
                                .id("device-error-dismiss")
                                .debug_selector(|| "device-error-dismiss".into())
                                .role(gpui::Role::Button)
                                .aria_label("Dismiss device error")
                                .flex_none()
                                .cursor_pointer()
                                .child(
                                    icons::icon(icons::CLOSE)
                                        .size(px(12.0))
                                        .text_color(theme.danger),
                                )
                                .on_click(cx.listener(|panel, _, _, cx| {
                                    panel.error = None;
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(div().flex_1().min_h_0().child(body))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use gpui::{Modifiers, TestAppContext, VisualTestContext};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use zeron_proto::device_hub::{
        DeviceBooting, DeviceHubSettings, DeviceStreamStatus, StreamLinkStatus,
    };

    pub(crate) type Calls = Arc<Mutex<Vec<(String, serde_json::Value)>>>;

    fn fake_engine(runtime: &tokio::runtime::Runtime) -> (crate::state::EngineHandle, Calls) {
        let (out, mut frames) = tokio::sync::mpsc::channel::<String>(64);
        let (replies, inbound) = tokio::sync::mpsc::channel::<String>(64);
        let calls: Calls = Arc::default();
        let seen = calls.clone();
        runtime.spawn(async move {
            while let Some(frame) = frames.recv().await {
                let frame: serde_json::Value = serde_json::from_str(&frame).unwrap();
                let method = frame["method"].as_str().unwrap_or_default().to_string();
                seen.lock()
                    .unwrap()
                    .push((method.clone(), frame["params"].clone()));
                let delay = if method == methods::CONFIGURE_DEVICE_HUB
                    || method == methods::CLOSE_HUB_DEVICE
                {
                    Duration::from_millis(200)
                } else {
                    Duration::ZERO
                };
                let replies = replies.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(delay).await;
                    let reply = serde_json::json!({ "id": frame["id"], "ok": {} });
                    let _ = replies.send(reply.to_string()).await;
                });
            }
            std::future::pending::<()>().await;
        });
        let client = zeron_rpc::RpcClient::new(out, inbound);
        (crate::state::EngineHandle::from_test_client(client), calls)
    }

    pub(crate) fn app_state(
        cx: &mut TestAppContext,
        runtime: &tokio::runtime::Runtime,
        hub: DeviceHubState,
    ) -> (Entity<AppState>, Calls) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
        });
        cx.executor().allow_parking();
        let (engine, calls) = fake_engine(runtime);
        let state = cx.new(|_| {
            let mut state = AppState::new();
            state.set_test_engine(engine);
            state.device_hub = Some(hub);
            state
        });
        (state, calls)
    }

    pub(crate) fn wait(
        cx: &mut VisualTestContext,
        mut done: impl FnMut(&mut VisualTestContext) -> bool,
    ) {
        for _ in 0..400 {
            cx.run_until_parked();
            if done(cx) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("condition never held");
    }

    pub(crate) fn calls_of(calls: &Calls, method: &str) -> Vec<serde_json::Value> {
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(called, _)| called == method)
            .map(|(_, params)| params.clone())
            .collect()
    }

    fn lists(calls: &Calls) -> usize {
        calls_of(calls, methods::LIST_HUB_DEVICES).len()
    }

    fn streaming(input: StreamLinkStatus) -> DeviceHubState {
        let mut state = hub(true);
        state.sessions.push(DeviceSession {
            chat_id: "chat".into(),
            host_id: "local".into(),
            device_id: "SIM".into(),
            ..Default::default()
        });
        state.streams.insert(
            "SIM".into(),
            DeviceStreamStatus {
                video: StreamLinkStatus::Live,
                input,
                subscribers: 1,
            },
        );
        state
    }

    fn streaming_panel<'a>(
        cx: &'a mut TestAppContext,
        runtime: &tokio::runtime::Runtime,
        input: StreamLinkStatus,
    ) -> (
        Entity<AppState>,
        Entity<DevicePanel>,
        Calls,
        &'a mut VisualTestContext,
    ) {
        let (state, calls) = app_state(cx, runtime, streaming(input));
        let panel_state = state.clone();
        let (panel, cx) = cx.add_window_view(|_, cx| {
            let mut panel = DevicePanel::new(panel_state, "chat".into(), cx);
            panel.target = Some(("local".into(), "SIM".into()));
            panel.revision = None;
            panel.sync(cx);
            panel
        });
        wait(cx, |cx| cx.debug_bounds("device-more").is_some());
        (state, panel, calls, cx)
    }

    pub(crate) fn click(cx: &mut VisualTestContext, selector: &'static str) {
        wait(cx, |cx| cx.debug_bounds(selector).is_some());
        let bounds = cx.debug_bounds(selector).unwrap();
        cx.simulate_click(bounds.center(), Modifiers::default());
    }

    #[gpui::test]
    fn rail_input_buttons_wait_for_live_input_and_close_only_closes_the_tab(
        cx: &mut TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (state, panel, calls, cx) = streaming_panel(cx, &runtime, StreamLinkStatus::Connecting);
        assert!(cx.debug_bounds("Step 2 of 2: connect video").is_some());
        click(cx, "device-home");
        click(cx, "device-rotate");
        cx.run_until_parked();
        assert!(calls_of(&calls, methods::SEND_HUB_DEVICE_INPUT).is_empty());

        state.update(cx, |state, cx| {
            let mut live = streaming(StreamLinkStatus::Live);
            live.revision = 1;
            state.device_hub = Some(live);
            cx.notify();
        });
        click(cx, "device-home");
        click(cx, "device-rotate");
        wait(cx, |_| {
            calls_of(&calls, methods::SEND_HUB_DEVICE_INPUT).len() == 2
        });
        let inputs: Vec<_> = calls_of(&calls, methods::SEND_HUB_DEVICE_INPUT)
            .into_iter()
            .map(|params| params["input"].clone())
            .collect();
        assert_eq!(
            inputs,
            vec![
                serde_json::json!({"button": {"button": "home"}}),
                serde_json::json!({"orientation": {"orientation": "landscape_left"}}),
            ]
        );

        let closed = Arc::new(Mutex::new(false));
        let seen = closed.clone();
        let _subscription = cx.update(|_, cx| {
            cx.subscribe(&panel, move |_, event: &DevicePanelEvent, _| {
                let DevicePanelEvent::Close = event;
                *seen.lock().unwrap() = true;
            })
        });
        click(cx, "device-more");
        click(cx, "device-close-panel");
        wait(cx, |_| *closed.lock().unwrap());
        assert!(calls_of(&calls, methods::CLOSE_HUB_DEVICE).is_empty());
    }

    #[gpui::test]
    fn power_off_closes_the_session_with_shutdown_and_returns_to_the_picker(
        cx: &mut TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (state, panel, calls, cx) = streaming_panel(cx, &runtime, StreamLinkStatus::Live);
        click(cx, "device-more");
        click(cx, "device-power-off");
        wait(cx, |_| {
            !calls_of(&calls, methods::CLOSE_HUB_DEVICE).is_empty()
        });
        assert!(
            cx.debug_bounds("device-more").is_none(),
            "the stream gives way to the powering-off message"
        );
        assert!(panel.read_with(cx, |panel, _| panel.target.is_some()));
        panel.update(cx, |panel, cx| panel.power_off(cx));
        wait(cx, |cx| {
            panel.read_with(cx, |panel, _| panel.target.is_none())
        });
        assert_eq!(
            calls_of(&calls, methods::CLOSE_HUB_DEVICE),
            vec![serde_json::json!({
                "chatId": "chat", "hostId": "local", "deviceId": "SIM", "shutdown": true
            })]
        );
        let phase = panel.read_with(cx, |panel, cx| {
            panel_phase(
                panel.hub(cx),
                &panel.chat_id,
                panel.target.as_ref(),
                panel.opening.as_deref(),
            )
        });
        assert_eq!(phase, PanelPhase::Picker);
        assert!(panel.read_with(cx, |panel, _| panel.stream_view().is_none()));
        assert!(
            state.read_with(cx, |state, _| state.device_hub.is_some()),
            "the hub state is untouched by the UI"
        );
        assert!(
            cx.debug_bounds("device-more").is_none(),
            "the rail left with the stream"
        );
    }

    #[gpui::test]
    fn a_list_started_while_enabling_does_not_drop_the_enable_reply(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (state, calls) = app_state(cx, &runtime, hub(false));
        let (panel, cx) =
            cx.add_window_view(|_, cx| DevicePanel::new(state.clone(), "chat".into(), cx));
        panel.update(cx, |panel, cx| {
            panel.configure(
                SetupPending::Hub,
                serde_json::json!({ "enabled": true }),
                cx,
            )
        });
        state.update(cx, |state, cx| {
            let mut enabled = hub(true);
            enabled.revision = 1;
            state.device_hub = Some(enabled);
            cx.notify();
        });
        wait(cx, |_| lists(&calls) == 1);
        wait(cx, |cx| {
            panel.read_with(cx, |panel, _| panel.pending.is_none())
        });
    }

    #[gpui::test]
    fn retry_in_the_failed_phase_lists_again(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let mut failed = hub(true);
        failed.host_status = DeviceHostStatus::Failed;
        failed.host_status_detail = Some("npm missing".into());
        let (state, calls) = app_state(cx, &runtime, failed);
        let (_panel, cx) = cx.add_window_view(|_, cx| DevicePanel::new(state, "chat".into(), cx));
        wait(cx, |_| lists(&calls) == 1);
        let retry = cx.debug_bounds("device-retry").expect("Retry is shown");
        cx.simulate_click(retry.center(), Modifiers::default());
        wait(cx, |_| lists(&calls) == 2);
    }

    pub(crate) fn hub(enabled: bool) -> DeviceHubState {
        DeviceHubState {
            settings: DeviceHubSettings {
                enabled,
                onboarding_completed: enabled,
                ..Default::default()
            },
            host_status: if enabled {
                DeviceHostStatus::Ready
            } else {
                DeviceHostStatus::Disabled
            },
            hosts: vec![zeron_proto::device_hub::DeviceHubHost {
                id: "local".into(),
                kind: "local".into(),
                label: "This machine".into(),
                ..Default::default()
            }],
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
            PanelPhase::Loading { .. }
        ));
        let mut fresh = hub(true);
        fresh.settings.onboarding_completed = false;
        assert_eq!(
            panel_phase(Some(&fresh), "c", None, None),
            PanelPhase::Setup
        );
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
        state.devices[0].version = "iOS 27.0".into();
        let starting = PanelPhase::Loading {
            name: "iPhone 17".into(),
            description: "This machine · iOS 27.0".into(),
            message: "Starting device...".into(),
        };
        assert_eq!(panel_phase(Some(&state), "c", None, Some("SIM")), starting);
        assert_eq!(
            booting_banner(&state, "c").as_deref(),
            Some("Starting iPhone 17... This can take a minute.")
        );
        assert_eq!(booting_banner(&state, "other"), None);
        state.devices[0].booted = true;
        assert!(matches!(
            panel_phase(Some(&state), "c", None, Some("SIM")),
            PanelPhase::Loading { message, .. } if message == "Opening device..."
        ));
        state.devices[0].booted = false;
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
            PanelPhase::Picker
        );
        assert_eq!(
            panel_phase(Some(&state), "c", Some(&target), None),
            PanelPhase::Stream,
            "a hub failure keeps an open device on its stream"
        );
        state.host_status = DeviceHostStatus::Starting;
        assert!(matches!(
            panel_phase(Some(&state), "other", None, None),
            PanelPhase::Loading { message, .. } if message == "Finding devices..."
        ));
    }

    #[gpui::test]
    fn the_wizard_continues_only_once_the_hub_is_ready_and_done_completes_onboarding(
        cx: &mut TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let mut starting = hub(true);
        starting.settings.onboarding_completed = false;
        starting.host_status = DeviceHostStatus::Starting;
        let (state, calls) = app_state(cx, &runtime, starting);
        let (panel, cx) =
            cx.add_window_view(|_, cx| DevicePanel::new(state.clone(), "chat".into(), cx));
        click(cx, "device-setup-continue");
        cx.run_until_parked();
        assert_eq!(panel.read_with(cx, |panel, _| panel.setup_step), 0);
        assert!(cx.debug_bounds("device-hub-enable").is_some());

        state.update(cx, |state, cx| {
            let mut ready = hub(true);
            ready.settings.onboarding_completed = false;
            ready.revision = 1;
            state.device_hub = Some(ready);
            cx.notify();
        });
        click(cx, "device-setup-continue");
        wait(cx, |cx| {
            panel.read_with(cx, |panel, _| panel.setup_step == 1)
        });
        assert!(cx.debug_bounds("device-setup-check").is_some());
        click(cx, "device-setup-continue");
        wait(cx, |cx| {
            panel.read_with(cx, |panel, _| panel.setup_step == 2)
        });
        click(cx, "device-agent-access");
        wait(cx, |_| {
            !calls_of(&calls, methods::CONFIGURE_DEVICE_HUB).is_empty()
        });
        wait(cx, |cx| {
            panel.read_with(cx, |panel, _| panel.pending.is_none())
        });
        click(cx, "device-setup-done");
        wait(cx, |_| {
            calls_of(&calls, methods::CONFIGURE_DEVICE_HUB).len() == 2
        });
        assert_eq!(
            calls_of(&calls, methods::CONFIGURE_DEVICE_HUB),
            vec![
                serde_json::json!({ "agentAccessEnabled": true }),
                serde_json::json!({ "onboardingCompleted": true }),
            ]
        );
    }

    #[gpui::test]
    fn the_wizard_switch_enables_the_hub_and_cancel_closes_the_tab(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (state, calls) = app_state(cx, &runtime, hub(false));
        let (panel, cx) = cx.add_window_view(|_, cx| DevicePanel::new(state, "chat".into(), cx));
        click(cx, "device-hub-enable");
        wait(cx, |_| {
            !calls_of(&calls, methods::CONFIGURE_DEVICE_HUB).is_empty()
        });
        assert_eq!(
            calls_of(&calls, methods::CONFIGURE_DEVICE_HUB),
            vec![serde_json::json!({ "enabled": true })]
        );
        let closed = Arc::new(Mutex::new(false));
        let seen = closed.clone();
        let _subscription = cx.update(|_, cx| {
            cx.subscribe(&panel, move |_, event: &DevicePanelEvent, _| {
                let DevicePanelEvent::Close = event;
                *seen.lock().unwrap() = true;
            })
        });
        click(cx, "device-setup-cancel");
        wait(cx, |_| *closed.lock().unwrap());
    }

    #[gpui::test]
    fn opening_a_device_shows_the_first_step_of_the_loading_view(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (state, _calls) = app_state(cx, &runtime, hub(true));
        let (panel, cx) = cx.add_window_view(|_, cx| DevicePanel::new(state, "chat".into(), cx));
        panel.update(cx, |panel, cx| {
            panel.opening = Some("SIM".into());
            cx.notify();
        });
        wait(cx, |cx| {
            cx.debug_bounds("Step 1 of 2: open device").is_some()
        });
        assert!(cx.debug_bounds("Step 2 of 2: connect video").is_none());
    }
}
