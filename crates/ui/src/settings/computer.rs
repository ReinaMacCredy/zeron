//! Settings › Computer, Synara's `ComputerSettingsPanel` with its Getting started and
//! Recent Computer actions sections.
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, Context, Entity, ObjectFit, SharedString, Subscription, Task, Window, div, img,
    prelude::*, px,
};
use zeron_proto::computer::{
    AgentCursorColorMode, COMPUTER_AUDIT_HISTORY_MAX_LIMIT, CUA_DRIVER_VERSION,
    CUA_NATIVE_REVISION, ComputerAuditEffect, ComputerAuditHistoryEntry,
    ComputerAuditHistoryStatus, ComputerAvailability, ComputerBuildSignature, ComputerCapabilities,
    ComputerGetAuditHistoryInput, ComputerGetAuditHistoryResult, ComputerHealthStatus,
    ComputerHostStatus, ComputerPermission, ComputerPreviewPlacement, ComputerPreviewSize,
    ComputerSettings, computer_permission_setup_message, list_computer_permissions,
    normalize_cursor_hex_color,
};
use zeron_rpc::methods;

use crate::appshots::AppshotCapabilities;
use crate::composer::{ComposerInput, ComposerInputEvent};
use crate::icons;
use crate::popover;
use crate::settings::permission_guide::{
    self, PermissionGuide, PermissionGuideHost, PermissionPane, choice_button, outline_button, row,
};
use crate::settings::{SavePolicy, widgets};
use crate::state::AppState;
use crate::theme::Theme;

const STATUS_POLL_INTERVAL: Duration = Duration::from_secs(10);

const COMPUTER_PANES: [(PermissionPane, &str); 3] = [
    (
        PermissionPane::Accessibility,
        "Lets Zeron move the pointer, click, and type on your behalf. Nothing is driven unless you authorize a Computer task.",
    ),
    (
        PermissionPane::ScreenRecording,
        "Lets Zeron capture windows and the desktop so the agent can see what it is driving.",
    ),
    (
        PermissionPane::InputMonitoring,
        "Lets Zeron detect Escape and pause when you take over during a Computer task. This does not enable the Appshots shortcut.",
    ),
];

pub const SETUP_ORDER: [ComputerPermission; 3] = crate::computer::cards::COMPUTER_PERMISSION_ORDER;

pub const HISTORY_PAGE_SIZE: u32 = 30;

pub fn next_setup_pane(missing: &[ComputerPermission]) -> Option<PermissionPane> {
    SETUP_ORDER
        .into_iter()
        .find(|permission| missing.contains(permission))
        .map(PermissionPane::from_permission)
}

pub fn history_title(entry: &ComputerAuditHistoryEntry) -> String {
    let action = crate::computer::presentation::describe_computer_call(&entry.tool, None);
    match crate::computer::presentation::app_name(entry.app.as_deref()) {
        Some(app) => format!("{action} · {app}"),
        None => action,
    }
}

pub fn next_history_page(
    pages: &[ComputerGetAuditHistoryResult],
) -> Option<ComputerGetAuditHistoryInput> {
    let last = pages.last()?;
    let loaded: usize = pages.iter().map(|page| page.entries.len()).sum();
    let remaining = (COMPUTER_AUDIT_HISTORY_MAX_LIMIT as usize).saturating_sub(loaded);
    let before = last.next_cursor.clone()?;
    (remaining > 0).then(|| ComputerGetAuditHistoryInput {
        limit: Some(HISTORY_PAGE_SIZE.min(remaining as u32)),
        before: Some(before),
    })
}

pub fn history_entries(pages: &[ComputerGetAuditHistoryResult]) -> Vec<&ComputerAuditHistoryEntry> {
    let mut seen = std::collections::HashSet::new();
    pages
        .iter()
        .flat_map(|page| page.entries.iter())
        .filter(|entry| seen.insert(entry.id.as_str()))
        .take(COMPUTER_AUDIT_HISTORY_MAX_LIMIT as usize)
        .collect()
}

fn effect_label(effect: ComputerAuditEffect) -> &'static str {
    match effect {
        ComputerAuditEffect::Verified => "Effect observed",
        ComputerAuditEffect::DispatchedUnknown => "Sent; effect unconfirmed",
        ComputerAuditEffect::NotDispatched => "Not sent",
        ComputerAuditEffect::Refused => "Blocked",
        ComputerAuditEffect::Error => "Failed",
    }
}

fn history_time(ts: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(ts)
        .map(|at| {
            at.with_timezone(&chrono::Local)
                .format("%b %-d, %Y, %-I:%M:%S %p")
                .to_string()
        })
        .unwrap_or_else(|_| ts.to_string())
}

