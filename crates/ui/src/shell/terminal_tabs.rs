use super::*;
use zeron_proto::{TerminalInfo, TerminalSession};

pub(super) const TERMINAL_TAB_PREFIX: &str = "term:";
const TERMINAL_TAB_SCOPE: &str = "term";

pub(super) fn is_terminal_tab(id: &str) -> bool {
    id.starts_with(TERMINAL_TAB_PREFIX)
}

fn terminal_tab_id(terminal_id: &str) -> String {
    format!("{TERMINAL_TAB_PREFIX}{terminal_id}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TerminalLaunch {
    Shell,
    ClaudeCode,
    Codex,
}

impl TerminalLaunch {
    pub(super) const ALL: [Self; 3] = [Self::Shell, Self::ClaudeCode, Self::Codex];

    fn command(self) -> Option<&'static str> {
        match self {
            Self::Shell => None,
            Self::ClaudeCode => Some("claude"),
            Self::Codex => Some("codex"),
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Shell => "Terminal",
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
        }
    }

    pub(super) fn menu_label(self) -> &'static str {
        match self {
            Self::Shell => "New terminal",
            Self::ClaudeCode => "New Claude Code CLI",
            Self::Codex => "New Codex CLI",
        }
    }

    pub(super) fn brand(self) -> (&'static str, Option<gpui::Hsla>) {
        match self {
            Self::Shell => (icons::TERMINAL, None),
            Self::ClaudeCode => crate::pickers::harness_brand_icon(HarnessId::ClaudeCode),
            Self::Codex => crate::pickers::harness_brand_icon(HarnessId::Codex),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct TerminalTabRow {
    pub id: String,
    pub key: u64,
    pub title: SharedString,
    pub exited: bool,
    pub launch: TerminalLaunch,
}

/// Boot syncs tabs before ListTerminals answers; pruning then would drop every stored tab.
pub(super) fn keep_terminal_tab(
    id: &str,
    synced: bool,
    live: &std::collections::HashSet<String>,
) -> bool {
    !synced || live.contains(id)
}

impl Shell {
    pub(super) fn main_terminal_panel(&mut self, cx: &mut Context<Self>) -> Entity<TerminalPanel> {
        if let Some(panel) = &self.main_terminal {
            return panel.clone();
        }
        let state = self.state.clone();
        let panel = cx.new(|cx| {
            let mut panel = TerminalPanel::new_scoped(state, TERMINAL_TAB_SCOPE, cx);
            panel.set_open(true, cx);
            panel
        });
        self._main_terminal_observe = Some(cx.observe(&panel, |this, panel, cx| {
            let signature: Vec<_> = panel
                .read(cx)
                .tabs_of(TERMINAL_TAB_SCOPE)
                .into_iter()
                .map(|(key, _, title, exited)| (key, title, exited))
                .collect();
            if signature != this.terminal_tab_signature {
                this.terminal_tab_signature = signature;
                cx.notify();
            }
        }));
        self.main_terminal = Some(panel.clone());
        panel
    }

    pub(super) fn terminal_tab_rows(&self, cx: &App) -> Vec<TerminalTabRow> {
        let Some(panel) = &self.main_terminal else {
            return Vec::new();
        };
        panel
            .read(cx)
            .tabs_of(TERMINAL_TAB_SCOPE)
            .into_iter()
            .filter_map(|(key, terminal_id, title, exited)| {
                let id = terminal_tab_id(&terminal_id?);
                Some(TerminalTabRow {
                    launch: self
                        .terminal_launches
                        .get(&id)
                        .copied()
                        .unwrap_or(TerminalLaunch::Shell),
                    id,
                    key,
                    title,
                    exited,
                })
            })
            .collect()
    }

    pub(super) fn terminal_tab_row(&self, id: &str, cx: &App) -> Option<TerminalTabRow> {
        self.terminal_tab_rows(cx)
            .into_iter()
            .find(|row| row.id == id)
    }

    pub(super) fn live_terminal_tab_ids(&self, cx: &App) -> std::collections::HashSet<String> {
        self.terminal_tab_rows(cx)
            .into_iter()
            .map(|row| row.id)
            .collect()
    }

    pub(super) fn selected_tab_id(&self, cx: &App) -> Option<String> {
        match self.route {
            Route::Terminal => self.active_terminal_tab.clone(),
            Route::PullRequests
            | Route::Usage
            | Route::Inbox
            | Route::Tasks
            | Route::TaskCenter
            | Route::Supervisor
            | Route::Docs => None,
            _ => self.state.read(cx).focused_id(),
        }
    }

    fn terminal_tab_cwd(&self, cx: &App) -> String {
        let state = self.state.read(cx);
        let local = state.local_device_id.as_deref();
        let chat_cwd = state
            .selected_chat
            .as_deref()
            .and_then(|id| state.chats.iter().find(|chat| chat.id == id))
            .filter(|chat| Some(chat.device_id.as_str()) == local)
            .and_then(|chat| chat.cwd.clone());
        chat_cwd
            .or_else(|| {
                state
                    .selected_space
                    .as_deref()
                    .and_then(|id| state.space_row(id))
                    .filter(|space| Some(space.device_id.as_str()) == local)
                    .map(|space| space.path.clone())
            })
            .filter(|path| !path.trim().is_empty())
            .unwrap_or_else(|| "~".to_string())
    }

    pub(super) fn open_terminal_tab(&mut self, launch: TerminalLaunch, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let panel = self.main_terminal_panel(cx);
        let (cols, rows) = panel.read(cx).grid_size().unwrap_or((80, 24));
        let mut params = serde_json::json!({
            "chatId": TERMINAL_TAB_SCOPE,
            "cols": cols,
            "rows": rows,
            "cwd": self.terminal_tab_cwd(cx),
        });
        if let (Some(command), Some(object)) = (launch.command(), params.as_object_mut()) {
            object.insert("command".into(), command.into());
        }
        cx.spawn(async move |this, cx| {
            let session = match engine
                .client()
                .call_as::<TerminalSession>(methods::OPEN_TERMINAL, params)
                .await
            {
                Ok(session) => session,
                Err(err) => {
                    tracing::warn!(error = %err, "terminal tab: OpenTerminal failed");
                    return;
                }
            };
            this.update(cx, |shell, cx| {
                let id = terminal_tab_id(&session.id);
                shell.terminal_launches.insert(id.clone(), launch);
                shell.attach_terminal_tab(session, launch.title(), cx);
                shell.show_terminal_tab(id, cx);
            })
            .ok();
        })
        .detach();
    }

    fn attach_terminal_tab(
        &mut self,
        session: TerminalSession,
        title: &str,
        cx: &mut Context<Self>,
    ) {
        let panel = self.main_terminal_panel(cx);
        panel.update(cx, |panel, cx| {
            let key = panel.reserve_tab_for_chat(TERMINAL_TAB_SCOPE.to_string(), title, cx);
            panel.attach_reserved_session(TERMINAL_TAB_SCOPE, key, session, None, cx);
        });
    }

    pub(super) fn show_terminal_tab(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(row) = self.terminal_tab_row(&id, cx) else {
            return;
        };
        self.snapshot_current_chat(cx);
        self.command_palette = None;
        self.set_voice_stage_open(false, cx);
        let tabs = self.settings.open_tabs.get_or_insert_with(Vec::new);
        if !tabs.contains(&id) {
            tabs.push(id.clone());
            self.schedule_save(cx);
        }
        self.route = Route::Terminal;
        self.active_terminal_tab = Some(id);
        let panel = self.main_terminal_panel(cx);
        panel.update(cx, |panel, cx| {
            panel.select_tab_by_key(row.key, cx);
            panel.request_focus(cx);
        });
        cx.notify();
    }

    fn leave_terminal_route(&mut self, cx: &mut Context<Self>) {
        self.route = Route::Chat;
        self.focus_composer(cx);
        cx.notify();
    }

    pub(super) fn close_terminal_tab(&mut self, id: &str, cx: &mut Context<Self>) {
        let order = self.open_tab_ids(cx);
        if self.terminal_tab_row(id, cx).is_some_and(|row| row.exited) {
            return self.kill_terminal_tab(id, cx);
        }
        if let Some(tabs) = self.settings.open_tabs.as_mut() {
            tabs.retain(|tab| tab != id);
        }
        self.schedule_save(cx);
        self.after_terminal_tab_left(id, &order, cx);
    }

    pub(super) fn kill_terminal_tab(&mut self, id: &str, cx: &mut Context<Self>) {
        let order = self.open_tab_ids(cx);
        if let Some(row) = self.terminal_tab_row(id, cx)
            && let Some(panel) = self.main_terminal.clone()
        {
            panel.update(cx, |panel, cx| panel.kill_tab_by_key(row.key, cx));
        }
        if let Some(tabs) = self.settings.open_tabs.as_mut() {
            tabs.retain(|tab| tab != id);
        }
        self.terminal_launches.remove(id);
        self.schedule_save(cx);
        self.after_terminal_tab_left(id, &order, cx);
    }

    fn after_terminal_tab_left(&mut self, id: &str, order: &[String], cx: &mut Context<Self>) {
        let shown = matches!(self.route, Route::Terminal)
            && self.active_terminal_tab.as_deref() == Some(id);
        if !shown {
            cx.notify();
            return;
        }
        self.active_terminal_tab = None;
        match tabs::next_after_close(order, id) {
            Some(next) => self.navigate_to(Some(next), false, cx),
            None => self.leave_terminal_route(cx),
        }
    }

    pub(super) fn sync_terminal_tabs(&mut self, cx: &mut Context<Self>) {
        if self.terminals_synced || self.terminals_sync_task.is_some() {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.terminals_sync_task = Some(cx.spawn(async move |this, cx| {
            let listed = engine
                .client()
                .call_as::<Vec<TerminalInfo>>(
                    methods::LIST_TERMINALS,
                    serde_json::json!({ "chatId": TERMINAL_TAB_SCOPE }),
                )
                .await;
            this.update(cx, |shell, cx| {
                shell.terminals_sync_task = None;
                let terminals = match listed {
                    Ok(terminals) => terminals,
                    Err(err) => {
                        tracing::warn!(error = %err, "terminal tabs: ListTerminals failed");
                        Vec::new()
                    }
                };
                let known = shell.live_terminal_tab_ids(cx);
                for info in terminals.into_iter().rev() {
                    if known.contains(&terminal_tab_id(&info.id)) {
                        continue;
                    }
                    let title = info.shell.clone();
                    shell.attach_terminal_tab(
                        TerminalSession {
                            id: info.id,
                            cwd: info.cwd,
                            shell: info.shell,
                            host_replies: true,
                        },
                        &title,
                        cx,
                    );
                }
                shell.terminals_synced = true;
                shell.sync_open_tabs(cx);
                cx.notify();
            })
            .ok();
        }));
    }

    fn close_new_tab_menu(&mut self, cx: &mut Context<Self>) {
        if self.new_tab_menu.begin_close() {
            popover::reap_popup(cx, |shell: &mut Self| &mut shell.new_tab_menu);
        }
        cx.notify();
    }

    pub(super) fn render_titlebar_plus(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let fade_key = "window-control-titlebar-new-session";
        let mut plus = div()
            .id("titlebar-new-session")
            .debug_selector(|| "titlebar-new-session".into())
            .size(px(24.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .bg(motion::hover_blend(
                fade_key,
                theme.glass_hover().opacity(0.0),
                theme.glass_hover(),
            ))
            .on_hover(motion::hover_listener(fade_key))
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, _| {
                    window.prevent_default();
                    this.new_tab_menu.note_trigger_press();
                }),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                if this.new_tab_menu.take_press_was_open() {
                    this.close_new_tab_menu(cx);
                } else {
                    this.new_tab_menu.open(());
                    cx.notify();
                }
            }))
            .tooltip(crate::settings::widgets::text_tooltip("New tab"))
            .child(
                icon(icons::PLUS)
                    .size(px(16.0))
                    .text_color(theme.text_muted),
            );
        if self.new_tab_menu.get().is_some() {
            let closing = self.new_tab_menu.closing_since();
            let row_icon = |(path, tint): (&'static str, Option<gpui::Hsla>)| {
                icon(path)
                    .size(px(13.0))
                    .text_color(tint.unwrap_or(theme.text_muted))
            };
            let mut rows = div().flex().flex_col().gap(px(2.0)).child(
                popover::menu_row(&theme, false, "new-tab-chat")
                    .id("new-tab-chat-row")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.close_new_tab_menu(cx);
                        this.open_new_session(None, cx);
                    }))
                    .child(row_icon((icons::CHAT_ROUND_LINE, None)))
                    .child(SharedString::from("New chat")),
            );
            rows = rows.child(popover::menu_separator());
            for launch in TerminalLaunch::ALL {
                rows = rows.child(
                    popover::menu_row(&theme, false, format!("new-tab-{launch:?}"))
                        .id(SharedString::from(format!("new-tab-{launch:?}-row")))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.close_new_tab_menu(cx);
                            this.open_terminal_tab(launch, cx);
                        }))
                        .child(row_icon(launch.brand()))
                        .child(SharedString::from(launch.menu_label())),
                );
            }
            let menu = popover::popover_card(&theme)
                .w(px(200.0))
                .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_new_tab_menu(cx)))
                .child(rows)
                .into_any_element();
            plus = plus.relative().child(popover::anchored_menu_below_gap(
                "new-tab-menu",
                menu,
                closing,
                10.0,
            ));
        }
        plus.into_any_element()
    }

    pub(super) fn render_terminal_section(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let rows = self.terminal_tab_rows(cx);
        if rows.is_empty() {
            return None;
        }
        let selected = self
            .active_terminal_tab
            .clone()
            .filter(|_| matches!(self.route, Route::Terminal));
        let header = spaces::sidebar_disclosure_header(
            theme,
            None,
            "Terminals".into(),
            None,
            None,
            div().into_any_element(),
        )
        .cursor_default();
        let mut list = div().flex().flex_col().gap(px(2.0));
        for row in rows {
            let is_selected = selected.as_deref() == Some(row.id.as_str());
            list = list.child(self.render_terminal_row(row, is_selected, theme, cx));
        }
        Some(
            div()
                .flex()
                .flex_col()
                .mt(px(spaces::SIDEBAR_SECTION_GAP))
                .child(header)
                .child(list)
                .into_any_element(),
        )
    }

    fn render_terminal_row(
        &self,
        row: TerminalTabRow,
        selected: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row_id = format!("terminal-row-{}", row.id);
        let hovered = self.chat_status_hover.as_deref() == Some(row_id.as_str());
        let fade_key = format!("{row_id}-hover");
        let selected_wash = crate::theme::glass_selected_bg();
        let rest_bg = if selected {
            selected_wash
        } else {
            crate::theme::wash(0.0)
        };
        let hover_bg = if selected {
            selected_wash
        } else {
            theme.glass_hover()
        };
        let text = if row.exited {
            theme.text_muted.opacity(0.5)
        } else if selected {
            theme.text
        } else {
            theme.text.opacity(0.8)
        };
        let (path, tint) = row.launch.brand();
        let open_id = row.id.clone();
        let kill_id = row.id.clone();
        let hover_id = row_id.clone();
        let trailing = hovered.then(|| {
            div()
                .id(SharedString::from(format!("terminal-kill-{}", row.id)))
                .size(px(18.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .hover(|s| s.bg(crate::theme::wash(0.14)))
                .tooltip(crate::settings::widgets::text_tooltip("Close terminal"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.kill_terminal_tab(&kill_id, cx);
                }))
                .child(
                    icon(icons::CLOSE)
                        .size(px(11.0))
                        .text_color(theme.text_muted),
                )
        });
        div()
            .id(SharedString::from(row_id))
            .h(px(30.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(Theme::SPACE_SM))
            .rounded(px(8.0))
            .px(px(Theme::SPACE_SM))
            .text_size(crate::typography::ui_rems(13.0))
            .text_color(text)
            .bg(motion::hover_blend(&fade_key, rest_bg, hover_bg))
            .on_hover({
                let fade_hover = motion::hover_listener(fade_key.clone());
                cx.listener(move |this, hovered: &bool, window, cx| {
                    fade_hover(hovered, window, cx);
                    if *hovered {
                        if this.chat_status_hover.as_deref() != Some(hover_id.as_str()) {
                            this.chat_status_hover = Some(hover_id.clone());
                            cx.notify();
                        }
                    } else if this.chat_status_hover.as_deref() == Some(hover_id.as_str()) {
                        this.chat_status_hover = None;
                        cx.notify();
                    }
                })
            })
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.show_terminal_tab(open_id.clone(), cx);
            }))
            .child(
                icon(path)
                    .size(px(13.0))
                    .flex_none()
                    .text_color(tint.unwrap_or(theme.text_muted)),
            )
            .child(div().flex_1().min_w_0().truncate().child(row.title))
            .children(trailing)
            .into_any_element()
    }

    pub(super) fn render_terminal_column(
        &mut self,
        width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let ui_settings = settings::current(cx);
        let background = ui_settings
            .terminal_background
            .resolve(ui_settings.chat_background);
        let artwork = ui_settings
            .new_thread_composer_background
            .as_ref()
            .filter(|_| background.visible())
            .and_then(|artwork| {
                crate::new_thread_background_effects::prepare(
                    ui_settings.new_thread_background_effect,
                    &theme,
                    std::path::Path::new(&artwork.path),
                    background.blur_level(),
                    cx,
                )
            });
        let frame_time = self.render_time.unwrap_or_else(motion::now);
        let artwork_opacity = self.chat_artwork_ready.opacity(
            artwork.as_ref().map(|image| image.id),
            self.reduced_motion,
            frame_time,
        );
        let layer = background.visible().then(|| {
            if artwork.is_some() && artwork_opacity < 1.0 {
                window.request_animation_frame();
            }
            crate::chat_background::layer(
                artwork,
                background,
                Some(width),
                crate::chat_background::layer_height(
                    background.placement,
                    self.viewport_height,
                    new_thread_background_height(self.viewport_height),
                ),
                theme.bg,
                artwork_opacity,
                None,
            )
        });
        let resize_suspended = self.tween_active(self.right_tween)
            || self.tween_active(self.files_tween)
            || self.tween_active(self.sidebar_tween);
        let panel = self.main_terminal_panel(cx);
        let key = self
            .active_terminal_tab
            .as_deref()
            .and_then(|id| self.terminal_tab_row(id, cx))
            .map(|row| row.key);
        panel.update(cx, |panel, cx| {
            panel.set_resize_suspended(resize_suspended);
            if let Some(key) = key {
                panel.select_tab_by_key(key, cx);
            }
        });
        div()
            .debug_selector(|| "terminal-tab-column".into())
            .h_full()
            .w(px(width))
            .flex_none()
            .relative()
            .overflow_hidden()
            .children(layer)
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .pt(px(Theme::TITLEBAR_HEIGHT))
                    .px(px(Theme::SPACE_MD))
                    .pb(px(Theme::SPACE_SM))
                    .child(panel),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_terminal_tabs_survive_until_the_engine_answers() {
        let live: std::collections::HashSet<String> = ["term:a".to_string()].into();
        assert!(keep_terminal_tab("term:gone", false, &live));
        assert!(keep_terminal_tab("term:a", true, &live));
        assert!(!keep_terminal_tab("term:gone", true, &live));
    }

    #[test]
    fn terminal_ids_are_namespaced() {
        assert!(is_terminal_tab(&terminal_tab_id("t1")));
        assert!(!is_terminal_tab("chat-1"));
    }
}
