use super::*;
use crate::device_panel::float::{DeviceFloat, DeviceFloatEvent};
use crate::device_panel::{DevicePanel, DevicePanelEvent};
use zeron_proto::device_hub::{DeviceOpener, DevicePlatform};

type DeviceTarget = (String, String);

#[derive(Default)]
pub(super) struct DeviceShellState {
    dismissed: std::collections::HashMap<String, std::collections::HashSet<DeviceTarget>>,
    floats: std::collections::HashMap<String, (Entity<DeviceFloat>, [Subscription; 2])>,
    seen_sessions: Option<std::collections::HashSet<(String, DeviceTarget)>>,
}

impl Shell {
    pub(super) fn device_surface_unavailable(&self, cx: &App) -> Option<&'static str> {
        let state = self.state.read(cx);
        if self.active_chat.is_empty() {
            return Some("Devices are only available from a thread.");
        }
        if state
            .selected_chat_row()
            .is_some_and(|chat| Some(chat.device_id.as_str()) != state.local_device_id.as_deref())
        {
            return Some("Devices are only available on the chat's own computer.");
        }
        if state.engine().is_some_and(|engine| {
            !engine
                .engine_info()
                .supports(zeron_proto::capabilities::DEVICE_HUB_V1)
        }) {
            return Some("This Zeron engine does not support devices.");
        }
        None
    }

    fn device_tab_ids(&self, key: &str) -> Vec<u64> {
        self.right_tabs
            .get(key)
            .into_iter()
            .flatten()
            .filter_map(|surface| match surface {
                RightSurface::Device(id) if self.device_panels.contains_key(id) => Some(*id),
                _ => None,
            })
            .collect()
    }

    fn device_tab_target(&self, id: u64, cx: &App) -> Option<DeviceTarget> {
        self.device_panels.get(&id)?.0.read(cx).target().cloned()
    }

    fn idle_device_picker(&self, id: u64, cx: &App) -> bool {
        self.device_panels
            .get(&id)
            .is_some_and(|(panel, _)| panel.read(cx).is_idle_picker())
    }

    fn new_device_panel(
        &mut self,
        chat_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> u64 {
        self.device_seq += 1;
        let id = self.device_seq;
        let state = self.state.clone();
        let panel = cx.new(|cx| DevicePanel::new(state, chat_id, cx));
        let observer = cx.observe(&panel, |_, _, cx| cx.notify());
        let events = cx.subscribe_in(
            &panel,
            window,
            move |this, _, event, window, cx| match event {
                DevicePanelEvent::Close => {
                    this.close_right_surface(RightSurface::Device(id), window, cx)
                }
                DevicePanelEvent::Bound => this.device_tab_bound(id, cx),
                DevicePanelEvent::Float => this.float_device_tab(id, cx),
            },
        );
        self.device_panels.insert(id, (panel, [observer, events]));
        id
    }

    fn show_device_tab(&mut self, chat_id: &str, id: u64, cx: &mut Context<Self>) {
        if chat_id == self.panel_key(cx) {
            self.set_right_active(RightSurface::Device(id), cx);
        } else {
            self.panels.update(chat_id, |panels| {
                panels.right_active = RightSurface::Device(id)
            });
        }
    }

    fn remove_device_tab(&mut self, chat_id: &str, id: u64) {
        if let Some(tabs) = self.right_tabs.get_mut(chat_id) {
            tabs.retain(|surface| *surface != RightSurface::Device(id));
        }
        self.panels
            .forget_right_surface(chat_id, RightSurface::Device(id));
        self.device_panels.remove(&id);
    }

    pub(super) fn add_device_surface(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        if self.device_surface_unavailable(cx).is_some() {
            return None;
        }
        let key = self.panel_key(cx);
        let placeholder = self
            .device_tab_ids(&key)
            .into_iter()
            .find(|id| self.device_tab_target(*id, cx).is_none());
        let id = match placeholder {
            Some(id) => id,
            None => {
                let id = self.new_device_panel(self.active_chat.clone(), window, cx);
                self.right_tabs
                    .entry(key.clone())
                    .or_default()
                    .push(RightSurface::Device(id));
                id
            }
        };
        self.show_device_tab(&key, id, cx);
        Some(id)
    }

    pub(super) fn open_device_tab(
        &mut self,
        chat_id: &str,
        target: DeviceTarget,
        automatic: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        let dismissed = self
            .device_shell
            .dismissed
            .entry(chat_id.to_string())
            .or_default();
        if automatic && dismissed.contains(&target) {
            return None;
        }
        dismissed.remove(&target);
        let ids = self.device_tab_ids(chat_id);
        let existing = ids
            .iter()
            .copied()
            .find(|id| self.device_tab_target(*id, cx).as_ref() == Some(&target));
        let placeholder = ids
            .iter()
            .copied()
            .find(|id| self.idle_device_picker(*id, cx));
        let id = match (existing, placeholder) {
            (Some(_), _) if automatic => return None,
            (Some(existing), placeholder) => {
                if let Some(placeholder) = placeholder {
                    self.remove_device_tab(chat_id, placeholder);
                }
                existing
            }
            (None, Some(placeholder)) => placeholder,
            (None, None) => {
                let id = self.new_device_panel(chat_id.to_string(), window, cx);
                self.right_tabs
                    .entry(chat_id.to_string())
                    .or_default()
                    .push(RightSurface::Device(id));
                id
            }
        };
        if existing.is_none()
            && let Some((panel, _)) = self.device_panels.get(&id)
        {
            panel.update(cx, |panel, cx| panel.bind(target.0, target.1, cx));
        }
        self.show_device_tab(chat_id, id, cx);
        Some(id)
    }

    fn device_tab_bound(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some((panel, _)) = self.device_panels.get(&id) else {
            return;
        };
        let (chat_id, target) = {
            let panel = panel.read(cx);
            (panel.chat_id().to_string(), panel.target().cloned())
        };
        let Some(target) = target else {
            return;
        };
        if let Some(dismissed) = self.device_shell.dismissed.get_mut(&chat_id) {
            dismissed.remove(&target);
        }
        let shown = self.device_tab_ids(&chat_id).into_iter().find(|other| {
            *other != id && self.device_tab_target(*other, cx).as_ref() == Some(&target)
        });
        if let Some(shown) = shown {
            self.remove_device_tab(&chat_id, id);
            self.show_device_tab(&chat_id, shown, cx);
        }
        cx.notify();
    }

    pub(super) fn drop_device_tab(&mut self, id: u64, cx: &App) {
        let Some((panel, _)) = self.device_panels.remove(&id) else {
            return;
        };
        let panel = panel.read(cx);
        if let Some(target) = panel.target() {
            self.device_shell
                .dismissed
                .entry(panel.chat_id().to_string())
                .or_default()
                .insert(target.clone());
        }
    }

    pub(super) fn device_tab_icon(&self, id: u64, cx: &App) -> &'static str {
        match self
            .device_panels
            .get(&id)
            .and_then(|(panel, _)| panel.read(cx).platform(cx))
        {
            Some(DevicePlatform::Ios) => icons::APPLE,
            Some(DevicePlatform::Android) => icons::ANDROID,
            None => icons::SMARTPHONE,
        }
    }

    pub(super) fn device_tab_detail(&self, id: u64, cx: &App) -> Option<SharedString> {
        let panel = self.device_panels.get(&id)?.0.read(cx);
        let description = panel.description(cx)?;
        Some(format!("{}\n{description}", panel.title(cx)).into())
    }

    pub(super) fn device_tab_label(
        &self,
        surface: RightSurface,
        title: SharedString,
        cx: &App,
    ) -> AnyElement {
        let input = match surface {
            RightSurface::Device(id) => self
                .device_panels
                .get(&id)
                .and_then(|(panel, _)| panel.read(cx).rename_input()),
            _ => None,
        };
        match input {
            Some(input) => div()
                .id("device-tab-rename")
                .debug_selector(|| "device-tab-rename".into())
                .w_full()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(input)
                .into_any_element(),
            None => title.into_any_element(),
        }
    }

    pub(super) fn prune_device_shell(&mut self, cx: &App) {
        let state = self.state.read(cx);
        if !state.chats_synced {
            return;
        }
        let live = state
            .chats
            .iter()
            .map(|chat| chat.id.as_str())
            .collect::<std::collections::HashSet<_>>();
        self.device_shell
            .floats
            .retain(|chat_id, _| live.contains(chat_id.as_str()));
        self.device_shell
            .dismissed
            .retain(|chat_id, _| live.contains(chat_id.as_str()));
    }

    pub(super) fn rename_device_tab(
        &mut self,
        surface: RightSurface,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let RightSurface::Device(id) = surface
            && let Some((panel, _)) = self.device_panels.get(&id)
        {
            panel.update(cx, |panel, cx| panel.begin_rename(window, cx));
        }
    }

    pub(super) fn cancel_device_rename(&mut self, cx: &mut Context<Self>) -> bool {
        let mut cancelled = false;
        for (panel, _) in self.device_panels.values() {
            cancelled |= panel.update(cx, |panel, cx| panel.finish_rename(false, cx));
        }
        cancelled
    }

    pub(super) fn device_rename_menu_row(
        &self,
        surface: RightSurface,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let RightSurface::Device(id) = surface else {
            return None;
        };
        self.device_tab_target(id, cx)?;
        Some(
            popover::menu_row(theme, false, "device-tab-menu-rename")
                .id("device-tab-menu-rename")
                .debug_selector(|| "device-tab-menu-rename".into())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.close_chat_menu(cx);
                    this.rename_device_tab(surface, window, cx);
                }))
                .child("Rename")
                .into_any_element(),
        )
    }

    fn float_device_tab(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some((panel, _)) = self.device_panels.get(&id) else {
            return;
        };
        let (chat_id, target) = {
            let panel = panel.read(cx);
            (panel.chat_id().to_string(), panel.target().cloned())
        };
        let Some(target) = target else {
            return;
        };
        self.open_device_float(chat_id.clone(), target, cx);
        if chat_id == self.panel_key(cx) {
            self.set_surfaces_open(false, cx);
        }
    }

    pub(super) fn float_active_device_tab(&mut self, cx: &mut Context<Self>) {
        if let RightSurface::Device(id) = self.resolved_right_active(cx)
            && let Some(target) = self.device_tab_target(id, cx)
        {
            self.open_device_float(self.active_chat.clone(), target, cx);
        }
    }

    pub(super) fn open_device_float(
        &mut self,
        chat_id: String,
        target: DeviceTarget,
        cx: &mut Context<Self>,
    ) {
        if self
            .device_shell
            .floats
            .get(&chat_id)
            .is_some_and(|(float, _)| float.read(cx).target() == &target)
        {
            return;
        }
        let shell = cx.weak_entity();
        let (open_chat, open_target) = (chat_id.clone(), target.clone());
        let state = self.state.clone();
        let float_chat = chat_id.clone();
        let float = cx.new(|cx| {
            DeviceFloat::new(
                state,
                float_chat,
                target,
                move |window, cx| {
                    shell
                        .update(cx, |shell, cx| {
                            shell.open_float_in_panel(&open_chat, open_target.clone(), window, cx)
                        })
                        .ok();
                },
                cx,
            )
        });
        let observer = cx.observe(&float, |_, _, cx| cx.notify());
        let close_chat = chat_id.clone();
        let events = cx.subscribe(&float, move |this, float, event, cx| match event {
            DeviceFloatEvent::Close => this.close_device_float(&close_chat, &float, cx),
        });
        self.device_shell
            .floats
            .insert(chat_id, (float, [observer, events]));
        cx.notify();
    }

    fn close_device_float(
        &mut self,
        chat_id: &str,
        float: &Entity<DeviceFloat>,
        cx: &mut Context<Self>,
    ) {
        if self
            .device_shell
            .floats
            .get(chat_id)
            .is_some_and(|(current, _)| current == float)
        {
            self.device_shell.floats.remove(chat_id);
            cx.notify();
        }
    }

    fn open_float_in_panel(
        &mut self,
        chat_id: &str,
        target: DeviceTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.device_shell.floats.remove(chat_id);
        self.open_device_tab(chat_id, target, false, window, cx);
        if chat_id == self.panel_key(cx) {
            self.set_surfaces_open(true, cx);
        }
        cx.notify();
    }

    pub(super) fn auto_open_devices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hub) = self.state.read(cx).device_hub.as_ref() else {
            self.device_shell.seen_sessions = None;
            return;
        };
        let current: std::collections::HashSet<(String, DeviceTarget)> = hub
            .sessions
            .iter()
            .filter(|session| hub.device(&session.host_id, &session.device_id).is_some())
            .map(|session| {
                (
                    session.chat_id.clone(),
                    (session.host_id.clone(), session.device_id.clone()),
                )
            })
            .collect();
        if self.device_shell.seen_sessions.as_ref() == Some(&current) {
            return;
        }
        let Some(previous) = self.device_shell.seen_sessions.replace(current.clone()) else {
            return;
        };
        let mut opened: Vec<_> = current.difference(&previous).cloned().collect();
        opened.sort();
        for (chat_id, target) in opened {
            let shown = self.device_tab_ids(&chat_id).into_iter().any(|id| {
                self.device_panels
                    .get(&id)
                    .is_some_and(|(panel, _)| panel.read(cx).shows(&target))
            });
            if shown {
                continue;
            }
            if self.settings.device_auto_float {
                self.open_device_float(chat_id, target, cx);
            } else {
                self.open_device_tab(&chat_id, target, true, window, cx);
            }
        }
    }

    pub(super) fn sync_device_visibility(
        &mut self,
        chat_visible: bool,
        pane_visible: bool,
        selected: RightSurface,
        cx: &mut Context<Self>,
    ) {
        let shown = match selected {
            RightSurface::Device(id) if pane_visible => Some(id),
            _ => None,
        };
        let shown_target = shown.and_then(|id| self.device_tab_target(id, cx));
        for (id, (panel, _)) in &self.device_panels {
            let visible = shown == Some(*id);
            panel.update(cx, |panel, cx| panel.set_visible(visible, cx));
        }
        for (chat_id, (float, _)) in &self.device_shell.floats {
            let visible = chat_visible
                && *chat_id == self.active_chat
                && shown_target.as_ref() != Some(float.read(cx).target());
            float.update(cx, |float, cx| float.set_visible(visible, cx));
        }
    }

    pub(super) fn sidecar_devices(
        &self,
        chat_id: &str,
        cx: &App,
    ) -> Vec<(DeviceTarget, String, DevicePlatform, Option<DeviceOpener>)> {
        let Some(hub) = self.state.read(cx).device_hub.as_ref() else {
            return Vec::new();
        };
        hub.sessions
            .iter()
            .filter(|session| session.chat_id == chat_id)
            .filter_map(|session| {
                let device = hub.device(&session.host_id, &session.device_id)?;
                Some((
                    (session.host_id.clone(), session.device_id.clone()),
                    device.name.clone(),
                    device.platform,
                    session.opened_by,
                ))
            })
            .collect()
    }

    pub(super) fn focus_device(
        &mut self,
        chat_id: &str,
        target: DeviceTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let shown = self
            .device_tab_ids(chat_id)
            .into_iter()
            .any(|id| self.device_tab_target(id, cx).as_ref() == Some(&target));
        let floating = self
            .device_shell
            .floats
            .get(chat_id)
            .map(|(float, _)| float.clone())
            .filter(|float| float.read(cx).target() == &target);
        if let Some(float) = floating.filter(|_| !shown) {
            float.update(cx, |float, cx| float.focus(window, cx));
            return;
        }
        if !self.right_pane_open(cx) {
            self.toggle_right_pane(cx);
        }
        self.open_device_tab(chat_id, target, false, window, cx);
    }

    pub(super) fn render_device_float(&self, cx: &App) -> Option<AnyElement> {
        let (float, _) = self.device_shell.floats.get(&self.active_chat)?;
        float
            .read(cx)
            .is_visible()
            .then(|| float.clone().into_any_element())
    }

    pub(super) fn device_tab_title(&self, id: u64, cx: &App) -> Option<SharedString> {
        self.device_panels
            .get(&id)
            .map(|(panel, _)| panel.read(cx).title(cx))
    }

    pub(super) fn render_device_surface(&self, id: u64) -> AnyElement {
        match self.device_panels.get(&id) {
            Some((panel, _)) => panel.clone().into_any_element(),
            None => Empty.into_any_element(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell_window(
        cx: &mut gpui::TestAppContext,
    ) -> (gpui::WindowHandle<Shell>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            crate::app_menus::init(cx);
        });
        let data_dir = dir.path().to_path_buf();
        let window = cx.add_window(|_, cx| {
            let state = cx.new(|_| AppState::new());
            Shell::new(
                state,
                EngineBootConfig {
                    data_dir,
                    ipc_port: 0,
                    edge_url: "http://127.0.0.1:1".into(),
                    edge_token: None,
                    org_id: None,
                    workos_client_id: None,
                    default_harness: zeron_proto::HarnessId::Mock,
                },
                cx,
            )
        });
        (window, dir)
    }

    #[gpui::test]
    fn a_device_tab_opens_closes_and_falls_back_to_the_previous_tab(cx: &mut gpui::TestAppContext) {
        let (window, _dir) = shell_window(cx);
        window
            .update(cx, |shell, window, cx| {
                shell.active_chat = "owner".into();
                shell.add_browser_surface(None, window, cx);
                let browser = RightSurface::Browser(shell.browser_seq);
                let id = shell
                    .add_device_surface(window, cx)
                    .expect("device tab opens");
                let device = RightSurface::Device(id);
                assert_eq!(shell.resolved_right_active(cx), device);
                let rows = shell.right_surface_rows(cx);
                assert_eq!(
                    rows.iter()
                        .map(|(surface, ..)| *surface)
                        .collect::<Vec<_>>(),
                    vec![browser, device]
                );
                assert_eq!(rows[1].1.as_ref(), "Device");
                shell.close_right_surface(device, window, cx);
                assert!(shell.device_panels.is_empty(), "closing drops the panel");
                assert_eq!(shell.resolved_right_active(cx), browser);
                assert!(
                    !shell
                        .right_surface_rows(cx)
                        .iter()
                        .any(|(surface, ..)| *surface == device)
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn the_device_row_is_unavailable_on_the_new_chat_canvas(cx: &mut gpui::TestAppContext) {
        let (window, _dir) = shell_window(cx);
        window
            .update(cx, |shell, window, cx| {
                shell.active_chat.clear();
                assert_eq!(
                    shell.device_surface_unavailable(cx),
                    Some("Devices are only available from a thread.")
                );
                assert_eq!(shell.add_device_surface(window, cx), None);
                assert!(shell.device_panels.is_empty());
                assert!(
                    shell
                        .right_tabs
                        .get(&shell.panel_key(cx))
                        .is_none_or(|tabs| tabs.is_empty())
                );
            })
            .unwrap();
    }

    use crate::device_panel::tests::{Calls, fake_engine, open_frame_streams};
    use zeron_proto::device_hub::{DeviceHubState, DeviceSession, HubDevice};

    fn two_devices(revision: u64) -> DeviceHubState {
        let mut hub = crate::device_panel::tests::hub(true);
        hub.revision = revision;
        hub.devices[0].version = "iOS 27.0".into();
        hub.devices.push(HubDevice {
            host_id: "local".into(),
            id: "PAD".into(),
            name: "iPad Air".into(),
            version: "iOS 27.0".into(),
            ..Default::default()
        });
        for device_id in ["SIM", "PAD"] {
            hub.sessions.push(DeviceSession {
                chat_id: "chat".into(),
                host_id: "local".into(),
                device_id: device_id.into(),
                ..Default::default()
            });
        }
        hub
    }

    fn target(device_id: &str) -> DeviceTarget {
        ("local".into(), device_id.into())
    }

    fn device_shell(
        cx: &mut gpui::TestAppContext,
    ) -> (gpui::WindowHandle<Shell>, tempfile::TempDir) {
        let (window, dir) = shell_window(cx);
        cx.executor().allow_parking();
        window
            .update(cx, |shell, _, cx| {
                shell.state.update(cx, |state, cx| {
                    state.selected_chat = Some("chat".into());
                    state.device_hub = Some(two_devices(1));
                    cx.notify();
                });
            })
            .unwrap();
        cx.run_until_parked();
        (window, dir)
    }

    fn device_tabs(shell: &Shell) -> Vec<u64> {
        shell.device_tab_ids("chat")
    }

    fn settle(
        cx: &mut gpui::TestAppContext,
        mut done: impl FnMut(&mut gpui::TestAppContext) -> bool,
    ) {
        for _ in 0..400 {
            cx.run_until_parked();
            if done(cx) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("condition never held");
    }

    #[gpui::test]
    fn a_placeholder_is_retargeted_in_place_and_a_shown_device_is_focused_not_duplicated(
        cx: &mut gpui::TestAppContext,
    ) {
        let (window, _dir) = device_shell(cx);
        let (picker, second) = window
            .update(cx, |shell, window, cx| {
                let picker = shell.add_device_surface(window, cx).unwrap();
                assert_eq!(
                    shell.add_device_surface(window, cx),
                    Some(picker),
                    "+ -> Device focuses the open picker instead of adding another"
                );
                assert_eq!(
                    shell.open_device_tab("chat", target("SIM"), false, window, cx),
                    Some(picker),
                    "choosing a device retargets the placeholder in place"
                );
                assert_eq!(device_tabs(shell), vec![picker]);
                let rows = shell.right_surface_rows(cx);
                assert_eq!(rows[0].1.as_ref(), "iPhone 17");
                assert_eq!(
                    rows[0].3.as_ref().map(|detail| detail.to_string()),
                    Some("iPhone 17\nThis machine · iOS 27.0".to_string())
                );
                assert_eq!(shell.device_tab_icon(picker, cx), icons::APPLE);

                let second = shell.add_device_surface(window, cx).unwrap();
                assert_ne!(
                    second, picker,
                    "a bound tab gets a second picker tab beside it"
                );
                assert_eq!(device_tabs(shell), vec![picker, second]);
                assert_eq!(shell.device_tab_icon(second, cx), icons::SMARTPHONE);
                assert_eq!(
                    shell.resolved_right_active(cx),
                    RightSurface::Device(second)
                );
                let panel = shell.device_panels[&second].0.clone();
                panel.update(cx, |panel, cx| {
                    panel.bind("local".into(), "SIM".into(), cx);
                    cx.emit(DevicePanelEvent::Bound);
                });
                (picker, second)
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, window, cx| {
                assert_eq!(
                    device_tabs(shell),
                    vec![picker],
                    "opening a device shown elsewhere drops the picker"
                );
                assert!(!shell.device_panels.contains_key(&second));
                assert_eq!(
                    shell.resolved_right_active(cx),
                    RightSurface::Device(picker)
                );
                assert!(
                    shell
                        .device_shell
                        .dismissed
                        .values()
                        .all(|ids| ids.is_empty()),
                    "the dropped picker was never bound, so nothing is dismissed"
                );
                let pad = shell.add_device_surface(window, cx).unwrap();
                assert_eq!(
                    shell.open_device_tab("chat", target("SIM"), false, window, cx),
                    Some(picker),
                    "a manual open of a shown device focuses its tab"
                );
                assert_eq!(
                    device_tabs(shell),
                    vec![picker],
                    "and removes the open picker"
                );
                assert!(!shell.device_panels.contains_key(&pad));
            })
            .unwrap();
    }

    #[gpui::test]
    fn agent_opened_devices_float_or_open_one_tab_after_the_baseline_skipping_dismissed_ones(
        cx: &mut gpui::TestAppContext,
    ) {
        let (window, _dir) = device_shell(cx);
        let publish = |shell: &mut Shell,
                       cx: &mut Context<Shell>,
                       revision,
                       devices: &[&str],
                       sessions: &[(&str, &str)]| {
            let mut hub = two_devices(revision);
            for id in devices {
                hub.devices.push(HubDevice {
                    host_id: "local".into(),
                    id: (*id).into(),
                    name: (*id).into(),
                    ..Default::default()
                });
            }
            for (chat_id, device_id) in sessions {
                hub.sessions.push(DeviceSession {
                    chat_id: (*chat_id).into(),
                    host_id: "local".into(),
                    device_id: (*device_id).into(),
                    ..Default::default()
                });
            }
            shell
                .state
                .update(cx, |state, _| state.device_hub = Some(hub));
        };
        let floats = |shell: &Shell, cx: &App| {
            let mut floats: Vec<_> = shell
                .device_shell
                .floats
                .iter()
                .map(|(chat, (float, _))| (chat.clone(), float.read(cx).target().1.clone()))
                .collect();
            floats.sort();
            floats
        };
        window
            .update(cx, |shell, window, cx| {
                shell.settings.device_auto_float = false;
                shell.auto_open_devices(window, cx);
                assert!(
                    device_tabs(shell).is_empty() && shell.device_shell.floats.is_empty(),
                    "sessions in the first snapshot are a baseline"
                );
                let mini = shell
                    .open_device_tab("chat", target("MINI"), false, window, cx)
                    .unwrap();
                shell.close_right_surface(RightSurface::Device(mini), window, cx);

                publish(
                    shell,
                    cx,
                    2,
                    &["MINI", "X"],
                    &[
                        ("chat", "MINI"),
                        ("chat", "X"),
                        ("other", "PAD"),
                        ("chat", "GHOST"),
                    ],
                );
                shell.auto_open_devices(window, cx);
                let targets = |shell: &Shell, chat: &str, cx: &App| {
                    shell
                        .device_tab_ids(chat)
                        .into_iter()
                        .filter_map(|id| shell.device_tab_target(id, cx))
                        .map(|(_, device)| device)
                        .collect::<Vec<_>>()
                };
                assert_eq!(
                    targets(shell, "chat", cx),
                    ["X"],
                    "a dismissed device stays closed"
                );
                assert_eq!(
                    targets(shell, "other", cx),
                    ["PAD"],
                    "a chat that is not shown gets its tab too"
                );
                shell.auto_open_devices(window, cx);
                assert_eq!(device_tabs(shell).len(), 1, "one tab per new session");

                publish(
                    shell,
                    cx,
                    3,
                    &["MINI", "X", "GHOST"],
                    &[
                        ("chat", "MINI"),
                        ("chat", "X"),
                        ("other", "PAD"),
                        ("chat", "GHOST"),
                    ],
                );
                shell.auto_open_devices(window, cx);
                assert_eq!(
                    targets(shell, "chat", cx),
                    ["X", "GHOST"],
                    "a session waits for its device summary"
                );

                shell.settings.device_auto_float = true;
                shell.open_device_tab("chat", target("Z"), false, window, cx);
                publish(
                    shell,
                    cx,
                    4,
                    &["MINI", "X", "GHOST", "Y", "Z"],
                    &[
                        ("chat", "MINI"),
                        ("chat", "X"),
                        ("other", "PAD"),
                        ("chat", "GHOST"),
                        ("chat", "Y"),
                        ("chat", "Z"),
                    ],
                );
                shell.auto_open_devices(window, cx);
                assert_eq!(
                    floats(shell, cx),
                    [("chat".to_string(), "Y".to_string())],
                    "auto-float floats the new device; one already in a tab is left alone"
                );
                assert_eq!(targets(shell, "chat", cx), ["X", "GHOST", "Z"]);

                let all = [
                    ("chat", "MINI"),
                    ("chat", "X"),
                    ("other", "PAD"),
                    ("chat", "GHOST"),
                    ("chat", "Y"),
                    ("chat", "Z"),
                ];
                shell.settings.device_auto_float = false;
                publish(shell, cx, 9, &["MINI", "X", "GHOST", "Y", "Z"], &all);
                shell.auto_open_devices(window, cx);
                assert_eq!(
                    targets(shell, "chat", cx),
                    ["X", "GHOST", "Z"],
                    "a new revision with the same sessions opens nothing"
                );

                shell.state.update(cx, |state, _| state.device_hub = None);
                shell.auto_open_devices(window, cx);
                let mut restarted = all.to_vec();
                restarted.push(("chat", "W"));
                publish(
                    shell,
                    cx,
                    1,
                    &["MINI", "X", "GHOST", "Y", "Z", "W"],
                    &restarted,
                );
                shell.auto_open_devices(window, cx);
                assert_eq!(
                    targets(shell, "chat", cx),
                    ["X", "GHOST", "Z"],
                    "the first snapshot of a new engine is a fresh baseline"
                );
                restarted.push(("chat", "V"));
                publish(
                    shell,
                    cx,
                    1,
                    &["MINI", "X", "GHOST", "Y", "Z", "W", "V"],
                    &restarted,
                );
                shell.auto_open_devices(window, cx);
                assert_eq!(
                    targets(shell, "chat", cx),
                    ["X", "GHOST", "Z", "V"],
                    "a reused revision number still opens the new session"
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn closing_a_bound_tab_is_remembered_for_automatic_opens_until_a_manual_open(
        cx: &mut gpui::TestAppContext,
    ) {
        let (window, _dir) = device_shell(cx);
        window
            .update(cx, |shell, window, cx| {
                let id = shell
                    .open_device_tab("chat", target("SIM"), false, window, cx)
                    .unwrap();
                shell.close_right_surface(RightSurface::Device(id), window, cx);
                assert!(device_tabs(shell).is_empty());
                assert_eq!(
                    shell.open_device_tab("chat", target("SIM"), true, window, cx),
                    None,
                    "an automatic open skips a dismissed device"
                );
                assert!(device_tabs(shell).is_empty());
                assert!(
                    shell
                        .open_device_tab("chat", target("PAD"), true, window, cx)
                        .is_some(),
                    "other devices still open automatically"
                );
                let again = shell
                    .open_device_tab("chat", target("SIM"), false, window, cx)
                    .unwrap();
                assert_eq!(
                    shell.open_device_tab("chat", target("SIM"), true, window, cx),
                    None,
                    "an automatic open never duplicates a shown device"
                );
                assert_eq!(device_tabs(shell).len(), 2);
                shell.close_right_surface(RightSurface::Device(again), window, cx);
                let picker = shell.add_device_surface(window, cx).unwrap();
                shell.device_panels[&picker].0.update(cx, |panel, cx| {
                    panel.bind("local".into(), "SIM".into(), cx);
                    cx.emit(DevicePanelEvent::Bound);
                });
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, window, cx| {
                assert!(
                    !shell.device_shell.dismissed["chat"].contains(&target("SIM")),
                    "opening from the picker clears the dismissal"
                );
                assert_eq!(
                    shell.open_device_tab("chat", target("SIM"), true, window, cx),
                    None
                );
                assert_eq!(device_tabs(shell).len(), 2);
            })
            .unwrap();
    }

    fn attach_engine(
        window: gpui::WindowHandle<Shell>,
        runtime: &tokio::runtime::Runtime,
        cx: &mut gpui::TestAppContext,
    ) -> Calls {
        let (engine, calls) = fake_engine(runtime);
        window
            .update(cx, |shell, _, cx| {
                shell.state.update(cx, |state, cx| {
                    state.set_test_engine(engine);
                    state.device_hub = Some(two_devices(2));
                    cx.notify();
                });
            })
            .unwrap();
        calls
    }

    fn set_pane(
        window: gpui::WindowHandle<Shell>,
        cx: &mut gpui::TestAppContext,
        open: bool,
        selected: Option<u64>,
    ) {
        window
            .update(cx, |shell, _, cx| {
                shell.panels.update("chat", |panels| {
                    panels.changes_open = open;
                    if let Some(id) = selected {
                        panels.right_active = RightSurface::Device(id);
                    }
                });
                cx.notify();
            })
            .unwrap();
    }

    #[gpui::test]
    fn only_the_shown_tab_or_float_keeps_a_frame_subscription(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (window, _dir) = device_shell(cx);
        // A flag flip mid open-tween becomes a fit exit whose last frame a test window never draws.
        cx.update(|cx| crate::motion::set_reduced_motion(cx, true));
        let (sim, pad) = window
            .update(cx, |shell, window, cx| {
                shell.debug_gate = Some(GatePhase::Ready);
                let sim = shell.open_device_tab("chat", target("SIM"), false, window, cx);
                let pad = shell.open_device_tab("chat", target("PAD"), false, window, cx);
                (sim.unwrap(), pad.unwrap())
            })
            .unwrap();
        set_pane(window, cx, true, Some(pad));
        let calls = attach_engine(window, &runtime, cx);
        let watches = |calls: &Calls| {
            crate::device_panel::tests::calls_of(calls, zeron_rpc::methods::WATCH_HUB_DEVICE_FRAMES)
                .into_iter()
                .map(|params| params["deviceId"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        };
        settle(cx, |_| {
            watches(&calls) == ["PAD"] && open_frame_streams(&calls) == 1
        });

        set_pane(window, cx, true, Some(sim));
        settle(cx, |_| {
            watches(&calls) == ["PAD", "SIM"] && open_frame_streams(&calls) == 1
        });

        set_pane(window, cx, false, None);
        settle(cx, |_| open_frame_streams(&calls) == 0);

        window
            .update(cx, |shell, _, cx| {
                shell.open_device_float("chat".into(), target("SIM"), cx)
            })
            .unwrap();
        settle(cx, |_| {
            watches(&calls).len() == 3 && open_frame_streams(&calls) == 1
        });
        assert!(
            window
                .update(cx, |shell, _, cx| shell.device_shell.floats["chat"]
                    .0
                    .read(cx)
                    .is_visible())
                .unwrap(),
            "the float counts as visible"
        );

        set_pane(window, cx, true, Some(sim));
        settle(cx, |_| {
            watches(&calls).len() == 4 && open_frame_streams(&calls) == 1
        });
        assert!(
            window
                .update(cx, |shell, _, cx| !shell.device_shell.floats["chat"]
                    .0
                    .read(cx)
                    .is_visible())
                .unwrap(),
            "the float hides while the pane shows the same device"
        );

        window
            .update(cx, |shell, _, cx| {
                shell.state.update(cx, |state, cx| {
                    state.selected_chat = Some("other".into());
                    cx.notify();
                })
            })
            .unwrap();
        settle(cx, |_| open_frame_streams(&calls) == 0);
        window
            .update(cx, |shell, _, cx| {
                shell.state.update(cx, |state, cx| {
                    state.selected_chat = Some("chat".into());
                    cx.notify();
                })
            })
            .unwrap();
        settle(cx, |_| {
            watches(&calls).len() == 5 && open_frame_streams(&calls) == 1
        });
        assert_eq!(watches(&calls).last().map(String::as_str), Some("SIM"));
        assert!(
            crate::device_panel::tests::calls_of(&calls, zeron_rpc::methods::CLOSE_HUB_DEVICE)
                .is_empty(),
            "hiding never ends a device session"
        );

        set_pane(window, cx, true, Some(pad));
        settle(cx, |_| open_frame_streams(&calls) == 2);
        window
            .update(cx, |shell, _, cx| {
                shell.right_pane_expanded = true;
                cx.notify();
            })
            .unwrap();
        settle(cx, |_| open_frame_streams(&calls) == 1);
        assert!(
            window
                .update(cx, |shell, _, cx| !shell.device_shell.floats["chat"]
                    .0
                    .read(cx)
                    .is_visible())
                .unwrap(),
            "pane takeover leaves no chat column for the float"
        );
    }

    #[gpui::test]
    fn closing_the_pane_or_the_more_menu_floats_the_device_and_the_float_follows_its_session(
        cx: &mut gpui::TestAppContext,
    ) {
        let (window, _dir) = device_shell(cx);
        let id = window
            .update(cx, |shell, window, cx| {
                let id = shell
                    .open_device_tab("chat", target("SIM"), false, window, cx)
                    .unwrap();
                shell.set_surfaces_open(true, cx);
                assert!(shell.right_pane_open(cx));
                shell.toggle_right_pane(cx);
                assert!(!shell.right_pane_open(cx));
                id
            })
            .unwrap();
        cx.run_until_parked();
        let float_target = |window: gpui::WindowHandle<Shell>, cx: &mut gpui::TestAppContext| {
            window
                .update(cx, |shell, _, cx| {
                    shell
                        .device_shell
                        .floats
                        .get("chat")
                        .map(|(float, _)| float.read(cx).target().clone())
                })
                .unwrap()
        };
        assert_eq!(float_target(window, cx), Some(target("SIM")));

        window
            .update(cx, |shell, window, cx| {
                shell.device_shell.floats.clear();
                shell.set_surfaces_open(true, cx);
                shell.open_device_tab("chat", target("PAD"), false, window, cx);
                shell.toggle_right_pane(cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            float_target(window, cx),
            Some(target("PAD")),
            "the active tab floats, replacing the chat's float"
        );

        window
            .update(cx, |shell, _, cx| {
                shell.set_surfaces_open(true, cx);
                shell.device_panels[&id]
                    .0
                    .update(cx, |_, cx| cx.emit(DevicePanelEvent::Float));
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(float_target(window, cx), Some(target("SIM")));
        window
            .update(cx, |shell, _, cx| {
                assert!(!shell.right_pane_open(cx), "floating closes the pane");
                assert_eq!(
                    device_tabs(shell).len(),
                    2,
                    "the tab stays in the strip while it floats"
                );
                shell.state.update(cx, |state, cx| {
                    let mut hub = two_devices(3);
                    hub.sessions.retain(|session| session.device_id != "SIM");
                    state.device_hub = Some(hub);
                    cx.notify();
                });
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            float_target(window, cx),
            None,
            "powering the device off elsewhere closes its float"
        );
        window
            .update(cx, |shell, _, cx| {
                assert_eq!(
                    shell.device_tab_target(id, cx),
                    None,
                    "and returns its tab to the picker"
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn a_picker_with_an_open_in_flight_is_neither_retargeted_nor_dropped(
        cx: &mut gpui::TestAppContext,
    ) {
        let (window, _dir) = device_shell(cx);
        window
            .update(cx, |shell, window, cx| {
                let picker = shell.add_device_surface(window, cx).unwrap();
                let device = shell.state.read(cx).device_hub.as_ref().unwrap().devices[0].clone();
                shell.device_panels[&picker]
                    .0
                    .update(cx, |panel, cx| panel.open_device(device, cx));
                let pad = shell
                    .open_device_tab("chat", target("PAD"), false, window, cx)
                    .unwrap();
                assert_ne!(pad, picker, "the opening picker keeps its device");
                assert_eq!(shell.device_tab_target(picker, cx), None);
                assert_eq!(
                    shell.open_device_tab("chat", target("PAD"), false, window, cx),
                    Some(pad)
                );
                assert_eq!(
                    device_tabs(shell),
                    vec![picker, pad],
                    "focusing a shown device leaves the opening picker alone"
                );
                assert!(
                    shell
                        .device_rename_menu_row(RightSurface::Device(picker), &Theme::default(), cx)
                        .is_none(),
                    "a picker tab has no Rename row"
                );
                assert!(
                    shell
                        .device_rename_menu_row(RightSurface::Device(pad), &Theme::default(), cx)
                        .is_some()
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn floats_and_dismissals_of_deleted_chats_are_pruned(cx: &mut gpui::TestAppContext) {
        let (window, _dir) = device_shell(cx);
        window
            .update(cx, |shell, _, cx| {
                for chat in ["chat", "gone"] {
                    shell.open_device_float(chat.into(), target("SIM"), cx);
                    shell
                        .device_shell
                        .dismissed
                        .entry(chat.into())
                        .or_default()
                        .insert(target("PAD"));
                }
                shell.state.update(cx, |state, cx| {
                    state.chats = vec![
                        serde_json::from_value(serde_json::json!({
                            "id": "chat",
                            "deviceId": "this-mac",
                            "archived": false,
                            "createdAt": "2026-09-28T00:00:00Z"
                        }))
                        .unwrap(),
                    ];
                    state.chats_synced = true;
                    cx.notify();
                });
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, _| {
                let mut floats: Vec<_> = shell.device_shell.floats.keys().cloned().collect();
                floats.sort();
                assert_eq!(floats, ["chat"]);
                let mut dismissed: Vec<_> = shell.device_shell.dismissed.keys().cloned().collect();
                dismissed.sort();
                assert_eq!(dismissed, ["chat"]);
            })
            .unwrap();
    }

    #[gpui::test]
    fn open_in_right_panel_moves_the_float_into_a_tab(cx: &mut gpui::TestAppContext) {
        let (window, _dir) = device_shell(cx);
        window
            .update(cx, |shell, window, cx| {
                shell.open_device_float("chat".into(), target("PAD"), cx);
                shell.open_float_in_panel("chat", target("PAD"), window, cx);
                assert!(shell.device_shell.floats.is_empty());
                assert!(shell.right_pane_open(cx));
                let tabs = device_tabs(shell);
                assert_eq!(tabs.len(), 1);
                assert_eq!(shell.device_tab_target(tabs[0], cx), Some(target("PAD")));
                assert_eq!(
                    shell.resolved_right_active(cx),
                    RightSurface::Device(tabs[0])
                );
            })
            .unwrap();
    }

    struct StripHost(Entity<Shell>);

    impl Render for StripHost {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let strip = self
                .0
                .update(cx, |shell, cx| shell.render_right_tab_strip(cx));
            div().size_full().child(div().h(px(40.)).child(strip))
        }
    }

    #[gpui::test]
    fn double_clicking_a_device_tab_renames_it_and_escape_reverts(cx: &mut gpui::TestAppContext) {
        let (window, _dir) = device_shell(cx);
        let shell = window.root(cx).unwrap();
        window
            .update(cx, |shell, window, cx| {
                shell.open_device_tab("chat", target("SIM"), false, window, cx);
            })
            .unwrap();
        let (_, cx) = cx.add_window_view(|_, _| StripHost(shell.clone()));
        cx.run_until_parked();
        let tab = cx
            .debug_bounds("right-surface-tab-0")
            .expect("the device tab renders");
        let at = tab.center();
        for click_count in [1, 2] {
            cx.simulate_event(gpui::MouseDownEvent {
                position: at,
                button: MouseButton::Left,
                modifiers: gpui::Modifiers::default(),
                click_count,
                first_mouse: false,
            });
            cx.simulate_event(gpui::MouseUpEvent {
                position: at,
                button: MouseButton::Left,
                modifiers: gpui::Modifiers::default(),
                click_count,
            });
        }
        cx.run_until_parked();
        assert!(cx.debug_bounds("device-tab-rename").is_some());
        cx.simulate_input("My phone");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert!(cx.debug_bounds("device-tab-rename").is_none());
        let title = |cx: &mut gpui::VisualTestContext| {
            shell.read_with(cx, |shell, cx| {
                shell.right_surface_rows(cx)[0].1.to_string()
            })
        };
        assert_eq!(title(cx), "My phone");

        let id = shell.read_with(cx, |shell, _| device_tabs(shell)[0]);
        cx.update(|window, cx| {
            shell.update(cx, |shell, cx| {
                shell.rename_device_tab(RightSurface::Device(id), window, cx)
            })
        });
        cx.run_until_parked();
        cx.simulate_input("Discarded");
        assert!(shell.update(cx, |shell, cx| shell.capture_escape_surface(cx)));
        cx.run_until_parked();
        assert_eq!(title(cx), "My phone");
        assert!(cx.debug_bounds("device-tab-rename").is_none());

        cx.update(|window, cx| {
            shell.update(cx, |shell, cx| {
                shell.chat_menu.open(ChatMenuState {
                    chat_id: String::new(),
                    tab: Some(("chat".into(), RightSurface::Device(id))),
                    position: Point::default(),
                    page: ChatMenuPage::Root,
                });
                shell.close_chat_menu(cx);
                shell.rename_device_tab(RightSurface::Device(id), window, cx);
            })
        });
        assert!(shell.read_with(cx, |shell, _| shell.chat_menu.get().is_some()));
        cx.simulate_input("Discarded");
        assert!(shell.update(cx, |shell, cx| shell.capture_escape_surface(cx)));
        cx.run_until_parked();
        assert_eq!(
            title(cx),
            "My phone",
            "Escape reverts while the context menu is still closing"
        );
        assert!(cx.debug_bounds("device-tab-rename").is_none());
    }

    #[gpui::test]
    fn the_device_tool_opens_a_picker_tab_then_steps_aside_and_hides_for_a_remote_chat(
        cx: &mut gpui::TestAppContext,
    ) {
        let (window, _dir) = device_shell(cx);
        let shell = window.root(cx).unwrap();
        let (_, cx) = cx.add_window_view(|_, _| StripHost(shell.clone()));
        cx.run_until_parked();
        let tool = cx
            .debug_bounds("pane-tool-device")
            .expect("the strip lists the Device tool");
        cx.simulate_click(tool.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        shell.read_with(cx, |shell, cx| {
            let tabs = device_tabs(shell);
            assert_eq!(tabs.len(), 1, "the tool opens one picker tab");
            assert_eq!(shell.device_tab_target(tabs[0], cx), None);
            assert_eq!(
                shell.resolved_right_active(cx),
                RightSurface::Device(tabs[0])
            );
        });
        assert!(
            cx.debug_bounds("pane-tool-device").is_none(),
            "the device tab replaces the tool"
        );

        cx.update(|_, cx| {
            shell.update(cx, |shell, cx| {
                shell.right_tabs.clear();
                shell.state.update(cx, |state, _| {
                    state.chats = vec![
                        serde_json::from_value(serde_json::json!({
                            "id": "chat",
                            "deviceId": "other-mac",
                            "archived": false,
                            "createdAt": "2026-09-28T00:00:00Z"
                        }))
                        .unwrap(),
                    ];
                    state.local_device_id = Some("this-mac".into());
                });
                cx.notify();
            })
        });
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("pane-tool-device").is_none(),
            "a remote chat has no Device tool"
        );
    }

    #[gpui::test]
    fn the_float_starts_below_the_header_with_the_edge_gap(cx: &mut gpui::TestAppContext) {
        let (window, _dir) = device_shell(cx);
        window
            .update(cx, |shell, _, cx| {
                shell.debug_gate = Some(GatePhase::Ready);
                shell.open_device_float("chat".into(), target("SIM"), cx);
            })
            .unwrap();
        let cx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        crate::device_panel::tests::wait(cx, |cx| cx.debug_bounds("device-float").is_some());
        let column = cx.debug_bounds("chat-dropzone").unwrap();
        let card = cx.debug_bounds("device-float").unwrap();
        assert!(
            card.top() >= column.top() + px(Theme::TITLEBAR_HEIGHT + 12.0),
            "the float {card:?} starts below the header of {column:?}"
        );
        assert_eq!(card.right(), column.right() - px(12.0));
    }
}
