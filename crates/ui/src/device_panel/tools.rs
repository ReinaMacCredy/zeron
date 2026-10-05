use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine as _;
use gpui::{
    Context, Entity, SharedString, Stateful, Subscription, Task, WeakEntity, Window, div,
    prelude::*, px,
};
use zeron_proto::device_hub::{
    ColorFilter, DeviceAction, DeviceAppState, DeviceAppearance, DeviceAxNode, DeviceAxTree,
    DeviceDetail, DeviceEventLogBatch, DeviceEventLogEntry, DevicePermission,
    DeviceSettingsSnapshot, DeviceTextSize, DeviceToggle, ForegroundApp, HubDeviceScreenshot,
    LiquidGlass, PermissionDecision,
};
use zeron_rpc::methods;

use super::stream_view::{AxBox, DeviceStreamView};
use crate::composer::{ComposerInput, ComposerInputEvent};
use crate::icons::{self, icon};
use crate::state::EngineHandle;
use crate::theme::Theme;

pub const DRAWER_WIDTH: f32 = 288.0;
pub const DOCK_MIN_WIDTH: f32 = 700.0;
pub const AX_POLL: Duration = Duration::from_secs(2);
pub const EVENT_LOG_LIMIT: usize = 100;
const FEED_RETRY: Duration = Duration::from_secs(2);
const INVALID_LOCATION: &str =
    "Enter a latitude between -90 and 90 and a longitude between -180 and 180.";

pub const TEXT_SIZES: [(DeviceTextSize, &str); 4] = [
    (DeviceTextSize::Small, "Small"),
    (DeviceTextSize::Default, "Default"),
    (DeviceTextSize::Large, "Large"),
    (DeviceTextSize::ExtraLarge, "Extra large"),
];

const COLOR_FILTERS: [(ColorFilter, &str); 5] = [
    (ColorFilter::None, "None"),
    (ColorFilter::Grayscale, "Grayscale"),
    (ColorFilter::RedGreen, "Red / green (protanopia)"),
    (ColorFilter::GreenRed, "Green / red (deuteranopia)"),
    (ColorFilter::BlueYellow, "Blue / yellow (tritanopia)"),
];

const PERMISSIONS: [(DevicePermission, &str); 11] = [
    (DevicePermission::Camera, "Camera"),
    (DevicePermission::Microphone, "Microphone"),
    (DevicePermission::Photos, "Photos"),
    (DevicePermission::Contacts, "Contacts"),
    (DevicePermission::Calendar, "Calendar"),
    (DevicePermission::Reminders, "Reminders"),
    (DevicePermission::Location, "Location"),
    (DevicePermission::Notifications, "Notifications"),
    (DevicePermission::Motion, "Motion"),
    (DevicePermission::MediaLibrary, "Media library"),
    (DevicePermission::FaceId, "Face ID"),
];

pub const LOCATION_PRESETS: [(&str, f64, f64); 5] = [
    ("San Francisco", 37.7749, -122.4194),
    ("New York", 40.7128, -74.006),
    ("London", 51.5074, -0.1278),
    ("Stockholm", 59.3293, 18.0686),
    ("Tokyo", 35.6762, 139.6503),
];

const TOGGLES: [(DeviceToggle, &str); 5] = [
    (DeviceToggle::ReduceMotion, "Reduce Motion"),
    (DeviceToggle::IncreaseContrast, "Increase Contrast"),
    (DeviceToggle::ReduceTransparency, "Reduce Transparency"),
    (DeviceToggle::ShowBorders, "Show Borders"),
    (DeviceToggle::VoiceOver, "VoiceOver"),
];

pub fn screenshot_file_name(device_name: &str, millis: i64) -> String {
    let base: String = device_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("{base}-{millis}.png")
}

pub fn downloads_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Downloads"))
}

pub fn save_screenshot(
    dir: &Path,
    device_name: &str,
    millis: i64,
    shot: &HubDeviceScreenshot,
) -> Result<PathBuf, String> {
    let png = base64::engine::general_purpose::STANDARD
        .decode(&shot.png)
        .map_err(|e| format!("The screenshot could not be read: {e}"))?;
    let path = dir.join(screenshot_file_name(device_name, millis));
    std::fs::write(&path, png).map_err(|e| format!("Could not save the screenshot: {e}"))?;
    Ok(path)
}

pub fn ax_boxes(tree: &DeviceAxTree) -> Vec<AxBox> {
    let Some(root) = tree.nodes.first() else {
        return Vec::new();
    };
    let (screen_w, screen_h) = (root.frame.width.max(1.0), root.frame.height.max(1.0));
    let mut boxes = Vec::new();
    fn visit(nodes: &[DeviceAxNode], screen: (f64, f64), boxes: &mut Vec<AxBox>) {
        for node in nodes {
            let frame = node.frame;
            let covers =
                (frame.width - screen.0).abs() < 0.5 && (frame.height - screen.1).abs() < 0.5;
            if !covers && frame.width > 0.0 && frame.height > 0.0 {
                boxes.push(AxBox {
                    x: (frame.x / screen.0) as f32,
                    y: (frame.y / screen.1) as f32,
                    width: (frame.width / screen.0) as f32,
                    height: (frame.height / screen.1) as f32,
                });
            }
            visit(&node.children, screen, boxes);
        }
    }
    visit(&tree.nodes, (screen_w, screen_h), &mut boxes);
    boxes
}

pub fn push_events(log: &mut VecDeque<DeviceEventLogEntry>, batch: DeviceEventLogBatch) {
    if batch.reset {
        log.clear();
    }
    log.extend(batch.events);
    while log.len() > EVENT_LOG_LIMIT {
        log.pop_front();
    }
}

fn parse_location(lat: &str, lon: &str) -> Option<(f64, f64)> {
    let (lat, lon): (f64, f64) = (lat.trim().parse().ok()?, lon.trim().parse().ok()?);
    ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)).then_some((lat, lon))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Url,
    Bundle,
    Latitude,
    Longitude,
    AppId,
    Push,
}