fn capability_summary(capabilities: ComputerCapabilities, capture_available: bool) -> String {
    let abilities = [
        (capabilities.capture && capture_available, "screen capture"),
        (capabilities.input, "input"),
        (capabilities.windows, "window listing"),
        (capabilities.window_bounds, "window geometry"),
        (capabilities.stacking, "stacking order"),
        (capabilities.focus, "keyboard focus"),
        (capabilities.raise, "window raising"),
        (capabilities.clipboard, "clipboard"),
        (capabilities.ghost_cursor, "ghost cursor"),
    ];
    let enabled: Vec<&str> = abilities
        .into_iter()
        .filter_map(|(on, label)| on.then_some(label))
        .collect();
    if enabled.is_empty() {
        "none".into()
    } else {
        enabled.join(", ")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewKind {
    Checking,
    Ready,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AvailabilityView {
    kind: ViewKind,
    title: String,
    description: String,
}

fn availability_view(
    status: Option<&ComputerHostStatus>,
    local_missing: &[ComputerPermission],
) -> AvailabilityView {
    let view = |kind, title: &str, description: String| AvailabilityView {
        kind,
        title: title.to_string(),
        description,
    };
    let Some(status) = status.map(|status| &status.status) else {
        return view(
            ViewKind::Checking,
            "Checking computer availability",
            "Waiting for the desktop backend.".into(),
        );
    };
    let health = &status.health;
    if health.status == ComputerHealthStatus::Reconnecting {
        return view(
            ViewKind::Checking,
            "Reconnecting to the desktop",
            health.last_failure.as_ref().map_or_else(
                || "Zeron is reconnecting to the desktop.".into(),
                |failure| failure.message.clone(),
            ),
        );
    }
    match &status.availability {
        ComputerAvailability::Available { .. } if !local_missing.is_empty() => AvailabilityView {
            kind: ViewKind::Blocked,
            title: format!(
                "Computer control needs {}",
                list_computer_permissions(local_missing)
            ),
            description: computer_permission_setup_message(
                local_missing,
                ComputerBuildSignature::Unknown,
                None,
            ),
        },
        ComputerAvailability::Available { .. } if health.status != ComputerHealthStatus::Connected => view(
            ViewKind::Checking,
            "Computer access has not been checked",
            "Choose Set up to check that Zeron can see and control the desktop.".into(),
        ),
        ComputerAvailability::Available { .. } if !health.capture_available => view(
            ViewKind::Blocked,
            "Screen capture is unavailable",
            "Desktop input is connected, but Zeron cannot take screenshots. Choose Set up to check access.".into(),
        ),
        ComputerAvailability::Available { .. } => view(
            ViewKind::Ready,
            "Computer control available",
            "The agent can use the desktop through its computer tools.".into(),
        ),
        ComputerAvailability::UnsupportedPlatform { platform } => view(
            ViewKind::Blocked,
            "Computer control is unavailable",
            format!("This engine is running on {platform}. Computer control needs macOS."),
        ),
        ComputerAvailability::PermissionRequired {
            missing, message, ..
        } => AvailabilityView {
            kind: ViewKind::Blocked,
            title: format!("Computer control needs {}", list_computer_permissions(missing)),
            description: message.clone(),
        },
        other => view(
            ViewKind::Blocked,
            "Computer control is unavailable",
            availability_message(other),
        ),
    }
}

fn availability_message(availability: &ComputerAvailability) -> String {
    serde_json::to_value(availability)
        .ok()
        .and_then(|value| {
            value
                .get("message")
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "The computer backend is not available.".into())
}

fn needs_setup(status: Option<&ComputerHostStatus>, local_missing: &[ComputerPermission]) -> bool {
    if !local_missing.is_empty() {
        return true;
    }
    let Some(status) = status.map(|status| &status.status) else {
        return false;
    };
    if matches!(
        status.availability,
        ComputerAvailability::UnsupportedPlatform { .. }
    ) {
        return false;
    }
    (status.provisionable == Some(true) && status.health.status != ComputerHealthStatus::Connected)
        || matches!(
            status.availability,
            ComputerAvailability::BackendUnavailable { .. }
                | ComputerAvailability::PermissionRequired { .. }
        )
        || !status.health.capture_available
        || !status.capabilities.input
        || !status.capabilities.capture
}

fn status_line(status: Option<&ComputerHostStatus>, error: Option<&str>) -> String {
    if let Some(error) = error {
        return format!("Status unavailable: {error}");
    }
    let Some(status) = status else {
        return "Checking the desktop host…".into();
    };
    let host = match status.host_bundle_id.as_deref() {
        Some(bundle) => format!("Host {bundle}"),
        None => "No desktop host registered".into(),
    };
    let driver = match status.driver_native_revision {
        Some(revision) => format!("Driver revision {revision}"),
        None => format!("Driver not started (bundled revision {CUA_NATIVE_REVISION})"),
    };
    format!("{host} · {driver}")
}

#[derive(Default)]
struct HistoryUi {
    open: bool,
    pages: Vec<ComputerGetAuditHistoryResult>,
    loading: bool,
    error: bool,
    task: Option<Task<()>>,
}

pub struct ComputerPage {
    state: Entity<AppState>,
    scroll: widgets::PageScroll,
    settings: Option<ComputerSettings>,
    settings_error: Option<SharedString>,
    load: Option<Task<()>>,
    save: Option<Task<()>>,
    status: Option<ComputerHostStatus>,
    status_error: Option<SharedString>,
    status_task: Option<Task<()>>,
    status_poll: Option<Task<()>>,
    grants: Rc<dyn Fn() -> AppshotCapabilities>,
    capabilities: AppshotCapabilities,
    native: bool,
    guide: PermissionGuide,
    notice: Option<SharedString>,
    setup_error: Option<SharedString>,
    advanced_open: bool,
    guide_requested: bool,
    history: HistoryUi,
    fill_input: Entity<ComposerInput>,
    rim_input: Entity<ComposerInput>,
    _inputs: [Subscription; 2],
}

impl PermissionGuideHost for ComputerPage {
    fn permission_guide(&mut self) -> &mut PermissionGuide {
        &mut self.guide
    }

    fn guide_opening(&mut self) {
        self.notice = None;
    }
}

#[derive(Clone, Copy)]
enum CursorChannel {
    Fill,
    Rim,
}

impl ComputerPage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let input = |cx: &mut Context<Self>| {
            cx.new(|cx| {
                ComposerInput::with_context("#rrggbb", "PaletteSearch", cx)
                    .with_text_metrics(12.0, 16.0)
                    .with_single_line()
                    .with_accessibility_role(gpui::Role::TextInput)
            })
        };
        let fill_input = input(cx);
        let rim_input = input(cx);
        let subscribe = |input: &Entity<ComposerInput>, channel, cx: &mut Context<Self>| {
            cx.subscribe(input, move |this: &mut Self, _, event, cx| {
                if matches!(event, ComposerInputEvent::Edited) {
                    this.cursor_color_edited(channel, cx);
                }
            })
        };
        let inputs = [
            subscribe(&fill_input, CursorChannel::Fill, cx),
            subscribe(&rim_input, CursorChannel::Rim, cx),
        ];
        let grants: Rc<dyn Fn() -> AppshotCapabilities> = Rc::new(crate::appshots::capabilities);
        let mut page = Self {
            state,
            scroll: widgets::PageScroll::default(),
            settings: None,
            settings_error: None,
            load: None,
            save: None,
            status: None,
            status_error: None,
            status_task: None,
            status_poll: None,
            capabilities: grants(),
            grants,
            native: true,
            guide: PermissionGuide::default(),
            notice: None,
            setup_error: None,
            advanced_open: false,
            guide_requested: false,
            history: HistoryUi::default(),
            fill_input,
            rim_input,
            _inputs: inputs,
        };
        page.refresh(cx);
        page
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        state: Entity<AppState>,
        grants: Rc<dyn Fn() -> AppshotCapabilities>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut page = Self::new(state, cx);
        page.capabilities = grants();
        page.grants = grants;
        page.native = false;
        page
    }

    fn editable(&self) -> bool {
        cfg!(target_os = "macos") && self.settings.is_some()
    }

    fn missing(&self) -> Vec<ComputerPermission> {
        if !cfg!(target_os = "macos") {
            return Vec::new();
        }
        crate::computer::cards::missing_computer_permissions(self.capabilities)
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load_settings(cx);
        self.refresh_status(cx);
    }

    fn load_settings(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.load = Some(cx.spawn(async move |this, cx| {
            let reply = engine
                .client()
                .call_as::<ComputerSettings>(methods::GET_COMPUTER_SETTINGS, serde_json::json!({}))
                .await;
            this.update(cx, |page, cx| {
                page.load = None;
                match reply {
                    Ok(settings) => {
                        page.settings_error = None;
                        page.adopt_settings(settings, cx);
                    }
                    Err(error) => page.settings_error = Some(error.to_string().into()),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn adopt_settings(&mut self, settings: ComputerSettings, cx: &mut Context<Self>) {
        for (input, value) in [
            (&self.fill_input, &settings.agent_cursor_fill_color),
            (&self.rim_input, &settings.agent_cursor_rim_color),
        ] {
            if input.read(cx).text() != value {
                input.update(cx, |input, cx| input.set_text(value.clone(), cx));
            }
        }
        self.state.update(cx, |state, _| {
            state.computer_settings = Some(settings.clone())
        });
        self.settings = Some(settings);
    }

    pub fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if active == self.status_poll.is_some() {
            return;
        }
        if active && self.settings.is_some() && self.load.is_none() {
            self.load_settings(cx);
        }
        self.status_poll = active.then(|| {
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(STATUS_POLL_INTERVAL).await;
                    let polled = this.update(cx, |page, cx| {
                        if page.status_task.is_none() {
                            page.refresh_status(cx);
                        }
                    });
                    if polled.is_err() {
                        return;
                    }
                }
            })
        });
    }

    fn refresh_status(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.status_task = Some(cx.spawn(async move |this, cx| {
            let reply = engine
                .client()
                .call_as::<ComputerHostStatus>(methods::GET_COMPUTER_STATUS, serde_json::json!({}))
                .await;
            this.update(cx, |page, cx| {
                page.status_task = None;
                match reply {
                    Ok(status) => {
                        page.status = Some(status);
                        page.status_error = None;
                    }
                    Err(error) => page.status_error = Some(error.to_string().into()),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn write_settings(
        &mut self,
        cx: &mut Context<Self>,
        mutate: impl FnOnce(&mut ComputerSettings),
    ) {
        if !self.editable() {
            return;
        }
        let Some(previous) = self.settings.clone() else {
            return;
        };
        let mut next = previous.clone();
        mutate(&mut next);
        if next == previous {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        #[cfg(target_os = "macos")]
        if !next.browser_developer_access {
            crate::browser::control_host::set_developer_blocked(true, cx);
        }
        self.settings = Some(next.clone());
        self.settings_error = None;
        let params = serde_json::to_value(&next).unwrap_or_default();
        self.save = Some(cx.spawn(async move |this, cx| {
            let reply = engine
                .client()
                .call(methods::SET_COMPUTER_SETTINGS, params)
                .await;
            this.update(cx, |page, cx| {
                page.save = None;
                match reply {
                    Ok(_) => {
                        #[cfg(target_os = "macos")]
                        crate::browser::control_host::set_developer_blocked(
                            !next.browser_developer_access,
                            cx,
                        );
                        page.state
                            .update(cx, |state, _| state.computer_settings = Some(next.clone()));
                        if next.agent_cursor_colors() != previous.agent_cursor_colors() {
                            #[cfg(target_os = "macos")]
                            crate::computer::registration::apply_cursor_colors(
                                next.agent_cursor_colors(),
                                cx,
                            );
                        }
                        if next.computer_control_enabled != previous.computer_control_enabled {
                            page.refresh_status(cx);
                        }
                    }
                    Err(error) => {
                        if page.settings.as_ref() == Some(&next) {
                            #[cfg(target_os = "macos")]
                            crate::browser::control_host::set_developer_blocked(
                                !previous.browser_developer_access,
                                cx,
                            );
                            page.adopt_settings(previous, cx);
                        }
                        page.settings_error = Some(format!("Could not save: {error}").into());
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn set_control(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.write_settings(cx, |settings| settings.computer_control_enabled = enabled);
    }

    fn cursor_color_edited(&mut self, channel: CursorChannel, cx: &mut Context<Self>) {
        let input = match channel {
            CursorChannel::Fill => &self.fill_input,
            CursorChannel::Rim => &self.rim_input,
        };
        let draft = input.read(cx).text().trim().to_string();
        let value = if draft.is_empty() {
            String::new()
        } else {
            match normalize_cursor_hex_color(&draft) {
                normalized if normalized.is_empty() => return cx.notify(),
                normalized => normalized,
            }
        };
        self.write_settings(cx, |settings| match channel {
            CursorChannel::Fill => settings.agent_cursor_fill_color = value,
            CursorChannel::Rim => settings.agent_cursor_rim_color = value,
        });
    }

    fn reset_cursor_colors(&mut self, cx: &mut Context<Self>) {
        let defaults = ComputerSettings::default();
        self.write_settings(cx, |settings| {
            settings.agent_cursor_color_mode = defaults.agent_cursor_color_mode;
            settings.agent_cursor_fill_color = defaults.agent_cursor_fill_color;
            settings.agent_cursor_rim_color = defaults.agent_cursor_rim_color;
        });
        if let Some(settings) = self.settings.clone() {
            self.adopt_settings(settings, cx);
        }
    }

    pub fn open_setup_guide(&mut self, missing: &[ComputerPermission], cx: &mut Context<Self>) {
        self.capabilities = (self.grants)();
        let Some(pane) = next_setup_pane(missing) else {
            return;
        };
        self.notice = None;
        self.advanced_open = true;
        if self.native {
            self.guide.request_once(pane);
            crate::appshots::refresh_registration();
        }
        self.guide.open(pane, cx);
    }

    fn set_up(&mut self, cx: &mut Context<Self>) {
        self.setup_error = None;
        #[cfg(target_os = "macos")]
        if self.native
            && let Err(message) = crate::computer::native::permissions::start_permission_setup()
        {
            self.setup_error = Some(message.into());
        }
        self.capabilities = (self.grants)();
        let missing = self.missing();
        self.open_setup_guide(&missing, cx);
        self.refresh_status(cx);
        cx.notify();
    }

    fn acknowledged(cx: &gpui::App) -> bool {
        crate::settings::current(cx).computer_getting_started_acknowledged
    }

    fn dismiss_getting_started(&mut self, cx: &mut Context<Self>) {
        self.guide_requested = false;
        if !Self::acknowledged(cx) {
            crate::settings::update(SavePolicy::Debounced, cx, |settings| {
                settings.computer_getting_started_acknowledged = true;
            });
        }
        cx.notify();
    }

    fn toggle_guide(&mut self, guide_open: bool, cx: &mut Context<Self>) {
        if guide_open {
            self.dismiss_getting_started(cx);
        } else {
            self.guide_requested = true;
            cx.notify();
        }
    }

    fn toggle_advanced(&mut self, cx: &mut Context<Self>) {
        self.advanced_open = !self.advanced_open;
        cx.notify();
    }

    fn toggle_history(&mut self, cx: &mut Context<Self>) {
        self.history.open = !self.history.open;
        if self.history.open && self.history.pages.is_empty() && self.history.task.is_none() {
            self.load_history(None, cx);
        }
        cx.notify();
    }

    fn refresh_history(&mut self, cx: &mut Context<Self>) {
        self.history.pages.clear();
        self.load_history(None, cx);
    }

    fn load_older_history(&mut self, cx: &mut Context<Self>) {
        if let Some(input) = next_history_page(&self.history.pages) {
            self.load_history(Some(input), cx);
        }
    }

    fn load_history(
        &mut self,
        input: Option<ComputerGetAuditHistoryInput>,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let input = input.unwrap_or(ComputerGetAuditHistoryInput {
            limit: Some(HISTORY_PAGE_SIZE),
            before: None,
        });
        let first = input.before.is_none();
        self.history.loading = true;
        self.history.error = false;
        self.history.task = Some(cx.spawn(async move |this, cx| {
            let reply = engine
                .client()
                .call_as::<ComputerGetAuditHistoryResult>(
                    methods::GET_COMPUTER_AUDIT_HISTORY,
                    serde_json::to_value(&input).unwrap_or_default(),
                )
                .await;
            this.update(cx, |page, cx| {
                page.history.loading = false;
                page.history.task = None;
                match reply {
                    Ok(result) => {
                        if first {
                            page.history.pages.clear();
                        }
                        page.history.pages.push(result);
                    }
                    Err(_) => page.history.error = true,
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn on_scroll_hovered(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.scroll.set_list_hovered(*hovered) {
            cx.notify();
        }
    }

    #[cfg(test)]
    pub(crate) fn guide_state(&self) -> &PermissionGuide {
        &self.guide
    }

    #[cfg(test)]
    pub(crate) fn set_advanced_open(&mut self, open: bool) {
        self.advanced_open = open;
    }

    #[cfg(test)]
    pub(crate) fn status_polling(&self) -> bool {
        self.status_poll.is_some()
    }

    #[cfg(test)]
    pub(crate) fn settings_value(&self) -> Option<&ComputerSettings> {
        self.settings.as_ref()
    }
}

#[cfg(feature = "appshots-fixture")]
impl ComputerPage {
    pub fn fixture_grants(&mut self, grants: AppshotCapabilities) {
        self.grants = Rc::new(move || grants);
        self.native = false;
    }

    pub fn fixture_status(&mut self, status: ComputerHostStatus) {
        self.status = Some(status);
        self.status_error = None;
        self.status_task = None;
        // Counts as active so no poll replaces the injected status.
        self.status_poll = Some(Task::ready(()));
    }

    pub fn fixture_history(&mut self, pages: Vec<ComputerGetAuditHistoryResult>) {
        self.history.open = true;
        self.history.pages = pages;
        self.history.task = None;
        self.history.loading = false;
    }

    pub fn fixture_view(&mut self, advanced: bool, scroll_y: f32, cx: &mut Context<Self>) {
        self.advanced_open = advanced;
        self.scroll
            .scroll
            .set_offset(gpui::point(px(0.0), px(-scroll_y)));
        cx.notify();
    }

    pub fn fixture_toggle(&mut self, cx: &mut Context<Self>) {
        let next = !self
            .settings
            .as_ref()
            .is_some_and(|settings| settings.computer_control_enabled);
        self.set_control(next, cx);
    }

    pub fn fixture_set_up(&mut self, cx: &mut Context<Self>) {
        self.set_up(cx);
    }
}

impl popover::ScrollRailHost for ComputerPage {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        self.scroll.rail_bar()
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.scroll.rail_scroll()
    }
}

fn disclosure(
    theme: &Theme,
    id: &'static str,
    open: bool,
    shown: &'static str,
    hidden: &'static str,
) -> gpui::Stateful<gpui::Div> {
    let accent = theme.accent;
    widgets::ghost_action(theme)
        .id(id)
        .debug_selector(move || id.to_string())
        .role(gpui::Role::Button)
        .aria_label(if open { shown } else { hidden })
        .aria_expanded(open)
        .tab_index(0)
        .focus_visible(move |style| style.border_2().border_color(accent))
        .py(px(4.0))
        .child(
            crate::icons::icon(if open {
                icons::ALT_ARROW_DOWN
            } else {
                icons::ALT_ARROW_RIGHT
            })
            .size(px(12.0))
            .text_color(theme.text_muted),
        )
        .child(if open { shown } else { hidden })
}

fn section_header(theme: &Theme, title: &'static str, action: Option<AnyElement>) -> gpui::Div {
    div()
        .mt(px(24.0))
        .px(px(8.0))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.0))
        .child(
            div()
                .py(px(4.0))
                .text_size(crate::typography::ui_rems(13.0))
                .text_color(theme.text_muted.opacity(0.58))
                .child(SharedString::from(title)),
        )
        .children(action)
}

fn muted_line(theme: &Theme, copy: impl Into<SharedString>) -> gpui::Div {
    div()
        .px(px(8.0))
        .pt(px(8.0))
        .text_size(crate::typography::ui_rems(12.0))
        .line_height(px(18.0))
        .text_color(theme.text_muted)
        .child(copy.into())
}

impl ComputerPage {
    fn render_history(&mut self, theme: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = vec![
            muted_line(
                theme,
                "Recent actions across chats on this engine. Read-only observations, typed text, page contents, and file paths are not included.",
            )
            .into_any_element(),
        ];
        let first = self.history.pages.first();
        if self.history.loading && first.is_none() {
            out.push(muted_line(theme, "Loading recent actions…").into_any_element());
        }
        if self.history.error {
            out.push(
                muted_line(
                    theme,
                    "Could not load recent actions. Refresh to try again.",
                )
                .text_color(theme.danger)
                .into_any_element(),
            );
        }
        if let Some(status) = first.map(|page| page.status) {
            let entries = history_entries(&self.history.pages);
            let truncated = self.history.pages.iter().any(|page| page.truncated);
            if entries.is_empty() {
                let copy = if status == ComputerAuditHistoryStatus::Disabled {
                    "Action history is not enabled on this engine."
                } else if truncated {
                    "Older retained actions are no longer available. Refresh to see recent actions."
                } else {
                    "No recorded actions yet."
                };
                out.push(muted_line(theme, copy).into_any_element());
            } else {
                let rows = entries.iter().enumerate().map(|(ix, entry)| {
                    widgets::card_row(theme, ix == 0)
                        .debug_selector(move || format!("computer-history-row-{ix}"))
                        .child(
                            crate::icons::icon(icons::CURSOR)
                                .size(px(16.0))
                                .flex_none()
                                .text_color(theme.text_muted),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(widgets::row_title(theme, history_title(entry)))
                                .child(
                                    div()
                                        .text_size(crate::typography::ui_rems(12.0))
                                        .text_color(theme.text_muted)
                                        .child(history_time(&entry.ts)),
                                ),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(crate::typography::ui_rems(11.0))
                                .text_color(theme.text_muted)
                                .child(effect_label(entry.effect)),
                        )
                        .into_any_element()
                });
                out.push(
                    widgets::section_card(theme)
                        .id("computer-history-list")
                        .mt(px(8.0))
                        .role(gpui::Role::List)
                        .aria_label("Recent Computer actions")
                        .children(rows.collect::<Vec<_>>())
                        .into_any_element(),
                );
                if truncated {
                    out.push(
                        muted_line(
                            theme,
                            "Some earlier actions are unavailable. This is not a complete history.",
                        )
                        .debug_selector(|| "computer-history-truncated".into())
                        .into_any_element(),
                    );
                }
                if entries.len() >= COMPUTER_AUDIT_HISTORY_MAX_LIMIT as usize {
                    out.push(
                        muted_line(
                            theme,
                            format!(
                                "Showing the latest {COMPUTER_AUDIT_HISTORY_MAX_LIMIT} loaded actions."
                            ),
                        )
                        .into_any_element(),
                    );
                }
            }
        }
        let accent = theme.accent;
        let busy = self.history.task.is_some();
        let mut buttons = div()
            .px(px(8.0))
            .pt(px(10.0))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.0))
            .child(
                outline_button(theme)
                    .id("computer-history-refresh")
                    .debug_selector(|| "computer-history-refresh".into())
                    .role(gpui::Role::Button)
                    .tab_index(0)
                    .focus_visible(move |style| style.border_2().border_color(accent))
                    .when(busy, |el| el.opacity(0.5))
                    .when(!busy, |el| {
                        el.on_click(cx.listener(|this, _, _, cx| this.refresh_history(cx)))
                    })
                    .child("Refresh history"),
            );
        if next_history_page(&self.history.pages).is_some() && !self.history.error {
            buttons = buttons.child(
                outline_button(theme)
                    .id("computer-history-older")
                    .debug_selector(|| "computer-history-older".into())
                    .role(gpui::Role::Button)
                    .tab_index(0)
                    .focus_visible(move |style| style.border_2().border_color(accent))
                    .when(busy, |el| el.opacity(0.5))
                    .when(!busy, |el| {
                        el.on_click(cx.listener(|this, _, _, cx| this.load_older_history(cx)))
                    })
                    .child(if busy {
                        "Loading…"
                    } else {
                        "Load older actions"
                    }),
            );
        }
        out.push(buttons.into_any_element());
        out
    }

    fn render_allowed_apps(
        &self,
        theme: &Theme,
        settings: &ComputerSettings,
        editable: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if settings.always_allowed_apps.is_empty() {
            return muted_line(
                theme,
                "No apps yet. Choose Always allow on a Computer consent card to skip that app's card in future tasks.",
            )
            .debug_selector(|| "computer-allowed-apps-empty".into())
            .into_any_element();
        }
        let accent = theme.accent;
        let rows = settings
            .always_allowed_apps
            .iter()
            .enumerate()
            .map(|(ix, app)| {
                let title = app
                    .name
                    .clone()
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| app.bundle_id.clone());
                let icon =
                    crate::appshots::presentation_icon(&crate::appshots::AppshotPresentation {
                        app_name: title.clone(),
                        window_title: None,
                        bundle_identifier: Some(app.bundle_id.clone()),
                    });
                let bundle_id = app.bundle_id.clone();
                let remove_id = format!("computer-allowed-app-remove-{}", app.bundle_id);
                widgets::card_row(theme, ix == 0)
                    .debug_selector(move || format!("computer-allowed-app-{ix}"))
                    .child(match icon {
                        Some(icon) => img(icon)
                            .size(px(20.0))
                            .flex_none()
                            .object_fit(ObjectFit::Contain)
                            .into_any_element(),
                        None => crate::icons::icon(icons::MONITOR)
                            .size(px(16.0))
                            .flex_none()
                            .text_color(theme.text_muted)
                            .into_any_element(),
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(widgets::row_title(theme, title.clone()))
                            .when(app.name.is_some(), |column| {
                                column.child(
                                    div()
                                        .truncate()
                                        .text_size(crate::typography::ui_rems(12.0))
                                        .text_color(theme.text_muted)
                                        .child(SharedString::from(app.bundle_id.clone())),
                                )
                            }),
                    )
                    .child(
                        widgets::ghost_action(theme)
                            .id(SharedString::from(remove_id.clone()))
                            .debug_selector(move || remove_id.clone())
                            .role(gpui::Role::Button)
                            .aria_label(format!("Remove {title} from always-allowed apps"))
                            .tab_index(0)
                            .focus_visible(move |style| style.border_2().border_color(accent))
                            .flex_none()
                            .size(px(24.0))
                            .p(px(0.0))
                            .rounded(px(6.0))
                            .justify_center()
                            .when(!editable, |el| el.opacity(0.4))
                            .when(editable, |el| {
                                el.cursor_pointer()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.remove_allowed_app(&bundle_id, cx)
                                    }))
                            })
                            .child(
                                crate::icons::icon(icons::TRASH_BIN_MINIMALISTIC)
                                    .size(px(14.0))
                                    .text_color(theme.text_muted),
                            ),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        widgets::section_card(theme)
            .id("computer-allowed-apps")
            .mt(px(8.0))
            .role(gpui::Role::List)
            .aria_label("Always-allowed apps")
            .children(rows)
            .into_any_element()
    }

    fn remove_allowed_app(&mut self, bundle_id: &str, cx: &mut Context<Self>) {
        self.write_settings(cx, |settings| {
            settings
                .always_allowed_apps
                .retain(|app| app.bundle_id != bundle_id)
        });
    }

    fn render_cursor_fields(&self, theme: &Theme, settings: &ComputerSettings) -> AnyElement {
        let field = |label: &'static str, value: &str, input: &Entity<ComposerInput>| {
            let swatch = normalize_cursor_hex_color(value);
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .w(px(28.0))
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.text_muted)
                        .child(label),
                )
                .child(
                    div()
                        .size(px(16.0))
                        .flex_none()
                        .rounded_full()
                        .border_1()
                        .border_color(theme.border)
                        .when_some(
                            (!swatch.is_empty())
                                .then(|| gpui::Rgba::try_from(swatch.as_str()).ok())
                                .flatten(),
                            |el, color| el.bg(color),
                        ),
                )
                .child(
                    div()
                        .w(px(96.0))
                        .h(px(26.0))
                        .px(px(8.0))
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(theme.border)
                        .flex()
                        .items_center()
                        .child(input.clone()),
                )
        };
        div()
            .px(px(20.0))
            .pb(px(14.0))
            .flex()
            .flex_wrap()
            .gap(px(16.0))
            .child(field(
                "Fill",
                &settings.agent_cursor_fill_color,
                &self.fill_input,
            ))
            .child(field(
                "Rim",
                &settings.agent_cursor_rim_color,
                &self.rim_input,
            ))
            .into_any_element()
    }
}

impl Render for ComputerPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        self.guide.rendered.clear();
        let theme = Theme::of(cx).clone();
        let accent = theme.accent;
        self.capabilities = (self.grants)();
        if let Some(pane) = self.guide.close_if_granted(self.capabilities, cx) {
            self.notice = Some(
                format!(
                    "Permission granted. {} is ready for Computer control.",
                    pane.label()
                )
                .into(),
            );
            self.refresh_status(cx);
        }
        let editable = self.editable();
        let settings = self.settings.clone().unwrap_or_default();
        let missing = self.missing();
        let view = availability_view(self.status.as_ref(), &missing);
        let setup = self.setup_error.is_some() || needs_setup(self.status.as_ref(), &missing);
        let reconnecting = self
            .status
            .as_ref()
            .is_some_and(|s| s.status.health.status == ComputerHealthStatus::Reconnecting);
        let attention = self.setup_error.is_some()
            || view.kind == ViewKind::Blocked
            || (view.kind == ViewKind::Checking && (setup || reconnecting));

        let toggle = widgets::toggle_switch(
            &theme,
            settings.computer_control_enabled,
            "computer-control-toggle",
        )
            .id("computer-control-toggle")
            .debug_selector(|| "computer-control-toggle".into())
            .role(gpui::Role::Switch)
            .aria_label("Let the agent use the desktop in any chat")
            .aria_toggled(if settings.computer_control_enabled {
                gpui::Toggled::True
            } else {
                gpui::Toggled::False
            })
            .tab_index(0)
            .focus_visible(move |style| style.border_2().border_color(accent))
            .when(!editable, |el| el.opacity(0.4))
            .when(editable, |el| {
                el.cursor_pointer().on_click(cx.listener(|this, _, _, cx| {
                    let next = !this
                        .settings
                        .as_ref()
                        .is_some_and(|s| s.computer_control_enabled);
                    this.set_control(next, cx);
                }))
            });

        let developer_toggle = widgets::toggle_switch(
            &theme,
            settings.browser_developer_access,
            "browser-developer-access",
        )
        .id("browser-developer-access")
        .debug_selector(|| "browser-developer-access".into())
        .role(gpui::Role::Switch)
        .aria_label("Developer access for Zeron browser")
        .aria_toggled(if settings.browser_developer_access {
            gpui::Toggled::True
        } else {
            gpui::Toggled::False
        })
        .when(!editable, |el| el.opacity(0.4))
        .when(editable, |el| {
            el.cursor_pointer().on_click(cx.listener(|this, _, _, cx| {
                this.write_settings(cx, |settings| {
                    settings.browser_developer_access = !settings.browser_developer_access
                });
            }))
        });

        let dot = match view.kind {
            ViewKind::Ready => theme.success,
            ViewKind::Checking => theme.warning,
            ViewKind::Blocked => theme.danger,
        };
        let status_row = div()
            .px(px(8.0))
            .pt(px(8.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .text_size(crate::typography::ui_rems(12.0))
            .text_color(theme.text_muted)
            .child(div().size(px(6.0)).flex_none().rounded_full().bg(dot))
            .child(SharedString::from(view.title.clone()))
            .child("·")
            .child(SharedString::from(status_line(
                self.status.as_ref(),
                self.status_error.as_deref(),
            )));

        let mut card = widgets::section_card(&theme).mt(px(12.0));
        let mut first = true;
        if attention {
            let set_up = setup.then(|| {
                outline_button(&theme)
                    .id("computer-set-up")
                    .debug_selector(|| "computer-set-up".into())
                    .role(gpui::Role::Button)
                    .tab_index(0)
                    .focus_visible(move |style| style.border_2().border_color(accent))
                    .on_click(cx.listener(|this, _, _, cx| this.set_up(cx)))
                    .child("Set up")
                    .into_any_element()
            });
            let (title, description) = match &self.setup_error {
                Some(error) => (
                    "Computer permission setup needs attention".to_string(),
                    error.to_string(),
                ),
                None => (view.title.clone(), view.description.clone()),
            };
            let reconnects = self
                .status
                .as_ref()
                .map(|s| s.status.health.reconnects)
                .filter(|count| *count > 0)
                .map(|count| {
                    SharedString::from(if count == 1 {
                        "Reconnected once since startup.".to_string()
                    } else {
                        format!("Reconnected {count} times since startup.")
                    })
                });
            card = card.child(
                row(
                    &theme,
                    first,
                    title,
                    &description,
                    reconnects,
                    Some(
                        div()
                            .size(px(8.0))
                            .rounded_full()
                            .bg(dot)
                            .into_any_element(),
                    ),
                    set_up.unwrap_or_else(|| div().into_any_element()),
                    None,
                )
                .debug_selector(|| "computer-attention-row".into()),
            );
            first = false;
        }
        let custom = settings.agent_cursor_color_mode == AgentCursorColorMode::Custom;
        let cursor_dirty = settings.agent_cursor_color_mode != AgentCursorColorMode::Stock
            || !settings.agent_cursor_fill_color.is_empty()
            || !settings.agent_cursor_rim_color.is_empty();
        let chip = |id: &'static str, label: &'static str, selected: bool| {
            choice_button(&theme, id, label, selected)
                .debug_selector(move || id.to_string())
                .when(!editable, |el| el.opacity(0.5))
        };
        let reset = |id: &'static str, label: &'static str| {
            widgets::ghost_action(&theme)
                .id(id)
                .debug_selector(move || id.to_string())
                .role(gpui::Role::Button)
                .aria_label(format!("Reset {label} to default"))
                .tab_index(0)
                .focus_visible(move |style| style.border_2().border_color(accent))
                .flex_none()
                .size(px(20.0))
                .p(px(0.0))
                .rounded(px(6.0))
                .justify_center()
                .child(
                    crate::icons::icon(icons::RESTART)
                        .size(px(12.0))
                        .text_color(theme.text_muted),
                )
        };
        let mode_control = div()
            .flex()
            .gap(px(6.0))
            .child(
                chip("computer-cursor-stock", "Stock", !custom).when(editable, |el| {
                    el.on_click(cx.listener(|this, _, _, cx| {
                        this.write_settings(cx, |s| {
                            s.agent_cursor_color_mode = AgentCursorColorMode::Stock
                        })
                    }))
                }),
            )
            .child(
                chip("computer-cursor-custom", "Custom", custom).when(editable, |el| {
                    el.on_click(cx.listener(|this, _, _, cx| {
                        this.write_settings(cx, |s| {
                            s.agent_cursor_color_mode = AgentCursorColorMode::Custom
                        })
                    }))
                }),
            );
        card = card.child(row(
            &theme,
            first,
            "Cursor colors",
            "The agent pointer is stock monochrome by default — like a normal pointer. Custom colors apply to new computer sessions.",
            None,
            (cursor_dirty && editable).then(|| {
                reset("computer-cursor-reset", "cursor colors")
                    .on_click(cx.listener(|this, _, _, cx| this.reset_cursor_colors(cx)))
                    .into_any_element()
            }),
            mode_control.into_any_element(),
            custom.then(|| self.render_cursor_fields(&theme, &settings)),
        ));
        let defaults = ComputerSettings::default();
        let preview_dirty = settings.auto_open_computer_pane != defaults.auto_open_computer_pane
            || settings.computer_preview_size != defaults.computer_preview_size;
        let large = settings.computer_preview_size == ComputerPreviewSize::Large;
        let preview_control = div()
            .flex()
            .items_center()
            .gap(px(12.0))
            .child(
                widgets::toggle_switch(
                    &theme,
                    settings.auto_open_computer_pane,
                    "computer-preview-auto",
                )
                    .id("computer-preview-auto")
                    .debug_selector(|| "computer-preview-auto".into())
                    .role(gpui::Role::Switch)
                    .aria_label(
                        "Show the computer preview automatically when an agent drives the desktop",
                    )
                    .aria_toggled(if settings.auto_open_computer_pane {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .tab_index(0)
                    .focus_visible(move |style| style.border_2().border_color(accent))
                    .when(!editable, |el| el.opacity(0.4))
                    .when(editable, |el| {
                        el.cursor_pointer().on_click(cx.listener(|this, _, _, cx| {
                            this.write_settings(cx, |s| {
                                s.auto_open_computer_pane = !s.auto_open_computer_pane
                            })
                        }))
                    }),
            )
            .child(
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(chip("computer-preview-compact", "Compact", !large).when(
                        editable,
                        |el| {
                            el.on_click(cx.listener(|this, _, _, cx| {
                                this.write_settings(cx, |s| {
                                    s.computer_preview_size = ComputerPreviewSize::Compact
                                })
                            }))
                        },
                    ))
                    .child(
                        chip("computer-preview-large", "Large", large).when(editable, |el| {
                            el.on_click(cx.listener(|this, _, _, cx| {
                                this.write_settings(cx, |s| {
                                    s.computer_preview_size = ComputerPreviewSize::Large
                                })
                            }))
                        }),
                    ),
            );
        card = card.child(row(
            &theme,
            false,
            "Preview",
            "Show the live preview the first time an agent acts on the desktop in a chat. Compact keeps it small and glanceable; Large gives it the full wide card.",
            None,
            (preview_dirty && editable).then(|| {
                reset("computer-preview-reset", "preview")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let defaults = ComputerSettings::default();
                        this.write_settings(cx, |s| {
                            s.auto_open_computer_pane = defaults.auto_open_computer_pane;
                            s.computer_preview_size = defaults.computer_preview_size;
                        })
                    }))
                    .into_any_element()
            }),
            preview_control.into_any_element(),
            None,
        ));
        let rail = settings.computer_preview_placement == ComputerPreviewPlacement::Rail;
        let placement_control = div()
            .flex()
            .gap(px(6.0))
            .child(
                chip("computer-placement-card", "Floating card", !rail).when(editable, |el| {
                    el.on_click(cx.listener(|this, _, _, cx| {
                        this.write_settings(cx, |s| {
                            s.computer_preview_placement = ComputerPreviewPlacement::Card
                        })
                    }))
                }),
            )
            .child(
                chip("computer-placement-rail", "Rail + thumbnail", rail).when(editable, |el| {
                    el.on_click(cx.listener(|this, _, _, cx| {
                        this.write_settings(cx, |s| {
                            s.computer_preview_placement = ComputerPreviewPlacement::Rail
                        })
                    }))
                }),
            );
        card = card.child(row(
            &theme,
            false,
            "Placement",
            "Floating card sits over the transcript. Rail puts a small thumbnail under Computer in the right rail; click it to pop out.",
            None,
            (settings.computer_preview_placement != defaults.computer_preview_placement
                && editable)
                .then(|| {
                    reset("computer-placement-reset", "placement")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.write_settings(cx, |s| {
                                s.computer_preview_placement =
                                    ComputerSettings::default().computer_preview_placement
                            })
                        }))
                        .into_any_element()
                }),
            placement_control.into_any_element(),
            None,
        ));

        let acknowledged = Self::acknowledged(cx);
        let guide_open = !acknowledged || self.guide_requested;
        let getting_started_toggle = disclosure(
            &theme,
            "computer-getting-started-toggle",
            guide_open,
            "Hide guide",
            "Show guide",
        )
        .on_click(cx.listener(move |this, _, _, cx| this.toggle_guide(guide_open, cx)))
        .on_a11y_action(gpui::AccessibleAction::Click, {
            let page = cx.entity().downgrade();
            move |_, _, cx| {
                page.update(cx, |this, cx| this.toggle_guide(guide_open, cx))
                    .ok();
            }
        });
        let getting_started = guide_open.then(|| {
            let steps = [
                (
                    "Ask for a task",
                    "Type /computer-use followed by your task, for example: “/computer-use open Calculator and calculate 123 × 45.” This enables Computer for that request only. The Computer control switch above can enable it on every turn.",
                ),
                (
                    "Approve the task",
                    "If asked, approve Computer for the task. Use the permission guide when desktop access is missing. Zeron may still ask before consequential actions.",
                ),
                (
                    "Follow and stop",
                    "Watch the preview while the agent works. Press Esc or Stop on the strip to take the desktop back; the agent then sums up. The chat's Stop button ends the turn. Closing the preview only hides it.",
                ),
            ];
            let mut section = div()
                .debug_selector(|| "computer-getting-started".into())
                .flex()
                .flex_col()
                .child(
                    widgets::section_card(&theme).mt(px(6.0)).children(
                        steps
                            .into_iter()
                            .enumerate()
                            .map(|(ix, (title, copy))| {
                                row(
                                    &theme,
                                    ix == 0,
                                    title,
                                    copy,
                                    None,
                                    None,
                                    div().into_any_element(),
                                    None,
                                )
                            })
                            .collect::<Vec<_>>(),
                    ),
                )
                .child(muted_line(
                    &theme,
                    "Appshots is separate: it attaches a window image without giving the agent control.",
                ));
            if !acknowledged {
                section = section.child(
                    div().px(px(8.0)).pt(px(10.0)).flex().child(
                        outline_button(&theme)
                            .id("computer-getting-started-done")
                            .debug_selector(|| "computer-getting-started-done".into())
                            .role(gpui::Role::Button)
                            .tab_index(0)
                            .focus_visible(move |style| style.border_2().border_color(accent))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.dismiss_getting_started(cx)
                            }))
                            .child("Got it"),
                    ),
                );
            }
            section
        });

        let history_toggle = disclosure(
            &theme,
            "computer-history-toggle",
            self.history.open,
            "Hide history",
            "Show history",
        )
        .on_click(cx.listener(|this, _, _, cx| this.toggle_history(cx)))
        .on_a11y_action(gpui::AccessibleAction::Click, {
            let page = cx.entity().downgrade();
            move |_, _, cx| {
                page.update(cx, |this, cx| this.toggle_history(cx)).ok();
            }
        });
        let history = self.history.open.then(|| self.render_history(&theme, cx));
        let allowed_apps = self.render_allowed_apps(&theme, &settings, editable, cx);

        let advanced_toggle = disclosure(
            &theme,
            "computer-advanced-toggle",
            self.advanced_open,
            "Hide",
            "Show",
        )
        .on_click(cx.listener(|this, _, _, cx| this.toggle_advanced(cx)))
        .on_a11y_action(gpui::AccessibleAction::Click, {
            let page = cx.entity().downgrade();
            move |_, _, cx| {
                page.update(cx, |this, cx| this.toggle_advanced(cx)).ok();
            }
        });
        let advanced = self.advanced_open.then(|| {
            let mut column = div().flex().flex_col();
            if cfg!(target_os = "macos") {
                let capabilities = self.capabilities;
                let rows = permission_guide::permission_rows(
                    self,
                    "computer",
                    &COMPUTER_PANES,
                    capabilities,
                    &theme,
                    window,
                    cx,
                );
                column = column
                    .child(muted_line(&theme, "macOS permissions"))
                    .child(widgets::section_card(&theme).mt(px(6.0)).children(rows));
                if let Some(notice) = self.notice.clone() {
                    column = column.child(muted_line(&theme, notice));
                }
            }
            let driver = match self.status.as_ref().and_then(|s| s.driver_native_revision) {
                Some(revision) if revision == CUA_NATIVE_REVISION => {
                    format!("Revision {revision}, matching this build.")
                }
                Some(revision) => format!(
                    "Revision {revision}; this build expects revision {CUA_NATIVE_REVISION}."
                ),
                None => format!(
                    "Not started yet. The bundled driver is revision {CUA_NATIVE_REVISION}."
                ),
            };
            let mut details = widgets::section_card(&theme).mt(px(12.0)).child(row(
                &theme,
                true,
                "Desktop driver",
                &driver,
                self.status
                    .as_ref()
                    .and_then(|s| s.host_bundle_id.clone())
                    .map(|bundle| format!("Registered host: {bundle}").into()),
                None,
                div().into_any_element(),
                None,
            ));
            if let Some(status) = self.status.as_ref().filter(|_| view.kind == ViewKind::Ready) {
                let capture_blocked = !status.status.health.capture_available
                    && status.status.health.status == ComputerHealthStatus::Connected;
                let backend = match &status.status.availability {
                    ComputerAvailability::Available {
                        backend: Some(backend),
                    } if backend == "cua" => format!("Cua {CUA_DRIVER_VERSION}"),
                    ComputerAvailability::Available {
                        backend: Some(backend),
                    } => backend.clone(),
                    _ => "No backend".to_string(),
                };
                details = details.child(row(
                    &theme,
                    false,
                    "Desktop abilities",
                    "The agent shares your Mac desktop and works in the background by default. It can bring a window forward when your task asks to watch. Background input may still affect focus. Press Esc or Stop on the strip to take the desktop back for the rest of the turn; the agent then sums up. Esc needs Input Monitoring. The chat's Stop button ends the turn.",
                    Some(
                        format!(
                            "{backend} · {}",
                            capability_summary(status.status.capabilities, !capture_blocked)
                        )
                        .into(),
                    ),
                    None,
                    div().into_any_element(),
                    None,
                ));
            }
            column.child(details)
        });

        let description = if editable || self.settings.is_none() {
            "Enable Computer by default in any chat. Leave this off and use /computer-use for one request without adding Computer tools to ordinary turns."
        } else {
            "Computer control is set on the Mac that runs this engine."
        };
        let scrollbar = popover::rail(self, "computer-page-scrollbar", &theme, cx);
        div()
            .id("computer-settings-page-host")
            .relative()
            .size_full()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .child(
                div()
                    .id("computer-settings-page")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll.scroll)
                    .child(
                        widgets::page_column()
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_start()
                                    .justify_between()
                                    .gap(px(24.0))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(200.0))
                                            .flex()
                                            .flex_col()
                                            .child(widgets::page_header(&theme, "Computer", None))
                                            .child(
                                                widgets::page_subtitle(
                                                    &theme,
                                                    "Let an agent see and drive this Mac's apps.",
                                                )
                                                .max_w(px(512.0))
                                                .line_height(px(20.0)),
                                            ),
                                    ),
                            )
                            .child(section_header(
                                &theme,
                                "Computer control",
                                Some(toggle.into_any_element()),
                            ))
                            .child(muted_line(&theme, description))
                            .child(status_row)
                            .children(
                                self.settings_error
                                    .clone()
                                    .map(|error| widgets::error_strip(&theme, error)),
                            )
                            .child(card)
                            .child(section_header(&theme, "Developer access", Some(developer_toggle.into_any_element())))
                            .child(muted_line(&theme, "Allow all tasks to run JavaScript and collect partial page diagnostics in their Zeron browser tabs. Page scripts can access sensitive site data. This does not enable full CDP."))
                            .child(section_header(&theme, "Always-allowed apps", None))
                            .child(muted_line(
                                &theme,
                                "In Ask mode these apps skip the per-app consent card. Other apps still ask.",
                            ))
                            .child(allowed_apps)
                            .child(section_header(
                                &theme,
                                "Getting started",
                                Some(getting_started_toggle.into_any_element()),
                            ))
                            .children(getting_started)
                            .child(section_header(
                                &theme,
                                "Recent Computer actions",
                                Some(history_toggle.into_any_element()),
                            ))
                            .children(history.unwrap_or_default())
                            .child(section_header(
                                &theme,
                                "Advanced",
                                Some(advanced_toggle.into_any_element()),
                            ))
                            .children(advanced),
                    ),
            )
            .children(scrollbar)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::{Arc, Mutex};

    use gpui::{TestAppContext, VisualTestContext};
    use serde_json::{Value, json};
    use zeron_proto::computer::ComputerPermission::{
        Accessibility, InputMonitoring, ScreenRecording,
    };

    use super::*;
    use crate::appshots::{AppshotPlatform, CapabilityState, CaptureTarget};

    fn grants(accessibility: bool, screen: bool, input: bool) -> AppshotCapabilities {
        let state = |granted: bool| {
            if granted {
                CapabilityState::Ready
            } else {
                CapabilityState::PermissionRequired
            }
        };
        AppshotCapabilities {
            platform: AppshotPlatform::MacOs,
            global_shortcut: CapabilityState::Ready,
            window_capture: state(screen),
            application_text: state(accessibility),
            input_monitoring: state(input),
            target: CaptureTarget::ActiveWindow,
        }
    }

    fn entry(id: usize, tool: &str) -> Value {
        json!({
            "id": format!("{id:064x}"),
            "ts": "2026-09-20T12:00:00.000Z",
            "tool": tool,
            "effect": "dispatched-unknown",
        })
    }

    fn page_result(ids: std::ops::Range<usize>, cursor: Option<&str>, truncated: bool) -> Value {
        json!({
            "entries": ids.map(|id| entry(id, "computer_type_text")).collect::<Vec<_>>(),
            "nextCursor": cursor,
            "truncated": truncated,
            "status": "available",
        })
    }

    fn init(cx: &mut TestAppContext, dir: &std::path::Path) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
            crate::settings::init(crate::settings::UiSettings::default(), dir, cx);
            crate::composer::init(cx, crate::settings::ComposerSendBehavior::default());
        });
        cx.executor().allow_parking();
    }

    fn wait(cx: &mut VisualTestContext, mut done: impl FnMut(&mut VisualTestContext) -> bool) {
        for _ in 0..400 {
            cx.run_until_parked();
            if done(cx) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("condition never held");
    }

    fn click(cx: &mut VisualTestContext, selector: &'static str) {
        cx.run_until_parked();
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} painted"));
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
        cx.run_until_parked();
    }

    type Calls = Arc<Mutex<Vec<(String, Value)>>>;

    fn scripted_engine(
        runtime: &tokio::runtime::Runtime,
        answer: impl Fn(&str, &Value) -> Result<Value, String> + Send + 'static,
    ) -> (crate::state::EngineHandle, Calls) {
        let (out, mut frames) = tokio::sync::mpsc::channel::<String>(64);
        let (replies, inbound) = tokio::sync::mpsc::channel::<String>(64);
        let calls: Calls = Arc::default();
        let seen = calls.clone();
        runtime.spawn(async move {
            while let Some(frame) = frames.recv().await {
                let frame: Value = serde_json::from_str(&frame).unwrap();
                let method = frame["method"].as_str().unwrap_or_default().to_string();
                let params = frame["params"].clone();
                seen.lock().unwrap().push((method.clone(), params.clone()));
                let reply = match answer(&method, &params) {
                    Ok(value) => json!({ "id": frame["id"], "ok": value }),
                    Err(error) => json!({ "id": frame["id"], "err": error }),
                };
                if replies.send(reply.to_string()).await.is_err() {
                    break;
                }
            }
            std::future::pending::<()>().await;
        });
        let client = zeron_rpc::RpcClient::new(out, inbound);
        (crate::state::EngineHandle::from_test_client(client), calls)
    }

    fn open_page(
        cx: &mut TestAppContext,
        engine: crate::state::EngineHandle,
        grants: Rc<dyn Fn() -> AppshotCapabilities>,
    ) -> (Entity<ComputerPage>, &mut VisualTestContext) {
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| state.set_test_engine(engine));
        let (page, cx) = cx.add_window_view(move |_, cx| ComputerPage::for_test(state, grants, cx));
        wait(cx, |cx| {
            page.read_with(cx, |page, _| {
                page.load.is_none() && page.status_task.is_none()
            })
        });
        (page, cx)
    }

    #[test]
    fn set_up_order_is_accessibility_then_screen_recording_then_input_monitoring() {
        assert_eq!(
            next_setup_pane(&[Accessibility, ScreenRecording, InputMonitoring]),
            Some(PermissionPane::Accessibility)
        );
        assert_eq!(
            next_setup_pane(&[ScreenRecording, InputMonitoring]),
            Some(PermissionPane::ScreenRecording)
        );
        assert_eq!(
            next_setup_pane(&[InputMonitoring]),
            Some(PermissionPane::InputMonitoring)
        );
        assert_eq!(next_setup_pane(&[]), None);
    }

    #[test]
    fn panes_follow_one_order() {
        let panes: Vec<_> = COMPUTER_PANES.iter().map(|(pane, _)| *pane).collect();
        let order: Vec<_> = crate::computer::cards::COMPUTER_PERMISSION_ORDER
            .into_iter()
            .map(PermissionPane::from_permission)
            .collect();
        assert_eq!(panes, order);
        assert_eq!(
            SETUP_ORDER,
            crate::computer::cards::COMPUTER_PERMISSION_ORDER
        );
    }

    #[test]
    fn history_rows_name_the_app() {
        let row = |app: Option<&str>| {
            let mut value = entry(0, "computer_type_text");
            if let Some(app) = app {
                value["app"] = json!(app);
            }
            history_title(&serde_json::from_value(value).unwrap())
        };
        assert_eq!(row(Some("Calculator")), "Type · Calculator");
        assert_eq!(row(Some("com.apple.calculator")), "Type · Calculator");
        assert_eq!(row(None), "Type");
    }

    #[test]
    fn history_pages_follow_the_cursor_up_to_the_cap_and_dedupe() {
        let parse = |value: Value| -> ComputerGetAuditHistoryResult {
            serde_json::from_value(value).unwrap()
        };
        let mut pages = vec![parse(page_result(0..30, Some("c1"), false))];
        assert_eq!(
            next_history_page(&pages),
            Some(ComputerGetAuditHistoryInput {
                limit: Some(30),
                before: Some("c1".into())
            })
        );
        pages.push(parse(page_result(25..55, Some("c2"), true)));
        pages.push(parse(page_result(55..85, Some("c3"), false)));
        assert_eq!(
            next_history_page(&pages).and_then(|input| input.limit),
            Some(10),
            "the last page only fills the cap"
        );
        pages.push(parse(page_result(85..95, Some("c4"), false)));
        assert_eq!(
            next_history_page(&pages),
            None,
            "100 loaded entries stop paging"
        );
        assert_eq!(
            history_entries(&pages).len(),
            95,
            "repeated ids render once"
        );
        assert_eq!(
            next_history_page(&[parse(page_result(0..3, None, false))]),
            None
        );
        let many: Vec<_> = (0..4)
            .map(|page| parse(page_result(page * 30..page * 30 + 30, Some("c"), false)))
            .collect();
        assert_eq!(history_entries(&many).len(), 100);
    }

    #[test]
    fn a_status_names_the_host_and_the_driver_revision() {
        let status: ComputerHostStatus = serde_json::from_value(json!({
            "hostBundleId": "sh.zeron.app.dev",
            "driverNativeRevision": 39,
            "status": {
                "computerId": "desktop",
                "availability": { "kind": "available", "backend": "cua" },
                "health": { "status": "connected", "consecutiveFailures": 0, "reconnects": 0, "captureAvailable": true },
                "capabilities": { "windows": true, "windowBounds": true, "stacking": true, "capture": true, "input": true, "clipboard": true, "focus": false, "raise": false, "ghostCursor": true, "visibleDesktop": true },
            },
        }))
        .unwrap();
        assert_eq!(
            status_line(Some(&status), None),
            "Host sh.zeron.app.dev · Driver revision 39"
        );
        assert_eq!(availability_view(Some(&status), &[]).kind, ViewKind::Ready);
        assert!(!needs_setup(Some(&status), &[]));
        let blocked = availability_view(Some(&status), &[Accessibility, InputMonitoring]);
        assert_eq!(blocked.kind, ViewKind::Blocked);
        assert_eq!(
            blocked.title,
            "Computer control needs Accessibility and Input Monitoring"
        );
        assert!(needs_setup(Some(&status), &[ScreenRecording]));
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn the_three_grants_render_once_in_the_shared_guide(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        init(cx, dir.path());
        let (engine, _) = scripted_engine(&runtime, |_, _| Err("offline".into()));
        let (page, cx) = open_page(cx, engine, Rc::new(|| grants(false, false, false)));
        page.update(cx, |page, cx| {
            page.set_advanced_open(true);
            cx.notify();
        });
        cx.run_until_parked();
        page.read_with(cx, |page, _| {
            assert_eq!(
                page.guide_state().rendered,
                vec![
                    PermissionPane::Accessibility,
                    PermissionPane::ScreenRecording,
                    PermissionPane::InputMonitoring,
                ]
            );
        });
        for pane in ["accessibility", "screen-recording", "input-monitoring"] {
            assert!(
                cx.debug_bounds(Box::leak(
                    format!("computer-permission-row-{pane}").into_boxed_str()
                ))
                .is_some(),
                "{pane} row painted"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn set_up_opens_the_next_missing_pane_in_order(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        init(cx, dir.path());
        let (engine, _) = scripted_engine(&runtime, |_, _| Err("offline".into()));
        let live = Rc::new(Cell::new(grants(false, false, false)));
        let read = live.clone();
        let (page, cx) = open_page(cx, engine, Rc::new(move || read.get()));
        let opened = |cx: &mut VisualTestContext| {
            page.read_with(cx, |page, _| page.guide_state().open_pane())
        };
        click(cx, "computer-set-up");
        assert_eq!(opened(cx), Some(PermissionPane::Accessibility));
        live.set(grants(true, false, false));
        click(cx, "computer-set-up");
        assert_eq!(opened(cx), Some(PermissionPane::ScreenRecording));
        live.set(grants(true, true, false));
        click(cx, "computer-set-up");
        assert_eq!(opened(cx), Some(PermissionPane::InputMonitoring));
        live.set(grants(true, true, true));
        page.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        assert_eq!(opened(cx), None, "the guide closes once the grant lands");
        assert!(cx.debug_bounds("computer-set-up").is_none());
    }

    #[gpui::test]
    fn the_toggle_round_trips_through_the_engine(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        init(cx, dir.path());
        let core = runtime
            .block_on(async {
                zeron_engine::EngineCore::assemble(
                    &dir.path().join("engine"),
                    Arc::new(zeron_engine::HarnessRegistry::new()),
                    zeron_proto::HarnessId::Mock,
                    None,
                )
            })
            .unwrap();
        let engine = crate::state::EngineHandle::from_test_client(zeron_rpc::memory_client(
            core.rpc_service(),
        ));
        let (page, cx) = open_page(cx, engine, Rc::new(|| grants(true, true, true)));
        wait(cx, |cx| {
            page.read_with(cx, |page, _| page.settings_value().is_some())
        });
        let enabled = |cx: &mut VisualTestContext| {
            page.read_with(cx, |page, _| {
                page.settings_value().map(|s| s.computer_control_enabled)
            })
        };
        assert_eq!(enabled(cx), Some(false), "Computer control defaults off");
        if cfg!(target_os = "macos") {
            click(cx, "computer-control-toggle");
            wait(cx, |_| core.computer.settings().computer_control_enabled);
            assert_eq!(enabled(cx), Some(true));
            let stored: ComputerSettings = serde_json::from_str(
                &std::fs::read_to_string(dir.path().join("engine/computer-settings.json")).unwrap(),
            )
            .unwrap();
            assert!(stored.computer_control_enabled);
            assert!(!stored.browser_developer_access);
            click(cx, "browser-developer-access");
            wait(cx, |_| core.computer.settings().browser_developer_access);
            let stored: ComputerSettings = serde_json::from_str(
                &std::fs::read_to_string(dir.path().join("engine/computer-settings.json")).unwrap(),
            )
            .unwrap();
            assert!(stored.browser_developer_access);
            click(cx, "browser-developer-access");
            wait(cx, |_| !core.computer.settings().browser_developer_access);
            click(cx, "computer-control-toggle");
            wait(cx, |_| !core.computer.settings().computer_control_enabled);
            page.update(cx, |page, cx| page.refresh(cx));
            wait(cx, |cx| enabled(cx) == Some(false));
        } else {
            click(cx, "computer-control-toggle");
            assert!(!core.computer.settings().computer_control_enabled);
        }
        drop(page);
        runtime.block_on(core.shutdown());
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn placement_chips_save_through_the_engine(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        init(cx, dir.path());
        let core = runtime
            .block_on(async {
                zeron_engine::EngineCore::assemble(
                    &dir.path().join("engine"),
                    Arc::new(zeron_engine::HarnessRegistry::new()),
                    zeron_proto::HarnessId::Mock,
                    None,
                )
            })
            .unwrap();
        let engine = crate::state::EngineHandle::from_test_client(zeron_rpc::memory_client(
            core.rpc_service(),
        ));
        let (page, cx) = open_page(cx, engine, Rc::new(|| grants(true, true, true)));
        wait(cx, |cx| {
            page.read_with(cx, |page, _| page.settings_value().is_some())
        });
        let placement = || core.computer.settings().computer_preview_placement;
        assert_eq!(placement(), ComputerPreviewPlacement::Rail);
        assert!(cx.debug_bounds("computer-placement-reset").is_none());
        click(cx, "computer-placement-card");
        wait(cx, |_| placement() == ComputerPreviewPlacement::Card);
        let stored: ComputerSettings = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("engine/computer-settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            stored.computer_preview_placement,
            ComputerPreviewPlacement::Card
        );
        click(cx, "computer-placement-reset");
        wait(cx, |_| placement() == ComputerPreviewPlacement::Rail);
        click(cx, "computer-placement-card");
        wait(cx, |_| placement() == ComputerPreviewPlacement::Card);
        click(cx, "computer-placement-rail");
        wait(cx, |_| placement() == ComputerPreviewPlacement::Rail);
        drop(page);
        runtime.block_on(core.shutdown());
    }

    #[gpui::test]
    fn always_allowed_apps_list_and_remove_through_the_engine(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        init(cx, dir.path());
        std::fs::create_dir_all(dir.path().join("engine")).unwrap();
        std::fs::write(
            dir.path().join("engine/computer-settings.json"),
            json!({
                "alwaysAllowedApps": [
                    { "bundleId": "com.apple.Safari", "name": "Safari" },
                    { "bundleId": "sh.paseo.desktop" },
                ],
            })
            .to_string(),
        )
        .unwrap();
        let core = runtime
            .block_on(async {
                zeron_engine::EngineCore::assemble(
                    &dir.path().join("engine"),
                    Arc::new(zeron_engine::HarnessRegistry::new()),
                    zeron_proto::HarnessId::Mock,
                    None,
                )
            })
            .unwrap();
        let engine = crate::state::EngineHandle::from_test_client(zeron_rpc::memory_client(
            core.rpc_service(),
        ));
        let (page, cx) = open_page(cx, engine, Rc::new(|| grants(true, true, true)));
        wait(cx, |cx| {
            page.read_with(cx, |page, _| page.settings_value().is_some())
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("computer-allowed-app-0").is_some());
        assert!(cx.debug_bounds("computer-allowed-app-1").is_some());
        assert!(cx.debug_bounds("computer-allowed-apps-empty").is_none());
        if cfg!(target_os = "macos") {
            click(cx, "computer-allowed-app-remove-com.apple.Safari");
            wait(cx, |_| {
                core.computer.settings().always_allowed_apps.len() == 1
            });
            assert_eq!(
                core.computer.settings().always_allowed_apps[0].bundle_id,
                "sh.paseo.desktop"
            );
            let stored: ComputerSettings = serde_json::from_str(
                &std::fs::read_to_string(dir.path().join("engine/computer-settings.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(stored.always_allowed_apps.len(), 1);
            click(cx, "computer-allowed-app-remove-sh.paseo.desktop");
            wait(cx, |_| {
                core.computer.settings().always_allowed_apps.is_empty()
            });
            cx.run_until_parked();
            assert!(cx.debug_bounds("computer-allowed-apps-empty").is_some());
        }
        drop(page);
        runtime.block_on(core.shutdown());
    }

    #[gpui::test]
    fn status_is_polled_only_while_the_page_is_active(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        init(cx, dir.path());
        let (engine, calls) = scripted_engine(&runtime, |method, _| {
            if method == methods::GET_COMPUTER_STATUS {
                Ok(json!({ "status": { "kind": "unsupported", "message": "no host" } }))
            } else {
                Err("offline".into())
            }
        });
        let (page, cx) = open_page(cx, engine, Rc::new(|| grants(true, true, true)));
        let status_reads = |calls: &Calls| {
            calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(method, _)| method == methods::GET_COMPUTER_STATUS)
                .count()
        };
        assert_eq!(status_reads(&calls), 1, "opening reads the status once");
        cx.executor().advance_clock(STATUS_POLL_INTERVAL * 2);
        cx.run_until_parked();
        assert_eq!(status_reads(&calls), 1, "an inactive page never polls");
        cx.update(|_, cx| page.update(cx, |page, cx| page.set_active(true, cx)));
        for expected in [2, 3] {
            cx.executor().advance_clock(STATUS_POLL_INTERVAL);
            wait(cx, |_| status_reads(&calls) == expected);
        }
        cx.update(|_, cx| page.update(cx, |page, cx| page.set_active(true, cx)));
        cx.executor().advance_clock(STATUS_POLL_INTERVAL / 2);
        cx.run_until_parked();
        assert_eq!(status_reads(&calls), 3, "re-activating keeps one timer");
        cx.update(|_, cx| page.update(cx, |page, cx| page.set_active(false, cx)));
        cx.executor().advance_clock(STATUS_POLL_INTERVAL * 3);
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(50));
        cx.run_until_parked();
        assert_eq!(status_reads(&calls), 3, "hiding the page stops the poll");
    }

    #[gpui::test]
    fn the_history_viewer_pages_on_request_and_marks_truncation(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        init(cx, dir.path());
        let (engine, calls) = scripted_engine(&runtime, |method, params| {
            if method != methods::GET_COMPUTER_AUDIT_HISTORY {
                return Err("offline".into());
            }
            Ok(match params["before"].as_str() {
                None => page_result(0..30, Some("c1"), false),
                Some("c1") => page_result(25..55, Some("c2"), true),
                Some("c2") => page_result(55..85, Some("c3"), false),
                Some(_) => page_result(85..95, Some("c4"), false),
            })
        });
        let (page, cx) = open_page(cx, engine, Rc::new(|| grants(true, true, true)));
        cx.simulate_resize(gpui::size(px(1000.0), px(12000.0)));
        let history_calls = |calls: &Calls| -> Vec<Value> {
            calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(method, _)| method == methods::GET_COMPUTER_AUDIT_HISTORY)
                .map(|(_, params)| params.clone())
                .collect()
        };
        cx.run_until_parked();
        assert!(
            history_calls(&calls).is_empty(),
            "history loads only when opened"
        );
        click(cx, "computer-history-toggle");
        wait(cx, |cx| {
            cx.debug_bounds("computer-history-row-29").is_some()
        });
        assert_eq!(history_calls(&calls), vec![json!({ "limit": 30 })]);
        assert!(cx.debug_bounds("computer-history-truncated").is_none());
        for expected in [
            json!({ "limit": 30, "before": "c1" }),
            json!({ "limit": 30, "before": "c2" }),
            json!({ "limit": 10, "before": "c3" }),
        ] {
            let before = history_calls(&calls).len();
            click(cx, "computer-history-older");
            wait(cx, |cx| {
                history_calls(&calls).len() > before
                    && page.read_with(cx, |page, _| page.history.task.is_none())
            });
            assert_eq!(history_calls(&calls).last(), Some(&expected));
        }
        cx.run_until_parked();
        assert!(cx.debug_bounds("computer-history-row-94").is_some());
        assert!(
            cx.debug_bounds("computer-history-row-95").is_none(),
            "duplicates render once"
        );
        assert!(cx.debug_bounds("computer-history-truncated").is_some());
        assert!(
            cx.debug_bounds("computer-history-older").is_none(),
            "100 loaded entries end paging"
        );
        page.read_with(cx, |page, _| {
            for entry in history_entries(&page.history.pages) {
                assert_eq!(
                    crate::computer::presentation::describe_computer_call(&entry.tool, None),
                    "Type"
                );
            }
        });
        click(cx, "computer-history-refresh");
        wait(cx, |cx| {
            page.read_with(cx, |page, _| page.history.pages.len() == 1)
        });
        assert_eq!(history_calls(&calls).last(), Some(&json!({ "limit": 30 })));
    }
}
