use super::*;
use crate::device_panel::{DevicePanel, DevicePanelEvent};

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

    pub(super) fn add_device_surface(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        if self.device_surface_unavailable(cx).is_some() {
            return None;
        }
        let key = self.panel_key(cx);
        self.device_seq += 1;
        let id = self.device_seq;
        let chat_id = self.active_chat.clone();
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
            },
        );
        self.device_panels.insert(id, (panel, [observer, events]));
        self.right_tabs
            .entry(key)
            .or_default()
            .push(RightSurface::Device(id));
        self.set_right_active(RightSurface::Device(id), cx);
        Some(id)
    }

    pub(super) fn device_tab_title(&self, id: u64, cx: &App) -> Option<SharedString> {
        self.device_panels
            .get(&id)
            .map(|(panel, _)| panel.read(cx).title(cx))
    }

    pub(super) fn render_device_surface(&mut self, id: u64, cx: &mut Context<Self>) -> AnyElement {
        match self.device_panels.get(&id) {
            Some((panel, _)) => panel.clone().into_any_element(),
            None => self.render_surface_picker(cx),
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
}