const FIELDS: [(Field, &str); 6] = [
    (Field::Url, "https://... or myapp://"),
    (Field::Bundle, "Bundle ID to launch"),
    (Field::Latitude, "Latitude"),
    (Field::Longitude, "Longitude"),
    (Field::AppId, "App ID for permissions"),
    (Field::Push, "Alert text"),
];

pub struct DeviceControls {
    engine: EngineHandle,
    host_id: String,
    device_id: String,
    view: WeakEntity<DeviceStreamView>,
    detail: Option<DeviceDetail>,
    error: Option<SharedString>,
    pending: bool,
    revision: u64,
    foreground: Option<Option<ForegroundApp>>,
    drawer: bool,
    ax_overlay: bool,
    events_open: bool,
    events: VecDeque<DeviceEventLogEntry>,
    permission: DevicePermission,
    inputs: Vec<(Field, Entity<ComposerInput>)>,
    request: Task<()>,
    app_feed: Task<()>,
    ax_poll: Task<()>,
    event_feed: Task<()>,
    _inputs: Vec<Subscription>,
}

impl DeviceControls {
    pub fn new(
        engine: EngineHandle,
        host_id: String,
        device_id: String,
        view: WeakEntity<DeviceStreamView>,
        cx: &mut Context<Self>,
    ) -> Self {
        let inputs: Vec<(Field, Entity<ComposerInput>)> = FIELDS
            .iter()
            .map(|(field, placeholder)| {
                let input = cx.new(|cx| {
                    ComposerInput::new(*placeholder, cx)
                        .with_text_metrics(12.0, 16.0)
                        .with_single_line()
                        .with_accessibility_role(gpui::Role::TextInput)
                });
                (*field, input)
            })
            .collect();
        let subscriptions = inputs
            .iter()
            .map(|(field, input)| {
                let field = *field;
                cx.subscribe(input, move |this: &mut Self, _, event, cx| {
                    if matches!(event, ComposerInputEvent::Submitted) {
                        this.submit(field, cx);
                    }
                })
            })
            .collect();
        let mut controls = Self {
            engine,
            host_id,
            device_id,
            view,
            detail: None,
            error: None,
            pending: false,
            revision: 0,
            foreground: None,
            drawer: false,
            ax_overlay: false,
            events_open: false,
            events: VecDeque::new(),
            permission: DevicePermission::Camera,
            inputs,
            request: Task::ready(()),
            app_feed: Task::ready(()),
            ax_poll: Task::ready(()),
            event_feed: Task::ready(()),
            _inputs: subscriptions,
        };
        controls.read(cx);
        controls
    }

    fn target(&self) -> serde_json::Value {
        serde_json::json!({ "hostId": self.host_id, "deviceId": self.device_id })
    }

    fn input(&self, field: Field) -> &Entity<ComposerInput> {
        &self
            .inputs
            .iter()
            .find(|(candidate, _)| *candidate == field)
            .expect("every field has an input")
            .1
    }

    fn text(&self, field: Field, cx: &Context<Self>) -> String {
        self.input(field).read(cx).text().trim().to_string()
    }

    pub fn settings(&self) -> Option<&DeviceSettingsSnapshot> {
        self.detail.as_ref().map(|detail| &detail.settings)
    }

    pub fn disabled(&self) -> bool {
        self.pending || self.detail.is_none()
    }

    pub fn drawer_open(&self) -> bool {
        self.drawer
    }

    pub fn foreground_app(&self) -> Option<ForegroundApp> {
        match &self.foreground {
            Some(app) => app.clone(),
            None => self.detail.as_ref()?.foreground_app.clone(),
        }
    }

    fn apply(&mut self, revision: u64, reply: Result<serde_json::Value, String>) {
        if revision != self.revision {
            return;
        }
        self.pending = false;
        match reply.and_then(|value| {
            serde_json::from_value::<DeviceDetail>(value).map_err(|e| e.to_string())
        }) {
            Ok(mut detail) => {
                let DeviceDetail { foreground_app, .. } = &mut detail;
                if foreground_app.is_none() {
                    *foreground_app = self.detail.take().and_then(|old| old.foreground_app);
                }
                self.detail = Some(detail);
                self.error = None;
            }
            Err(error) => self.error = Some(error.into()),
        }
    }

    fn send(&mut self, method: &'static str, params: serde_json::Value, cx: &mut Context<Self>) {
        self.revision += 1;
        let revision = self.revision;
        let engine = self.engine.clone();
        self.request = cx.spawn(async move |this, cx| {
            let reply = engine
                .client()
                .call(method, params)
                .await
                .map_err(|error| error.to_string());
            this.update(cx, |controls, cx| {
                controls.apply(revision, reply);
                cx.notify();
            })
            .ok();
        });
    }

    pub fn read(&mut self, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        self.send(methods::GET_HUB_DEVICE_DETAIL, self.target(), cx);
    }

    pub fn act(&mut self, action: DeviceAction, cx: &mut Context<Self>) {
        if self.disabled() {
            return;
        }
        self.pending = true;
        self.error = None;
        let mut params = self.target();
        params["action"] = serde_json::to_value(action).unwrap_or_default();
        self.send(methods::RUN_HUB_DEVICE_ACTION, params, cx);
        cx.notify();
    }

    pub fn set_drawer(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.drawer == open {
            return;
        }
        self.drawer = open;
        if open {
            self.read(cx);
            self.app_feed = self.feed(methods::WATCH_HUB_DEVICE_APP_STATE, cx, |controls, item| {
                if let Ok(state) = serde_json::from_value::<DeviceAppState>(item) {
                    controls.foreground = Some(state.foreground_app);
                }
            });
        } else {
            self.app_feed = Task::ready(());
            self.foreground = None;
        }
        self.sync_events(cx);
        cx.notify();
    }

    fn feed(
        &self,
        method: &'static str,
        cx: &mut Context<Self>,
        apply: impl Fn(&mut Self, serde_json::Value) + 'static,
    ) -> Task<()> {
        let (engine, params) = (self.engine.clone(), self.target());
        cx.spawn(async move |this, cx| {
            loop {
                if let Ok(mut items) = engine
                    .client()
                    .subscribe_scoped(method, params.clone())
                    .await
                {
                    while let Some(item) = items.recv().await {
                        let applied = this.update(cx, |controls, cx| {
                            apply(controls, item);
                            cx.notify();
                        });
                        if applied.is_err() {
                            return;
                        }
                    }
                }
                cx.background_executor().timer(FEED_RETRY).await;
            }
        })
    }

