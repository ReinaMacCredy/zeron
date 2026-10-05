pub mod float;
pub mod geometry;
pub mod h264;
pub mod loading;
pub mod presentation;
mod rail;
pub mod setup;
pub mod stream_view;
pub mod tools;

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, Focusable, SharedString, Subscription, Task,
    Window, div, prelude::*, px,
};
use zeron_proto::device_hub::{
    DeviceHostStatus, DeviceHubState, DevicePlatform, DeviceSession, HubDevice, HubDeviceScreenshot,
};
use zeron_rpc::methods;

use crate::composer::{ComposerInput, ComposerInputEvent};
use crate::icons;
use crate::state::AppState;
use crate::theme::Theme;
use loading::{LoadingStage, LoadingView};
use setup::SetupPending;
use stream_view::DeviceStreamView;
use tools::DeviceControls;

const TOAST_FOR: std::time::Duration = std::time::Duration::from_secs(4);

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

pub(crate) fn feed_view(
    view: &Entity<DeviceStreamView>,
    hub: &DeviceHubState,
    host_id: &str,
    device_id: &str,
    cx: &mut App,
) {
    let screen = hub.screens.get(device_id).cloned();
    let link = hub.streams.get(device_id).cloned();
    let label = hub
        .device(host_id, device_id)
        .map(|device| (device.name.clone(), device_description(hub, device)));
    view.update(cx, |view, cx| {
        view.set_screen(screen, cx);
        view.set_link(link, cx);
        if let Some((name, description)) = label {
            view.set_label(name, description, cx);
        }
    });
}

pub fn picker_groups(hub: &DeviceHubState) -> Vec<(DevicePlatform, &'static str, Vec<HubDevice>)> {
    [
        (DevicePlatform::Ios, "iOS Simulators"),
        (DevicePlatform::Android, "Android Emulators"),
    ]
    .into_iter()
    .map(|(platform, title)| {
        let mut devices: Vec<HubDevice> = hub
            .devices
            .iter()
            .filter(|device| device.platform == platform)
            .cloned()
            .collect();
        devices.sort_by(|a, b| b.booted.cmp(&a.booted).then_with(|| a.name.cmp(&b.name)));
        (platform, title, devices)
    })
    .filter(|(_, _, devices)| !devices.is_empty())
    .collect()
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
    Bound,
    Float,
}

struct TabRename {
    input: Entity<ComposerInput>,
    initial: String,
    _subscriptions: [Subscription; 2],
}