    fn sync_events(&mut self, cx: &mut Context<Self>) {
        if self.drawer && self.events_open {
            self.event_feed = self.feed(methods::WATCH_HUB_DEVICE_EVENTS, cx, |controls, item| {
                if let Ok(batch) = serde_json::from_value::<DeviceEventLogBatch>(item) {
                    push_events(&mut controls.events, batch);
                }
            });
        } else {
            self.event_feed = Task::ready(());
            self.events.clear();
        }
    }

    pub fn set_ax_overlay(&mut self, on: bool, cx: &mut Context<Self>) {
        self.ax_overlay = on;
        if !on {
            self.ax_poll = Task::ready(());
            self.show_boxes(Vec::new(), cx);
            cx.notify();
            return;
        }
        let (engine, params) = (self.engine.clone(), self.target());
        self.ax_poll = cx.spawn(async move |this, cx| {
            loop {
                if let Ok(value) = engine
                    .client()
                    .call(methods::GET_HUB_DEVICE_AX_TREE, params.clone())
                    .await
                    && let Ok(tree) = serde_json::from_value::<DeviceAxTree>(value)
                {
                    let boxes = ax_boxes(&tree);
                    if this
                        .update(cx, |controls, cx| controls.show_boxes(boxes, cx))
                        .is_err()
                    {
                        return;
                    }
                }
                cx.background_executor().timer(AX_POLL).await;
            }
        });
        cx.notify();
    }

    fn show_boxes(&self, boxes: Vec<AxBox>, cx: &mut Context<Self>) {
        if let Some(view) = self.view.upgrade() {
            view.update(cx, |view, cx| view.set_ax_boxes(boxes, cx));
        }
    }

    fn permission_app(&self, cx: &Context<Self>) -> Option<String> {
        let typed = self.text(Field::AppId, cx);
        if !typed.is_empty() {
            return Some(typed);
        }
        self.foreground_app().map(|app| app.id)
    }

    fn submit(&mut self, field: Field, cx: &mut Context<Self>) {
        let text = self.text(field, cx);
        let action = match field {
            Field::Url if !text.is_empty() => DeviceAction::OpenUrl { url: text },
            Field::Bundle if !text.is_empty() => DeviceAction::LaunchApp { app_id: text },
            Field::Latitude | Field::Longitude => {
                return self.set_location(cx);
            }
            Field::Push if !text.is_empty() => {
                let Some(app) = self.foreground_app() else {
                    return;
                };
                DeviceAction::SendPush {
                    app_id: app.id,
                    payload: serde_json::Value::String(text),
                }
            }
            Field::AppId => return self.decide(PermissionDecision::Grant, cx),
            _ => return,
        };
        self.act(action, cx);
        if matches!(field, Field::Url | Field::Bundle | Field::Push) {
            self.input(field)
                .update(cx, |input, cx| input.set_text("", cx));
        }
    }

    fn set_location(&mut self, cx: &mut Context<Self>) {
        let (lat, lon) = (
            self.text(Field::Latitude, cx),
            self.text(Field::Longitude, cx),
        );
        match parse_location(&lat, &lon) {
            Some((lat, lon)) => self.act(DeviceAction::SetLocation { lat, lon }, cx),
            None => {
                self.error = Some(INVALID_LOCATION.into());
                cx.notify();
            }
        }
    }

    fn preset(&mut self, lat: f64, lon: f64, cx: &mut Context<Self>) {
        self.input(Field::Latitude)
            .update(cx, |input, cx| input.set_text(lat.to_string(), cx));
        self.input(Field::Longitude)
            .update(cx, |input, cx| input.set_text(lon.to_string(), cx));
        self.act(DeviceAction::SetLocation { lat, lon }, cx);
    }

    fn decide(&mut self, decision: PermissionDecision, cx: &mut Context<Self>) {
        let Some(app_id) = self.permission_app(cx) else {
            return;
        };
        self.act(
            DeviceAction::SetPermission {
                app_id,
                permission: self.permission,
                decision,
            },
            cx,
        );
    }
}

fn caption(theme: &Theme, text: impl Into<SharedString>) -> gpui::Div {
    div()
        .text_size(crate::typography::ui_rems(11.0))
        .text_color(theme.text_muted)
        .child(text.into())
}

fn section(theme: &Theme, title: &'static str) -> gpui::Div {
    div()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .px(px(12.0))
        .py(px(10.0))
        .border_b_1()
        .border_color(theme.border)
        .child(caption(theme, title).font_weight(gpui::FontWeight::MEDIUM))
}

fn labelled(theme: &Theme, label: &'static str, control: impl IntoElement) -> gpui::Div {
    div()
        .min_w_0()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.0))
        .child(caption(theme, label).flex_none())
        .child(div().min_w_0().flex().justify_end().child(control))
}

fn control(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    enabled: bool,
    theme: &Theme,
) -> Stateful<gpui::Div> {
    let id: SharedString = id.into();
    let selector = id.clone();
    let el = div()
        .id(id)
        .debug_selector(move || selector.to_string())
        .aria_label(label.into());
    if enabled {
        el.cursor_pointer().hover(|s| s.bg(crate::theme::ink(0.06)))
    } else {
        el.opacity(0.45).cursor_default()
    }
    .text_color(theme.text)
}

fn button(
    id: impl Into<SharedString>,
    label: &str,
    enabled: bool,
    theme: &Theme,
) -> Stateful<gpui::Div> {
    control(id, label.to_string(), enabled, theme)
        .role(gpui::Role::Button)
        .flex_none()
        .px(px(8.0))
        .py(px(3.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(theme.border)
        .text_size(crate::typography::ui_rems(12.0))
        .child(label.to_string())
}

fn chip(
    id: impl Into<SharedString>,
    label: &str,
    selected: bool,
    enabled: bool,
    theme: &Theme,
) -> Stateful<gpui::Div> {
    control(id, label.to_string(), enabled, theme)
        .role(gpui::Role::Button)
        .aria_toggled(if selected {
            gpui::Toggled::True
        } else {
            gpui::Toggled::False
        })
        .max_w_full()
        .px(px(8.0))
        .py(px(3.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(if selected { theme.accent } else { theme.border })
        .when(selected, |el| el.bg(theme.accent.opacity(0.12)))
        .text_size(crate::typography::ui_rems(12.0))
        .truncate()
        .child(label.to_string())
}

fn chips() -> gpui::Div {
    div().min_w_0().flex().flex_wrap().gap(px(6.0))
}

fn switch(
    id: &'static str,
    label: &'static str,
    value: Option<bool>,
    enabled: bool,
    theme: &Theme,
) -> Stateful<gpui::Div> {
    control(id, label, enabled && value.is_some(), theme)
        .role(gpui::Role::Switch)
        .aria_toggled(if value == Some(true) {
            gpui::Toggled::True
        } else {
            gpui::Toggled::False
        })
        .flex_none()
        .child(crate::settings::widgets::toggle_switch(
            theme,
            value == Some(true),
            id,
        ))
}

fn field_box(input: &Entity<ComposerInput>, theme: &Theme) -> gpui::Div {
    div()
        .flex_1()
        .min_w_0()
        .h(px(26.0))
        .px(px(8.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(theme.border)
        .overflow_hidden()
        .flex()
        .items_center()
        .child(input.clone())
}

fn row() -> gpui::Div {
    div().min_w_0().flex().items_center().gap(px(6.0))
}

impl DeviceControls {
    fn render_app(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let enabled = !self.disabled();
        let foreground = self.foreground_app();
        let id_text = foreground
            .as_ref()
            .map_or_else(|| "None".to_string(), |app| app.id.clone());
        let mut app = section(theme, "App").child(labelled(
            theme,
            "Foreground",
            div()
                .id("device-tool-foreground")
                .debug_selector(|| "device-tool-foreground".into())
                .role(gpui::Role::Status)
                .aria_label(SharedString::from(format!("Foreground app {id_text}")))
                .min_w_0()
                .truncate()
                .font_family("monospace")
                .text_size(crate::typography::ui_rems(11.0))
                .text_color(theme.text)
                .child(id_text),
        ));
        if let Some(app_id) = foreground.map(|app| app.id) {
            let relaunch = app_id.clone();
            app = app.child(
                row()
                    .child(
                        button("device-tool-terminate", "Terminate", enabled, theme).when(
                            enabled,
                            |el| {
                                el.on_click(cx.listener(move |this, _, _, cx| {
                                    this.act(
                                        DeviceAction::TerminateApp {
                                            app_id: app_id.clone(),
                                        },
                                        cx,
                                    )
                                }))
                            },
                        ),
                    )
                    .child(
                        button("device-tool-relaunch", "Relaunch", enabled, theme).when(
                            enabled,
                            |el| {
                                el.on_click(cx.listener(move |this, _, _, cx| {
                                    this.act(
                                        DeviceAction::LaunchApp {
                                            app_id: relaunch.clone(),
                                        },
                                        cx,
                                    )
                                }))
                            },
                        ),
                    ),
            );
        }
        let submit_row = |field: Field, id: &'static str, label: &'static str| {
            row().child(field_box(self.input(field), theme)).child(
                button(id, label, enabled, theme).when(enabled, |el| {
                    el.on_click(cx.listener(move |this, _, _, cx| this.submit(field, cx)))
                }),
            )
        };
        app.child(submit_row(Field::Url, "device-tool-open-url", "Open"))
            .child(submit_row(Field::Bundle, "device-tool-launch", "Launch"))
    }

    fn render_simulator(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let enabled = !self.disabled();
        let settings = self.settings().cloned().unwrap_or_default();
        let choice = |title: &'static str, chips: gpui::Div| {
            div()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(caption(theme, title))
                .child(chips)
        };
        let appearance = [
            (DeviceAppearance::Light, "Light"),
            (DeviceAppearance::Dark, "Dark"),
        ]
        .into_iter()
        .fold(chips(), |el, (value, label)| {
            el.child(
                chip(
                    format!("device-tool-appearance-{label}"),
                    label,
                    settings.appearance == Some(value),
                    enabled,
                    theme,
                )
                .when(enabled, |el| {
                    el.on_click(cx.listener(move |this, _, _, cx| {
                        this.act(DeviceAction::SetAppearance { value }, cx)
                    }))
                }),
            )
        });
        let text_size =
            TEXT_SIZES
                .into_iter()
                .enumerate()
                .fold(chips(), |el, (i, (value, label))| {
                    el.child(
                        chip(
                            format!("device-tool-text-size-{i}"),
                            label,
                            settings.text_size == Some(value),
                            enabled,
                            theme,
                        )
                        .when(enabled, |el| {
                            el.on_click(cx.listener(move |this, _, _, cx| {
                                this.act(DeviceAction::SetTextSize { value }, cx)
                            }))
                        }),
                    )
                });
        let glass_known = settings.liquid_glass.is_some();
        let glass = [
            (LiquidGlass::Clear, "Clear"),
            (LiquidGlass::Tinted, "Tinted"),
        ]
        .into_iter()
        .fold(chips(), |el, (value, label)| {
            let enabled = enabled && glass_known;
            el.child(
                chip(
                    format!("device-tool-glass-{label}"),
                    label,
                    settings.liquid_glass == Some(value),
                    enabled,
                    theme,
                )
                .when(enabled, |el| {
                    el.on_click(cx.listener(move |this, _, _, cx| {
                        this.act(DeviceAction::SetLiquidGlass { value }, cx)
                    }))
                }),
            )
        });
        let filters =
            COLOR_FILTERS
                .into_iter()
                .enumerate()
                .fold(chips(), |el, (i, (value, label))| {
                    el.child(
                        chip(
                            format!("device-tool-filter-{i}"),
                            label,
                            settings.color_filter == Some(value),
                            enabled,
                            theme,
                        )
                        .when(enabled, |el| {
                            el.on_click(cx.listener(move |this, _, _, cx| {
                                this.act(DeviceAction::SetColorFilter { value }, cx)
                            }))
                        }),
                    )
                });
        let toggles = TOGGLES.into_iter().map(|(setting, label)| {
            let value = match setting {
                DeviceToggle::ReduceMotion => settings.reduce_motion,
                DeviceToggle::IncreaseContrast => settings.increase_contrast,
                DeviceToggle::ReduceTransparency => settings.reduce_transparency,
                DeviceToggle::ShowBorders => settings.show_borders,
                _ => settings.voice_over,
            };
            let id = match setting {
                DeviceToggle::ReduceMotion => "device-tool-reduce-motion",
                DeviceToggle::IncreaseContrast => "device-tool-increase-contrast",
                DeviceToggle::ReduceTransparency => "device-tool-reduce-transparency",
                DeviceToggle::ShowBorders => "device-tool-show-borders",
                _ => "device-tool-voiceover",
            };
            let live = enabled && value.is_some();
            labelled(
                theme,
                label,
                switch(id, label, value, enabled, theme).when(live, |el| {
                    el.on_click(cx.listener(move |this, _, _, cx| {
                        this.act(
                            DeviceAction::SetToggle {
                                setting,
                                enabled: value != Some(true),
                            },
                            cx,
                        )
                    }))
                }),
            )
        });
        section(theme, "Simulator")
            .child(choice("Appearance", appearance))
            .child(choice("Text size", text_size))
            .child(choice("Liquid Glass", glass))
            .child(choice("Color filter", filters))
            .children(toggles)
    }

    fn render_location(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let enabled = !self.disabled();
        let presets =
            LOCATION_PRESETS
                .into_iter()
                .enumerate()
                .fold(chips(), |el, (i, (label, lat, lon))| {
                    el.child(
                        chip(
                            format!("device-tool-preset-{i}"),
                            label,
                            false,
                            enabled,
                            theme,
                        )
                        .when(enabled, |el| {
                            el.on_click(
                                cx.listener(move |this, _, _, cx| this.preset(lat, lon, cx)),
                            )
                        }),
                    )
                });
        section(theme, "Location")
            .child(
                row()
                    .child(field_box(self.input(Field::Latitude), theme))
                    .child(field_box(self.input(Field::Longitude), theme)),
            )
            .child(presets)
            .child(
                row()
                    .child(
                        button("device-tool-location-set", "Set", enabled, theme)
                            .when(enabled, |el| {
                                el.on_click(cx.listener(|this, _, _, cx| this.set_location(cx)))
                            }),
                    )
                    .child(
                        button("device-tool-location-clear", "Clear", enabled, theme).when(
                            enabled,
                            |el| {
                                el.on_click(cx.listener(|this, _, _, cx| {
                                    for field in [Field::Latitude, Field::Longitude] {
                                        this.input(field)
                                            .update(cx, |input, cx| input.set_text("", cx));
                                    }
                                    this.act(DeviceAction::ClearLocation, cx)
                                }))
                            },
                        ),
                    ),
            )
    }

    fn render_permissions(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let enabled = !self.disabled() && self.permission_app(cx).is_some();
        let kinds = PERMISSIONS
            .into_iter()
            .enumerate()
            .fold(chips(), |el, (i, (value, label))| {
                el.child(
                    chip(
                        format!("device-tool-permission-{i}"),
                        label,
                        self.permission == value,
                        true,
                        theme,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.permission = value;
                        cx.notify();
                    })),
                )
            });
        let decisions = [
            (PermissionDecision::Grant, "Grant"),
            (PermissionDecision::Revoke, "Revoke"),
            (PermissionDecision::Reset, "Reset"),
        ]
        .into_iter()
        .fold(row(), |el, (decision, label)| {
            el.child(
                button(
                    format!("device-tool-permission-{label}"),
                    label,
                    enabled,
                    theme,
                )
                .when(enabled, |el| {
                    el.on_click(cx.listener(move |this, _, _, cx| this.decide(decision, cx)))
                }),
            )
        });
        section(theme, "Permissions")
            .child(row().child(field_box(self.input(Field::AppId), theme)))
            .child(kinds)
            .child(decisions)
    }

    fn render_push(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let has_app = self.foreground_app().is_some();
        let enabled = !self.disabled() && has_app;
        section(theme, "Push notification")
            .child(
                row()
                    .child(field_box(self.input(Field::Push), theme))
                    .child(button("device-tool-push", "Send", enabled, theme).when(
                        enabled,
                        |el| {
                            el.on_click(cx.listener(|this, _, _, cx| this.submit(Field::Push, cx)))
                        },
                    )),
            )
            .when(!has_app, |el| {
                el.child(caption(theme, "Open an app first."))
            })
    }

    fn render_events(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let open = self.events_open;
        let header = div()
            .id("device-tool-event-log")
            .debug_selector(|| "device-tool-event-log".into())
            .role(gpui::Role::Button)
            .aria_label("Event log")
            .aria_expanded(open)
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(12.0))
            .py(px(10.0))
            .cursor_pointer()
            .child(
                caption(theme, "Event log")
                    .flex_1()
                    .font_weight(gpui::FontWeight::MEDIUM),
            )
            .child(
                icon(if open {
                    icons::LUCIDE_CHEVRON_DOWN
                } else {
                    icons::LUCIDE_CHEVRON_RIGHT
                })
                .size(px(12.0))
                .text_color(theme.text_muted),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.events_open = !this.events_open;
                this.sync_events(cx);
                cx.notify();
            }));
        let entries = self.events.iter().map(|entry| {
            div()
                .min_w_0()
                .flex()
                .gap(px(8.0))
                .child(
                    div()
                        .flex_none()
                        .text_color(theme.text_muted)
                        .child(entry.timestamp.get(11..19).unwrap_or_default().to_string()),
                )
                .child(div().min_w_0().truncate().child(entry.summary.clone()))
        });
        div().min_w_0().child(header).when(open, |el| {
            el.child(
                div()
                    .id("device-tool-event-list")
                    .role(gpui::Role::Log)
                    .aria_label("Device events")
                    .max_h(px(256.0))
                    .overflow_y_scroll()
                    .min_w_0()
                    .px(px(12.0))
                    .pb(px(10.0))
                    .font_family("monospace")
                    .text_size(crate::typography::ui_rems(10.5))
                    .text_color(theme.text)
                    .when(self.events.is_empty(), |el| {
                        el.child(caption(theme, "No events yet."))
                    })
                    .children(entries),
            )
        })
    }
}