pub struct DevicePanel {
    state: Entity<AppState>,
    chat_id: String,
    target: Option<(String, String)>,
    custom_title: Option<String>,
    rename: Option<TabRename>,
    visible: bool,
    seen_live: bool,
    opening: Option<String>,
    pending: Option<SetupPending>,
    setup_step: usize,
    powering_off: bool,
    listed: bool,
    error: Option<SharedString>,
    view: Option<Entity<DeviceStreamView>>,
    controls: Option<(Entity<DeviceControls>, Subscription)>,
    panel_width: Rc<Cell<f32>>,
    screenshot_task: Option<Task<()>>,
    toast: Option<(u64, SharedString)>,
    revision: Option<u64>,
    configure_task: Option<Task<()>>,
    list_task: Option<Task<()>>,
    open_task: Option<Task<()>>,
    close_task: Option<Task<()>>,
    more: crate::popover::Popup<()>,
    text_size: crate::popover::Popup<()>,
    rotate: crate::popover::Popup<()>,
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
            custom_title: None,
            rename: None,
            visible: true,
            seen_live: false,
            opening: None,
            pending: None,
            setup_step: 0,
            powering_off: false,
            listed: false,
            error: None,
            view: None,
            controls: None,
            panel_width: Rc::default(),
            screenshot_task: None,
            toast: None,
            revision: None,
            configure_task: None,
            list_task: None,
            open_task: None,
            close_task: None,
            more: Default::default(),
            text_size: Default::default(),
            rotate: Default::default(),
            _state: observer,
        };
        panel.sync(cx);
        panel
    }

    fn hub<'a>(&self, cx: &'a App) -> Option<&'a DeviceHubState> {
        self.state.read(cx).device_hub.as_ref()
    }

    fn device<'a>(&self, cx: &'a App) -> Option<&'a HubDevice> {
        let (host, device) = self.target.as_ref()?;
        self.hub(cx)?.device(host, device)
    }

    pub fn title(&self, cx: &App) -> SharedString {
        if let Some(title) = &self.custom_title {
            return title.clone().into();
        }
        self.device(cx)
            .map(|device| SharedString::from(device.name.clone()))
            .unwrap_or_else(|| "Device".into())
    }

    pub fn platform(&self, cx: &App) -> Option<DevicePlatform> {
        self.device(cx).map(|device| device.platform)
    }

    pub fn description(&self, cx: &App) -> Option<String> {
        Some(device_description(self.hub(cx)?, self.device(cx)?))
    }

    pub fn chat_id(&self) -> &str {
        &self.chat_id
    }

    pub fn target(&self) -> Option<&(String, String)> {
        self.target.as_ref()
    }

    pub fn is_idle_picker(&self) -> bool {
        self.target.is_none() && self.opening.is_none()
    }

    pub fn shows(&self, target: &(String, String)) -> bool {
        self.target.as_ref() == Some(target) || self.opening.as_deref() == Some(target.1.as_str())
    }

    pub fn bind(&mut self, host_id: String, device_id: String, cx: &mut Context<Self>) {
        let target = Some((host_id, device_id));
        if self.target != target {
            self.custom_title = None;
        }
        self.target = target;
        self.seen_live = false;
        self.drop_view();
        self.revision = None;
        self.sync(cx);
    }

    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        if visible {
            self.revision = None;
            self.sync(cx);
        } else {
            self.drop_view();
            cx.notify();
        }
    }

    fn drop_view(&mut self) {
        self.view = None;
        self.controls = None;
        self.text_size = Default::default();
        self.rotate = Default::default();
    }

    pub fn controls(&self) -> Option<&Entity<DeviceControls>> {
        self.controls.as_ref().map(|(controls, _)| controls)
    }

    pub fn rename_input(&self) -> Option<Entity<ComposerInput>> {
        self.rename.as_ref().map(|rename| rename.input.clone())
    }

    pub fn begin_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.rename.is_some() || self.target.is_none() {
            return;
        }
        let title = self.title(cx).to_string();
        let input = cx.new(|cx| {
            ComposerInput::new("Device tab name", cx)
                .with_single_line()
                .with_text_metrics(11.5, 16.0)
                .with_accessibility_role(gpui::Role::TextInput)
        });
        input.update(cx, |input, cx| {
            input.set_text(title.clone(), cx);
            input.select_all_text(cx);
        });
        let submit = cx.subscribe(&input, |panel, _, event, cx| {
            if matches!(event, ComposerInputEvent::Submitted) {
                panel.finish_rename(true, cx);
            }
        });
        let focus = input.focus_handle(cx);
        window.focus(&focus, cx);
        let blur = cx.on_blur(&focus, window, |panel, _, cx| {
            panel.finish_rename(true, cx);
        });
        self.rename = Some(TabRename {
            input,
            initial: title,
            _subscriptions: [submit, blur],
        });
        cx.notify();
    }

    pub fn finish_rename(&mut self, commit: bool, cx: &mut Context<Self>) -> bool {
        let Some(rename) = self.rename.take() else {
            return false;
        };
        if commit {
            let text = rename.input.read(cx).text().trim().to_string();
            if text != rename.initial {
                self.custom_title = (!text.is_empty()).then_some(text);
            }
        }
        cx.notify();
        true
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
            .map(|session| (session.device_id.clone(), session.platform));
        match live {
            Some((device_id, platform)) => {
                self.seen_live = true;
                if self.view.is_none()
                    && self.visible
                    && let Some(engine) = self.state.read(cx).engine().cloned()
                {
                    let id = device_id.clone();
                    let physical = self.device(cx).is_some_and(|device| device.physical);
                    let view = cx
                        .new(|cx| DeviceStreamView::new(engine.clone(), id.clone(), platform, cx));
                    let host = self.target.as_ref().map(|(host, _)| host.clone());
                    let weak = view.downgrade();
                    let controls = cx.new(|cx| {
                        DeviceControls::new(
                            engine,
                            host.unwrap_or_default(),
                            id,
                            platform,
                            physical,
                            weak,
                            cx,
                        )
                    });
                    let observer = cx.observe(&controls, |_, _, cx| cx.notify());
                    self.controls = Some((controls, observer));
                    self.view = Some(view);
                }
                if let (Some(view), Some((host, _))) = (&self.view, &self.target) {
                    feed_view(view, &hub, host, &device_id, cx);
                }
            }
            None => {
                self.drop_view();
                if self.opening.is_none() && std::mem::take(&mut self.seen_live) {
                    self.target = None;
                    self.custom_title = None;
                }
            }
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

    pub(crate) fn open_device(&mut self, device: HubDevice, cx: &mut Context<Self>) {
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
                    panel.bind(session.host_id, session.device_id, cx);
                    cx.emit(DevicePanelEvent::Bound);
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
                    panel.custom_title = None;
                    panel.drop_view();
                    panel.revision = None;
                    panel.sync(cx);
                }
                Err(error) => panel.error = Some(error.into()),
            }
        });
        self.powering_off = self.close_task.is_some();
    }

    fn save_screenshot(&mut self, cx: &mut Context<Self>) {
        let Some((host_id, device_id)) = self.target.clone() else {
            return;
        };
        if self.screenshot_task.is_some() {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let params = serde_json::json!({ "hostId": host_id, "deviceId": device_id });
        self.error = None;
        self.screenshot_task = Some(cx.spawn(async move |this, cx| {
            let saved = engine
                .client()
                .call(methods::HUB_DEVICE_SCREENSHOT, params)
                .await
                .map_err(|error| error.to_string())
                .and_then(|reply| {
                    serde_json::from_value::<HubDeviceScreenshot>(reply).map_err(|e| e.to_string())
                });
            this.update(cx, |panel, cx| {
                panel.screenshot_task = None;
                match saved {
                    Ok(shot) => panel.show_toast(format!("Saved screenshot to {}", shot.path), cx),
                    Err(error) => panel.error = Some(error.into()),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn show_toast(&mut self, text: String, cx: &mut Context<Self>) {
        let id = self.toast.as_ref().map_or(0, |(id, _)| id + 1);
        self.toast = Some((id, text.into()));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TOAST_FOR).await;
            this.update(cx, |panel, cx| {
                if panel.toast.as_ref().is_some_and(|(shown, _)| *shown == id) {
                    panel.toast = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn render_stream(
        &self,
        view: Entity<DeviceStreamView>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let docked = self.panel_width.get() >= tools::DOCK_MIN_WIDTH;
        let drawer = self
            .controls()
            .filter(|controls| controls.read(cx).drawer_open())
            .cloned();
        let width = self.panel_width.clone();
        let panel = cx.entity().downgrade();
        let measure = gpui::canvas(
            move |bounds, _, cx| {
                let now = f32::from(bounds.size.width);
                let was = width.replace(now);
                if (was >= tools::DOCK_MIN_WIDTH) != (now >= tools::DOCK_MIN_WIDTH) {
                    panel.update(cx, |_, cx| cx.notify()).ok();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0();
        let toast = self.toast.as_ref().map(|(_, text)| {
            div()
                .absolute()
                .left_0()
                .right(px(geometry::RAIL_WIDTH))
                .top(px(8.0))
                .flex()
                .justify_center()
                .child(
                    div()
                        .id("device-toast")
                        .debug_selector(|| "device-toast".into())
                        .role(gpui::Role::Status)
                        .max_w_full()
                        .rounded(px(6.0))
                        .bg(theme.bg.opacity(0.92))
                        .border_1()
                        .border_color(theme.border)
                        .px(px(8.0))
                        .py(px(4.0))
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.text)
                        .child(text.clone()),
                )
        });
        let screen = div()
            .id("device-screen-area")
            .debug_selector(|| "device-screen-area".into())
            .flex_1()
            .min_w_0()
            .h_full()
            .relative()
            .child(view.clone())
            .child(self.render_rail(&view, theme, cx))
            .children(toast)
            .when_some(drawer.clone().filter(|_| !docked), |el, drawer| {
                el.child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .right_0()
                        .w_full()
                        .max_w(px(tools::DRAWER_WIDTH))
                        .child(drawer),
                )
            });
        div()
            .size_full()
            .relative()
            .flex()
            .child(measure)
            .child(screen)
            .when_some(drawer.filter(|_| docked), |el, drawer| {
                el.child(
                    div()
                        .flex_none()
                        .w(px(tools::DRAWER_WIDTH))
                        .h_full()
                        .child(drawer),
                )
            })
            .into_any_element()
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
        let groups = if ready {
            picker_groups(&hub)
        } else {
            Vec::new()
        };
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
        let row = |device: HubDevice, cx: &mut Context<Self>| {
            let opening = self.opening.as_deref() == Some(device.id.as_str());
            let status = if device.booted { "Running" } else { "Stopped" };
            let action = if device.booted { "Open" } else { "Start" };
            let detail = format!("{} · {status}", device_description(&hub, &device));
            let row_id = SharedString::from(format!("device-row-{}", device.id));
            let selector = row_id.clone();
            let label = SharedString::from(format!("{action} {}", device.name));
            let clicked = device.clone();
            div()
                .id(row_id)
                .debug_selector(move || selector.to_string())
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
        };
        let small = |text: String| {
            div()
                .text_size(crate::typography::ui_rems(12.0))
                .text_color(theme.text_muted)
                .child(SharedString::from(text))
        };
        let empty_list = groups.is_empty();
        let mut sections = Vec::new();
        for (platform, title, devices) in groups {
            let id = match platform {
                DevicePlatform::Ios => "device-group-ios",
                DevicePlatform::Android => "device-group-android",
            };
            sections.push(
                div()
                    .id(id)
                    .debug_selector(move || id.into())
                    .text_size(crate::typography::ui_rems(11.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text_muted)
                    .child(title)
                    .into_any_element(),
            );
            sections.extend(
                devices
                    .into_iter()
                    .map(|device| row(device, cx).into_any_element()),
            );
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(16.0))
            .when(empty_list, |el| el.child(small(empty)))
            .children(sections)
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
                Some(view) => self.render_stream(view, &theme, cx),
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

    const CANCEL: &str = "cancel";

    pub(crate) fn open_frame_streams(calls: &Calls) -> usize {
        calls_of(calls, methods::WATCH_HUB_DEVICE_FRAMES).len() - calls_of(calls, CANCEL).len()
    }

    pub(crate) type Reply = Arc<dyn Fn(&str) -> serde_json::Value + Send + Sync>;

    pub(crate) fn fake_engine(
        runtime: &tokio::runtime::Runtime,
    ) -> (crate::state::EngineHandle, Calls) {
        fake_engine_with(runtime, Arc::new(|_| serde_json::json!({})))
    }

    pub(crate) fn fake_engine_with(
        runtime: &tokio::runtime::Runtime,
        reply: Reply,
    ) -> (crate::state::EngineHandle, Calls) {
        let (out, mut frames) = tokio::sync::mpsc::channel::<String>(64);
        let (replies, inbound) = tokio::sync::mpsc::channel::<String>(64);
        let calls: Calls = Arc::default();
        let seen = calls.clone();
        runtime.spawn(async move {
            while let Some(frame) = frames.recv().await {
                let frame: serde_json::Value = serde_json::from_str(&frame).unwrap();
                let method = match frame["method"].as_str() {
                    Some(method) => method.to_string(),
                    None if frame["cancel"] == true => CANCEL.to_string(),
                    None => String::new(),
                };
                seen.lock()
                    .unwrap()
                    .push((method.clone(), frame["params"].clone()));
                if matches!(
                    method.as_str(),
                    methods::WATCH_HUB_DEVICE_FRAMES
                        | methods::WATCH_HUB_DEVICE_APP_STATE
                        | methods::WATCH_HUB_DEVICE_EVENTS
                        | CANCEL
                ) {
                    continue;
                }
                let delay = if method == methods::CONFIGURE_DEVICE_HUB
                    || method == methods::CLOSE_HUB_DEVICE
                {
                    Duration::from_millis(200)
                } else {
                    Duration::ZERO
                };
                let replies = replies.clone();
                let ok = reply(&method);
                tokio::spawn(async move {
                    tokio::time::sleep(delay).await;
                    let reply = serde_json::json!({ "id": frame["id"], "ok": ok });
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
        app_state_with(cx, runtime, hub, Arc::new(|_| serde_json::json!({})))
    }

    pub(crate) fn app_state_with(
        cx: &mut TestAppContext,
        runtime: &tokio::runtime::Runtime,
        hub: DeviceHubState,
        reply: Reply,
    ) -> (Entity<AppState>, Calls) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
        });
        cx.executor().allow_parking();
        let (engine, calls) = fake_engine_with(runtime, reply);
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

    pub(crate) fn streaming(input: StreamLinkStatus) -> DeviceHubState {
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
                avcc: StreamLinkStatus::Live,
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
                if let DevicePanelEvent::Close = event {
                    *seen.lock().unwrap() = true;
                }
            })
        });
        click(cx, "device-more");
        click(cx, "device-close-panel");
        wait(cx, |_| *closed.lock().unwrap());
        assert!(calls_of(&calls, methods::CLOSE_HUB_DEVICE).is_empty());
    }

    fn android(mut state: DeviceHubState) -> DeviceHubState {
        state.devices[0].platform = DevicePlatform::Android;
        state.devices[0].name = "Medium_Phone_API_36.1".into();
        for session in &mut state.sessions {
            session.platform = DevicePlatform::Android;
        }
        state
    }

    fn android_panel<'a>(
        cx: &'a mut TestAppContext,
        runtime: &tokio::runtime::Runtime,
        reply: Reply,
    ) -> (Entity<DevicePanel>, Calls, &'a mut VisualTestContext) {
        let (state, calls) = app_state_with(
            cx,
            runtime,
            android(streaming(StreamLinkStatus::Live)),
            reply,
        );
        let (panel, cx) = cx.add_window_view(|_, cx| {
            let mut panel = DevicePanel::new(state, "chat".into(), cx);
            panel.target = Some(("local".into(), "SIM".into()));
            panel.revision = None;
            panel.sync(cx);
            panel
        });
        wait(cx, |cx| cx.debug_bounds("device-more").is_some());
        (panel, calls, cx)
    }

    #[test]
    fn the_picker_groups_simulators_then_emulators_with_running_devices_first() {
        let mut state = hub(true);
        let device = |id: &str, name: &str, platform, booted| HubDevice {
            host_id: "local".into(),
            id: id.into(),
            name: name.into(),
            platform,
            booted,
            ..Default::default()
        };
        state.devices.extend([
            device("Pixel_9", "Pixel_9", DevicePlatform::Android, false),
            device(
                "emulator-5554",
                "Medium_Phone",
                DevicePlatform::Android,
                true,
            ),
            device("PAD", "iPad", DevicePlatform::Ios, true),
        ]);
        let shape = |state: &DeviceHubState| {
            picker_groups(state)
                .into_iter()
                .map(|(_, title, devices)| {
                    (
                        title,
                        devices
                            .into_iter()
                            .map(|device| device.id)
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            shape(&state),
            [
                ("iOS Simulators", vec!["PAD".to_string(), "SIM".to_string()]),
                (
                    "Android Emulators",
                    vec!["emulator-5554".to_string(), "Pixel_9".to_string()]
                ),
            ]
        );
        state.devices.truncate(1);
        assert_eq!(shape(&state), [("iOS Simulators", vec!["SIM".to_string()])]);
    }

    #[gpui::test]
    fn an_avd_row_under_android_emulators_starts_it_with_the_android_platform(
        cx: &mut TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let mut state = hub(true);
        state.devices.push(HubDevice {
            host_id: "local".into(),
            id: "Medium_Phone_API_36.1".into(),
            name: "Medium_Phone_API_36.1".into(),
            platform: DevicePlatform::Android,
            version: "Android".into(),
            ..Default::default()
        });
        let (state, calls) = app_state(cx, &runtime, state);
        let (_panel, cx) = cx.add_window_view(|_, cx| DevicePanel::new(state, "chat".into(), cx));
        wait(cx, |cx| cx.debug_bounds("device-group-android").is_some());
        let ios = cx.debug_bounds("device-group-ios").unwrap();
        let android = cx.debug_bounds("device-group-android").unwrap();
        assert!(ios.top() < android.top());
        click(cx, "device-row-Medium_Phone_API_36.1");
        wait(cx, |_| {
            !calls_of(&calls, methods::OPEN_HUB_DEVICE).is_empty()
        });
        assert_eq!(
            calls_of(&calls, methods::OPEN_HUB_DEVICE),
            vec![serde_json::json!({
                "chatId": "chat", "hostId": "local",
                "deviceId": "Medium_Phone_API_36.1", "platform": "android"
            })]
        );
    }

    #[gpui::test]
    fn the_android_rail_has_back_recents_and_a_rotate_menu_of_device_actions(
        cx: &mut TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (_panel, calls, cx) = android_panel(cx, &runtime, Arc::new(|_| serde_json::json!({})));
        click(cx, "device-back");
        click(cx, "device-recents");
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
                serde_json::json!({"button": {"button": "back"}}),
                serde_json::json!({"button": {"button": "app_switcher"}}),
            ]
        );
        wait(cx, |_| {
            !calls_of(&calls, methods::GET_HUB_DEVICE_DETAIL).is_empty()
        });
        click(cx, "device-rotate");
        click(cx, "device-rotate-landscape");
        wait(cx, |_| {
            !calls_of(&calls, methods::RUN_HUB_DEVICE_ACTION).is_empty()
        });
        assert_eq!(
            calls_of(&calls, methods::RUN_HUB_DEVICE_ACTION)[0]["action"],
            serde_json::json!({"setOrientation": {"value": "landscape_left"}})
        );
        assert!(
            calls_of(&calls, methods::SEND_HUB_DEVICE_INPUT).len() == 2,
            "rotation is an adb action, never an input"
        );
    }

    #[gpui::test]
    fn the_android_tools_drawer_shows_the_emulator_section_and_drops_ios_only_tools(
        cx: &mut TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let detail = serde_json::json!({
            "hostId": "local", "deviceId": "SIM", "readAt": "now",
            "settings": {"appearance": "light", "textSize": "default", "networkEnabled": true},
            "foregroundApp": {"id": "com.android.settings"}
        });
        let (_panel, calls, cx) = android_panel(
            cx,
            &runtime,
            Arc::new(move |method| match method {
                methods::GET_HUB_DEVICE_DETAIL | methods::RUN_HUB_DEVICE_ACTION => detail.clone(),
                _ => serde_json::json!({}),
            }),
        );
        cx.simulate_resize(gpui::size(gpui::px(900.0), gpui::px(1400.0)));
        click(cx, "device-tools-toggle");
        wait(cx, |cx| cx.debug_bounds("device-tool-network").is_some());
        for absent in [
            "device-tool-push",
            "device-tool-event-log",
            "device-tool-ax-overlay",
            "device-tool-location-clear",
            "device-tool-permission-Reset",
            "device-tool-glass-Clear",
        ] {
            assert!(cx.debug_bounds(absent).is_none(), "{absent}");
        }
        assert!(cx.debug_bounds("device-tool-orientation-3").is_some());
        click(cx, "device-tool-network");
        wait(cx, |_| {
            !calls_of(&calls, methods::RUN_HUB_DEVICE_ACTION).is_empty()
        });
        assert_eq!(
            calls_of(&calls, methods::RUN_HUB_DEVICE_ACTION)[0]["action"],
            serde_json::json!({"setToggle": {"setting": "networkEnabled", "enabled": false}})
        );
        assert!(
            calls_of(&calls, methods::WATCH_HUB_DEVICE_APP_STATE).is_empty(),
            "the foreground app comes from the detail read on Android"
        );
    }

    #[gpui::test]
    fn a_failed_android_action_rereads_the_detail_and_keeps_its_error(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let detail = serde_json::json!({
            "hostId": "local", "deviceId": "SIM", "readAt": "now",
            "settings": {"networkEnabled": true}
        });
        let (_panel, calls, cx) = android_panel(
            cx,
            &runtime,
            Arc::new(move |method| match method {
                methods::GET_HUB_DEVICE_DETAIL => detail.clone(),
                _ => serde_json::json!("The device action failed at step 2 of 2"),
            }),
        );
        cx.simulate_resize(gpui::size(gpui::px(900.0), gpui::px(1400.0)));
        click(cx, "device-tools-toggle");
        wait(cx, |cx| cx.debug_bounds("device-tool-network").is_some());
        let reads = calls_of(&calls, methods::GET_HUB_DEVICE_DETAIL).len();
        click(cx, "device-tool-network");
        wait(cx, |_| {
            calls_of(&calls, methods::GET_HUB_DEVICE_DETAIL).len() == reads + 1
        });
        std::thread::sleep(Duration::from_millis(200));
        cx.run_until_parked();
        assert!(cx.debug_bounds("device-tools-error").is_some());
    }

    #[gpui::test]
    fn a_physical_android_device_offers_neither_power_off_nor_location(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let detail = serde_json::json!({
            "hostId": "local", "deviceId": "SIM", "readAt": "now", "settings": {}
        });
        let mut hub = android(streaming(StreamLinkStatus::Live));
        hub.devices[0].physical = true;
        let (state, _) = app_state_with(cx, &runtime, hub, Arc::new(move |_| detail.clone()));
        let (_panel, cx) = cx.add_window_view(|_, cx| {
            let mut panel = DevicePanel::new(state, "chat".into(), cx);
            panel.target = Some(("local".into(), "SIM".into()));
            panel.revision = None;
            panel.sync(cx);
            panel
        });
        cx.simulate_resize(gpui::size(gpui::px(900.0), gpui::px(1400.0)));
        wait(cx, |cx| cx.debug_bounds("device-tools-toggle").is_some());
        click(cx, "device-tools-toggle");
        wait(cx, |cx| cx.debug_bounds("device-tool-network").is_some());
        assert!(cx.debug_bounds("device-tool-location-set").is_none());
        click(cx, "device-more");
        wait(cx, |cx| cx.debug_bounds("device-close-panel").is_some());
        assert!(cx.debug_bounds("device-power-off").is_none());
    }

    #[gpui::test]
    fn a_hidden_panel_drops_its_frame_subscription_and_resubscribes_when_shown(
        cx: &mut TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (_state, panel, calls, cx) = streaming_panel(cx, &runtime, StreamLinkStatus::Live);
        wait(cx, |_| open_frame_streams(&calls) == 1);
        panel.update(cx, |panel, cx| panel.set_visible(false, cx));
        wait(cx, |_| open_frame_streams(&calls) == 0);
        assert!(panel.read_with(cx, |panel, _| panel.stream_view().is_none()));
        assert_eq!(calls_of(&calls, methods::WATCH_HUB_DEVICE_FRAMES).len(), 1);
        cx.run_until_parked();
        assert_eq!(
            open_frame_streams(&calls),
            0,
            "a hidden panel never resubscribes"
        );
        panel.update(cx, |panel, cx| panel.set_visible(true, cx));
        wait(cx, |_| open_frame_streams(&calls) == 1);
        assert_eq!(calls_of(&calls, methods::WATCH_HUB_DEVICE_FRAMES).len(), 2);
        wait(cx, |cx| cx.debug_bounds("device-more").is_some());
    }

    #[gpui::test]
    fn renaming_commits_trims_and_an_empty_name_falls_back_to_the_device_name(
        cx: &mut TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (_state, panel, _calls, cx) = streaming_panel(cx, &runtime, StreamLinkStatus::Live);
        let rename = |cx: &mut VisualTestContext, text: &str, commit: bool| {
            cx.update(|window, cx| {
                panel.update(cx, |panel, cx| panel.begin_rename(window, cx));
            });
            let input = panel.read_with(cx, |panel, _| panel.rename_input().unwrap());
            assert_eq!(
                input.read_with(cx, |input, _| input.text().to_string()),
                panel.read_with(cx, |panel, cx| panel.title(cx).to_string()),
                "the field starts from the current title"
            );
            input.update(cx, |input, cx| input.set_text(text, cx));
            panel.update(cx, |panel, cx| panel.finish_rename(commit, cx));
            panel.read_with(cx, |panel, cx| panel.title(cx).to_string())
        };
        assert_eq!(rename(cx, "iPhone 17", true), "iPhone 17");
        assert!(
            panel.read_with(cx, |panel, _| panel.custom_title.is_none()),
            "an unedited rename does not pin the device name"
        );
        assert_eq!(rename(cx, "  My phone ", true), "My phone");
        assert_eq!(rename(cx, "Discarded", false), "My phone", "Escape reverts");
        assert_eq!(rename(cx, "   ", true), "iPhone 17");
        assert!(panel.read_with(cx, |panel, _| panel.rename_input().is_none()));
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
        panel.update(cx, |panel, _| panel.custom_title = Some("My phone".into()));
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
        assert!(panel.read_with(cx, |panel, _| panel.custom_title.is_none()));
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
    fn a_tab_whose_session_ends_returns_to_the_picker_and_a_new_target_drops_the_custom_title(
        cx: &mut TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (state, panel, _calls, cx) = streaming_panel(cx, &runtime, StreamLinkStatus::Live);
        panel.update(cx, |panel, cx| {
            panel.custom_title = Some("My phone".into());
            panel.bind("local".into(), "PAD".into(), cx);
            assert_eq!(panel.custom_title, None, "a new device drops the old name");
            assert_eq!(
                panel.target,
                Some(("local".into(), "PAD".into())),
                "a session the hub has not published yet keeps the bind"
            );
            panel.bind("local".into(), "SIM".into(), cx);
            panel.custom_title = Some("My phone".into());
            panel.bind("local".into(), "SIM".into(), cx);
            assert_eq!(panel.custom_title.as_deref(), Some("My phone"));
        });
        state.update(cx, |state, cx| {
            let mut ended = hub(true);
            ended.revision = 1;
            state.device_hub = Some(ended);
            cx.notify();
        });
        cx.run_until_parked();
        panel.read_with(cx, |panel, cx| {
            assert_eq!(panel.target, None, "the session ended elsewhere");
            assert_eq!(panel.custom_title, None);
            assert_eq!(panel.title(cx).as_ref(), "Device");
        });
        cx.update(|window, cx| {
            panel.update(cx, |panel, cx| panel.begin_rename(window, cx));
        });
        assert!(
            panel.read_with(cx, |panel, _| panel.rename_input().is_none()),
            "a picker tab cannot be renamed"
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
                if let DevicePanelEvent::Close = event {
                    *seen.lock().unwrap() = true;
                }
            })
        });
        click(cx, "device-setup-cancel");
        wait(cx, |_| *closed.lock().unwrap());
    }

    #[gpui::test]
    fn the_wizard_switches_stay_inside_the_card_in_a_narrow_pane(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let mut state = hub(true);
        state.settings.onboarding_completed = false;
        let (state, _calls) = app_state(cx, &runtime, state);
        let (_panel, cx) = cx.add_window_view(|_, cx| DevicePanel::new(state, "chat".into(), cx));
        cx.simulate_resize(gpui::size(gpui::px(360.0), gpui::px(800.0)));
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
        });
        let inside = |cx: &mut VisualTestContext, selector: &'static str| {
            wait(cx, |cx| cx.debug_bounds(selector).is_some());
            let card = cx.debug_bounds("device-setup-card").unwrap();
            let control = cx.debug_bounds(selector).unwrap();
            assert!(
                control.left() >= card.left() && control.right() <= card.right(),
                "{selector} {control:?} is outside the card {card:?}"
            );
            assert!(card.right() <= gpui::px(360.0), "{card:?}");
        };
        inside(cx, "device-hub-enable");
        click(cx, "device-setup-continue");
        wait(cx, |cx| cx.debug_bounds("device-setup-check").is_some());
        click(cx, "device-setup-continue");
        inside(cx, "device-agent-access");
    }

    #[gpui::test]
    fn the_wizard_footer_stays_inside_the_card_on_every_step(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let mut state = hub(true);
        state.settings.onboarding_completed = false;
        let (state, _calls) = app_state(cx, &runtime, state);
        let (_panel, cx) = cx.add_window_view(|_, cx| DevicePanel::new(state, "chat".into(), cx));
        cx.simulate_resize(gpui::size(gpui::px(620.0), gpui::px(900.0)));
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
        });
        for (back, next) in [
            ("device-setup-cancel", "device-setup-continue"),
            ("device-setup-back", "device-setup-continue"),
            ("device-setup-back", "device-setup-done"),
        ] {
            for selector in [back, next] {
                wait(cx, |cx| cx.debug_bounds(selector).is_some());
                let card = cx.debug_bounds("device-setup-card").unwrap();
                let button = cx.debug_bounds(selector).unwrap();
                assert!(
                    button.top() >= card.top() && button.bottom() <= card.bottom(),
                    "{selector} {button:?} is outside the card {card:?}"
                );
            }
            if next == "device-setup-continue" {
                click(cx, next);
            }
        }
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