impl Render for DeviceControls {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let header = div()
            .flex_none()
            .h(px(36.0))
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(12.0))
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .text_size(crate::typography::ui_rems(12.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child("Tools"),
            )
            .when(self.pending, |el| el.child(caption(&theme, "Applying...")))
            .child(div().flex_1())
            .child(
                div()
                    .id("device-tools-close")
                    .debug_selector(|| "device-tools-close".into())
                    .role(gpui::Role::Button)
                    .aria_label("Close tools")
                    .cursor_pointer()
                    .child(
                        icon(icons::CLOSE)
                            .size(px(12.0))
                            .text_color(theme.text_muted),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.set_drawer(false, cx))),
            );
        let reading = self.detail.is_none() && self.error.is_none();
        let ax_enabled = self.ax_overlay;
        let body = div()
            .id("device-tools-body")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_y_scroll()
            .when_some(self.error.clone(), |el, error| {
                el.child(
                    div()
                        .id("device-tools-error")
                        .debug_selector(|| "device-tools-error".into())
                        .role(gpui::Role::Alert)
                        .min_w_0()
                        .px(px(12.0))
                        .py(px(8.0))
                        .bg(theme.danger_muted)
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.danger)
                        .child(error),
                )
            })
            .when(reading, |el| {
                el.child(
                    caption(&theme, "Reading device settings...")
                        .px(px(12.0))
                        .py(px(10.0)),
                )
            })
            .child(self.render_app(&theme, cx))
            .child(self.render_simulator(&theme, cx))
            .child(
                section(&theme, "Accessibility").child(labelled(
                    &theme,
                    "Overlay element frames",
                    switch(
                        "device-tool-ax-overlay",
                        "Overlay element frames",
                        Some(ax_enabled),
                        true,
                        &theme,
                    )
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.set_ax_overlay(!ax_enabled, cx)),
                    ),
                )),
            )
            .child(self.render_location(&theme, cx))
            .child(self.render_permissions(&theme, cx))
            .child(self.render_push(&theme, cx))
            .child(self.render_events(&theme, cx));
        div()
            .id("device-tools")
            .debug_selector(|| "device-tools".into())
            .role(gpui::Role::Group)
            .aria_label("Device tools")
            .occlude()
            .size_full()
            .min_w_0()
            .overflow_hidden()
            .flex()
            .flex_col()
            .bg(theme.bg)
            .border_l_1()
            .border_color(theme.border)
            .child(header)
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device_panel::DevicePanel;
    use crate::device_panel::tests::{Calls, app_state_with, calls_of, click, streaming, wait};
    use gpui::{Bounds, Pixels, TestAppContext, VisualTestContext};
    use serde_json::json;
    use std::sync::Arc;
    use zeron_proto::device_hub::{AxRect, StreamLinkStatus};

    fn detail(appearance: &str, voice_over: bool) -> serde_json::Value {
        json!({
            "hostId": "local", "deviceId": "SIM", "readAt": "now",
            "settings": {
                "appearance": appearance, "textSize": "default", "reduceMotion": false,
                "increaseContrast": false, "reduceTransparency": false, "showBorders": false,
                "voiceOver": voice_over, "liquidGlass": "clear", "colorFilter": "none"
            },
            "foregroundApp": {"id": "com.apple.Maps", "pid": 7}
        })
    }

    fn panel_with<'a>(
        cx: &'a mut TestAppContext,
        runtime: &tokio::runtime::Runtime,
        size: (f32, f32),
    ) -> (Entity<DevicePanel>, Calls, &'a mut VisualTestContext) {
        let (state, calls) = app_state_with(
            cx,
            runtime,
            streaming(StreamLinkStatus::Live),
            Arc::new(|method| match method {
                methods::GET_HUB_DEVICE_DETAIL => detail("light", false),
                methods::RUN_HUB_DEVICE_ACTION => detail("dark", true),
                methods::GET_HUB_DEVICE_AX_TREE => json!({"nodes": [{
                    "frame": {"x": 0, "y": 0, "width": 400, "height": 800},
                    "children": [
                        {"frame": {"x": 40, "y": 80, "width": 100, "height": 200}, "label": "Maps"},
                        {"frame": {"x": 0, "y": 0, "width": 400, "height": 800}}
                    ]
                }]}),
                _ => json!({}),
            }),
        );
        let (panel, cx) = cx.add_window_view(|_, cx| {
            let mut panel = DevicePanel::new(state, "chat".into(), cx);
            panel.target = Some(("local".into(), "SIM".into()));
            panel.revision = None;
            panel.sync(cx);
            panel
        });
        cx.simulate_resize(gpui::size(px(size.0), px(size.1)));
        redraw(cx);
        wait(cx, |cx| {
            panel.read_with(cx, |panel, cx| {
                panel
                    .controls()
                    .is_some_and(|controls| !controls.read(cx).disabled())
            })
        });
        (panel, calls, cx)
    }

    fn redraw(cx: &mut VisualTestContext) {
        for _ in 0..2 {
            cx.update(|window, cx| {
                window.refresh();
                window.draw(cx).clear();
            });
            cx.run_until_parked();
        }
    }

    fn actions(calls: &Calls) -> Vec<serde_json::Value> {
        calls_of(calls, methods::RUN_HUB_DEVICE_ACTION)
            .into_iter()
            .map(|params| {
                assert_eq!(
                    (&params["hostId"], &params["deviceId"]),
                    (&json!("local"), &json!("SIM"))
                );
                params["action"].clone()
            })
            .collect()
    }

    fn act_and_wait(cx: &mut VisualTestContext, calls: &Calls, selector: &'static str) {
        let before = actions(calls).len();
        click(cx, selector);
        wait(cx, |_| actions(calls).len() == before + 1);
    }

    fn controls(panel: &Entity<DevicePanel>, cx: &mut VisualTestContext) -> Entity<DeviceControls> {
        panel.read_with(cx, |panel, _| panel.controls().unwrap().clone())
    }

    #[gpui::test]
    fn rail_settings_follow_the_read_back_detail_and_each_action_replaces_it(
        cx: &mut TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (panel, calls, cx) = panel_with(cx, &runtime, (600.0, 900.0));
        assert_eq!(
            calls_of(&calls, methods::GET_HUB_DEVICE_DETAIL),
            vec![json!({"hostId": "local", "deviceId": "SIM"})]
        );
        let controls = controls(&panel, cx);
        act_and_wait(cx, &calls, "device-appearance");
        assert_eq!(
            actions(&calls)[0],
            json!({"setAppearance": {"value": "dark"}})
        );
        wait(cx, |cx| {
            controls.read_with(cx, |controls, _| {
                !controls.disabled()
                    && controls.settings().and_then(|s| s.appearance)
                        == Some(DeviceAppearance::Dark)
            })
        });
        act_and_wait(cx, &calls, "device-appearance");
        assert_eq!(
            actions(&calls)[1],
            json!({"setAppearance": {"value": "light"}}),
            "the next flip follows the confirmed dark value"
        );
        wait(cx, |cx| {
            controls.read_with(cx, |controls, _| !controls.disabled())
        });
        click(cx, "device-text-size");
        act_and_wait(cx, &calls, "device-text-size-3");
        assert_eq!(
            actions(&calls)[2],
            json!({"setTextSize": {"value": "extra-large"}})
        );
    }

    #[gpui::test]
    fn drawer_controls_send_typed_actions_and_closing_stops_the_feeds(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (panel, calls, cx) = panel_with(cx, &runtime, (1000.0, 2400.0));
        let controls = controls(&panel, cx);
        click(cx, "device-tools-toggle");
        wait(cx, |cx| cx.debug_bounds("device-tools").is_some());
        wait(cx, |_| {
            calls_of(&calls, methods::WATCH_HUB_DEVICE_APP_STATE).len() == 1
        });
        wait(cx, |cx| {
            controls.read_with(cx, |controls, _| !controls.disabled())
        });
        let settle = |cx: &mut VisualTestContext| {
            wait(cx, |cx| {
                controls.read_with(cx, |controls, _| !controls.disabled())
            })
        };
        act_and_wait(cx, &calls, "device-tool-voiceover");
        settle(cx);
        act_and_wait(cx, &calls, "device-tool-preset-4");
        settle(cx);
        act_and_wait(cx, &calls, "device-tool-filter-1");
        settle(cx);
        act_and_wait(cx, &calls, "device-tool-glass-Tinted");
        settle(cx);
        act_and_wait(cx, &calls, "device-tool-terminate");
        settle(cx);
        click(cx, "device-tool-permission-7");
        act_and_wait(cx, &calls, "device-tool-permission-Grant");
        settle(cx);
        let input = |field: Field, cx: &mut VisualTestContext| {
            controls.read_with(cx, |controls, _| controls.input(field).clone())
        };
        let url = input(Field::Url, cx);
        url.update(cx, |input, cx| input.set_text("myapp://route", cx));
        act_and_wait(cx, &calls, "device-tool-open-url");
        assert_eq!(url.read_with(cx, |input, _| input.text().to_string()), "");
        settle(cx);
        input(Field::Push, cx).update(cx, |input, cx| input.set_text("Build done", cx));
        act_and_wait(cx, &calls, "device-tool-push");
        settle(cx);
        input(Field::Latitude, cx).update(cx, |input, cx| input.set_text("91", cx));
        click(cx, "device-tool-location-set");
        wait(cx, |cx| cx.debug_bounds("device-tools-error").is_some());
        assert_eq!(
            actions(&calls),
            vec![
                json!({"setToggle": {"setting": "voiceOver", "enabled": true}}),
                json!({"setLocation": {"lat": 35.6762, "lon": 139.6503}}),
                json!({"setColorFilter": {"value": "grayscale"}}),
                json!({"setLiquidGlass": {"value": "tinted"}}),
                json!({"terminateApp": {"appId": "com.apple.Maps"}}),
                json!({"setPermission": {"appId": "com.apple.Maps", "permission": "notifications", "decision": "grant"}}),
                json!({"openUrl": {"url": "myapp://route"}}),
                json!({"sendPush": {"appId": "com.apple.Maps", "payload": "Build done"}}),
            ],
            "an out-of-range latitude never reaches the engine"
        );
        click(cx, "device-tool-event-log");
        wait(cx, |_| {
            calls_of(&calls, methods::WATCH_HUB_DEVICE_EVENTS).len() == 1
        });
        let cancels_before = calls_of(&calls, "cancel").len();
        click(cx, "device-tools-close");
        wait(cx, |cx| cx.debug_bounds("device-tools").is_none());
        wait(cx, |_| {
            calls_of(&calls, "cancel").len() >= cancels_before + 2
        });
    }

    fn inside(outer: Bounds<Pixels>, inner: Bounds<Pixels>) -> bool {
        inner.left() >= outer.left() - px(0.5) && inner.right() <= outer.right() + px(0.5)
    }

    #[gpui::test]
    fn the_drawer_overlays_a_narrow_panel_and_docks_beside_a_wide_one(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (panel, _calls, cx) = panel_with(cx, &runtime, (480.0, 2400.0));
        click(cx, "device-tools-toggle");
        wait(cx, |cx| cx.debug_bounds("device-tools").is_some());
        let controls = controls(&panel, cx);
        controls.update(cx, |controls, cx| {
            controls.foreground = Some(Some(ForegroundApp {
                id: format!("com.example.{}", "very-long-bundle-segment.".repeat(12)),
                pid: None,
            }));
            cx.notify();
        });
        controls
            .read_with(cx, |controls, _| controls.input(Field::Url).clone())
            .update(cx, |input, cx| {
                input.set_text(
                    format!("https://example.com/{}", "very-long-segment/".repeat(30)),
                    cx,
                )
            });
        redraw(cx);
        let drawer = cx.debug_bounds("device-tools").unwrap();
        let screen = cx.debug_bounds("device-screen-area").unwrap();
        assert!(drawer.size.width <= px(DRAWER_WIDTH + 0.5), "{drawer:?}");
        assert!(
            (drawer.right() - screen.right()).abs() < px(0.5),
            "overlay hugs the screen's right edge: {drawer:?} {screen:?}"
        );
        let selectors = [
            "device-tool-foreground",
            "device-tool-terminate",
            "device-tool-relaunch",
            "device-tool-open-url",
            "device-tool-launch",
            "device-tool-filter-4",
            "device-tool-voiceover",
            "device-tool-preset-0",
            "device-tool-location-clear",
            "device-tool-permission-Reset",
            "device-tool-push",
            "device-tool-event-log",
        ];
        for selector in selectors {
            let control = cx.debug_bounds(selector).unwrap();
            assert!(
                inside(drawer, control),
                "{selector} {control:?} leaves {drawer:?}"
            );
        }

        cx.simulate_resize(gpui::size(px(1100.0), px(2400.0)));
        redraw(cx);
        let drawer = cx.debug_bounds("device-tools").unwrap();
        let screen = cx.debug_bounds("device-screen-area").unwrap();
        assert!(
            (drawer.size.width - px(DRAWER_WIDTH)).abs() < px(0.5),
            "{drawer:?}"
        );
        assert!(
            (drawer.left() - screen.right()).abs() < px(0.5),
            "docked beside the screen: {drawer:?} {screen:?}"
        );
        for selector in selectors {
            let control = cx.debug_bounds(selector).unwrap();
            assert!(
                inside(drawer, control),
                "{selector} {control:?} leaves {drawer:?}"
            );
        }
    }

    #[gpui::test]
    fn the_ax_overlay_polls_the_tree_and_draws_normalized_boxes(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (panel, calls, cx) = panel_with(cx, &runtime, (1000.0, 2400.0));
        click(cx, "device-tools-toggle");
        click(cx, "device-tool-ax-overlay");
        let view = panel.read_with(cx, |panel, _| panel.stream_view().unwrap().clone());
        wait(cx, |cx| {
            view.read_with(cx, |view, _| !view.ax_boxes().is_empty())
        });
        assert_eq!(
            view.read_with(cx, |view, _| view.ax_boxes().to_vec()),
            vec![AxBox {
                x: 0.1,
                y: 0.1,
                width: 0.25,
                height: 0.25
            }]
        );
        assert_eq!(
            calls_of(&calls, methods::GET_HUB_DEVICE_AX_TREE)[0],
            json!({"hostId": "local", "deviceId": "SIM"})
        );
        click(cx, "device-tool-ax-overlay");
        wait(cx, |cx| {
            view.read_with(cx, |view, _| view.ax_boxes().is_empty())
        });
    }

    #[test]
    fn screenshots_are_named_after_the_device_and_saved_as_png() {
        assert_eq!(
            screenshot_file_name("iPhone 17 Pro", 1_759_651_200_123),
            "iPhone-17-Pro-1759651200123.png"
        );
        assert_eq!(screenshot_file_name("../é/x", 5), "-----x-5.png");
        let dir = tempfile::tempdir().unwrap();
        let png = [0x89, b'P', b'N', b'G', 1, 2, 3];
        let shot = HubDeviceScreenshot {
            png: base64::engine::general_purpose::STANDARD.encode(png),
            width: 1206,
            height: 2622,
        };
        let path = save_screenshot(dir.path(), "iPhone 17", 42, &shot).unwrap();
        assert_eq!(path, dir.path().join("iPhone-17-42.png"));
        assert_eq!(std::fs::read(&path).unwrap(), png);
        let bad = HubDeviceScreenshot {
            png: "%%%".into(),
            ..Default::default()
        };
        assert!(save_screenshot(dir.path(), "x", 1, &bad).is_err());
    }

    #[test]
    fn ax_boxes_skip_screen_sized_nodes_and_events_keep_the_last_hundred() {
        let node = |x, y, width, height, children| DeviceAxNode {
            frame: AxRect {
                x,
                y,
                width,
                height,
            },
            children,
            ..Default::default()
        };
        let tree = DeviceAxTree {
            nodes: vec![node(
                0.0,
                0.0,
                200.0,
                400.0,
                vec![
                    node(
                        0.0,
                        0.0,
                        200.0,
                        400.0,
                        vec![node(20.0, 40.0, 100.0, 40.0, vec![])],
                    ),
                    node(10.0, 10.0, 0.0, 10.0, vec![]),
                ],
            )],
        };
        assert_eq!(
            ax_boxes(&tree),
            vec![AxBox {
                x: 0.1,
                y: 0.1,
                width: 0.5,
                height: 0.1
            }]
        );
        assert!(ax_boxes(&DeviceAxTree::default()).is_empty());

        let entry = |id: usize| DeviceEventLogEntry {
            id: id.to_string(),
            ..Default::default()
        };
        let mut log = VecDeque::new();
        push_events(
            &mut log,
            DeviceEventLogBatch {
                reset: true,
                events: (0..90).map(entry).collect(),
            },
        );
        push_events(
            &mut log,
            DeviceEventLogBatch {
                reset: false,
                events: (90..130).map(entry).collect(),
            },
        );
        assert_eq!(log.len(), EVENT_LOG_LIMIT);
        assert_eq!((log[0].id.as_str(), log[99].id.as_str()), ("30", "129"));
        push_events(
            &mut log,
            DeviceEventLogBatch {
                reset: true,
                events: vec![entry(7)],
            },
        );
        assert_eq!(log.len(), 1);
        assert_eq!(
            parse_location(" 35.6762", "139.6503 "),
            Some((35.6762, 139.6503))
        );
        assert_eq!(parse_location("91", "0"), None);
        assert_eq!(parse_location("x", "0"), None);
    }
}
