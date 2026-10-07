use std::collections::{BTreeSet, HashSet};

use gpui::{
    AnyElement, Context, Entity, EventEmitter, SharedString, Stateful, Subscription, Task, Window,
    div, prelude::*, px,
};
use zeron_proto::tools::{TOOL_GROUPS, ToolGroup, ToolGroupInfo, ToolSettings, ToolSettingsState};
use zeron_rpc::methods;

use super::widgets;
use crate::device_panel::setup::activate;
use crate::icons;
use crate::popover::{self, Loadable};
use crate::shell::SettingsSection;
use crate::state::AppState;
use crate::theme::Theme;

const SUBTITLE: &str = "The tools Zeron adds to every agent through its zeron MCP server, on every provider. A chat that is already running keeps its tool list until its next turn; a call to a tool you turn off is refused right away.";

pub enum ToolsPageEvent {
    OpenSection(SettingsSection),
}

pub struct ToolsPage {
    state: Entity<AppState>,
    scroll: widgets::PageScroll,
    settings: Loadable<ToolSettings>,
    open: HashSet<ToolGroup>,
    error: Option<SharedString>,
    task: Option<Task<()>>,
    switch_task: Option<Task<()>>,
    _state: Subscription,
}

impl EventEmitter<ToolsPageEvent> for ToolsPage {}

impl ToolsPage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let observer = cx.observe(&state, |page: &mut Self, _, cx| {
            if matches!(page.settings, Loadable::Idle) {
                page.call(None, cx);
            }
            cx.notify();
        });
        let mut page = Self {
            state,
            scroll: widgets::PageScroll::default(),
            settings: Loadable::Idle,
            open: HashSet::new(),
            error: None,
            task: None,
            switch_task: None,
            _state: observer,
        };
        page.call(None, cx);
        page
    }

    fn call(&mut self, save: Option<ToolSettings>, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let (method, params) = match &save {
            Some(settings) => (
                methods::SET_TOOL_SETTINGS,
                serde_json::to_value(settings).unwrap_or_default(),
            ),
            None => (methods::GET_TOOL_SETTINGS, serde_json::json!({})),
        };
        let previous = self.settings.ready().cloned();
        match save {
            Some(settings) => self.settings = Loadable::Ready(settings),
            None if matches!(self.settings, Loadable::Idle) => self.settings = Loadable::Loading,
            None => {}
        }
        self.task = Some(cx.spawn(async move |this, cx| {
            let reply = engine
                .client()
                .call_as::<ToolSettingsState>(method, params)
                .await;
            this.update(cx, |page, cx| {
                match reply {
                    Ok(state) => {
                        page.settings = Loadable::Ready(state.settings);
                        page.error = None;
                    }
                    Err(error) => match previous {
                        Some(previous) => {
                            page.settings = Loadable::Ready(previous);
                            page.error = Some(format!("Could not save: {error}").into());
                        }
                        None => page.settings = Loadable::Error(error.to_string()),
                    },
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn edit(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut ToolSettings)) {
        let Some(mut next) = self.settings.ready().cloned() else {
            return;
        };
        change(&mut next);
        self.call(Some(next), cx);
    }

    fn computer_on(&self, cx: &gpui::App) -> Option<bool> {
        self.state
            .read(cx)
            .computer_settings
            .as_ref()
            .map(|settings| settings.computer_control_enabled)
    }

    fn devices(&self, cx: &gpui::App) -> Option<(bool, bool)> {
        self.state
            .read(cx)
            .device_hub
            .as_ref()
            .map(|hub| (hub.settings.enabled, hub.settings.agent_access_enabled))
    }

    fn group_on(&self, settings: &ToolSettings, group: ToolGroup, cx: &gpui::App) -> bool {
        match group {
            ToolGroup::Computer => self.computer_on(cx).unwrap_or(false),
            ToolGroup::Devices => self.devices(cx).is_some_and(|(hub, agent)| hub && agent),
            _ => settings.group_on(group),
        }
    }

    fn set_group(&mut self, group: ToolGroup, on: bool, cx: &mut Context<Self>) {
        match group {
            ToolGroup::Computer => self.set_computer(on, cx),
            ToolGroup::Devices => self.write_switch(
                methods::CONFIGURE_DEVICE_HUB,
                serde_json::json!({ "agentAccessEnabled": on }),
                cx,
            ),
            _ => self.edit(cx, |settings| settings.set_group(group, on)),
        }
    }

    fn set_computer(&mut self, on: bool, cx: &mut Context<Self>) {
        let Some(mut next) = self.state.read(cx).computer_settings.clone() else {
            return;
        };
        next.computer_control_enabled = on;
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let params = serde_json::to_value(&next).unwrap_or_default();
        self.switch_task = Some(cx.spawn(async move |this, cx| {
            let reply = engine
                .client()
                .call(methods::SET_COMPUTER_SETTINGS, params)
                .await;
            this.update(cx, |page, cx| {
                page.switch_task = None;
                match reply {
                    Ok(_) => page
                        .state
                        .update(cx, |state, _| state.computer_settings = Some(next)),
                    Err(error) => page.error = Some(format!("Could not save: {error}").into()),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn write_switch(
        &mut self,
        method: &'static str,
        params: serde_json::Value,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.switch_task = Some(cx.spawn(async move |this, cx| {
            let reply = engine.client().call(method, params).await;
            this.update(cx, |page, cx| {
                page.switch_task = None;
                if let Err(error) = reply {
                    page.error = Some(format!("Could not save: {error}").into());
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

    fn disabled(&self, settings: &ToolSettings, cx: &gpui::App) -> BTreeSet<&'static str> {
        let also_off: Vec<ToolGroup> = [ToolGroup::Computer, ToolGroup::Devices]
            .into_iter()
            .filter(|group| !self.group_on(settings, *group, cx))
            .collect();
        settings.disabled(&also_off)
    }

    fn group_card(
        &self,
        theme: &Theme,
        info: &'static ToolGroupInfo,
        settings: &ToolSettings,
        disabled: &BTreeSet<&'static str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let group = info.group;
        let tools = info.tools();
        let on = self.group_on(settings, group, cx);
        let open = self.open.contains(&group);
        let live = tools.iter().filter(|t| !disabled.contains(t.name)).count();
        let badge = if !on {
            widgets::badge(theme, "Off")
        } else if live < tools.len() {
            widgets::badge_active(theme, format!("{live} of {}", tools.len()))
        } else {
            widgets::badge(theme, format!("{} tools", tools.len()))
        };
        let (switch_ready, linked) = match group {
            ToolGroup::Computer => (
                self.computer_on(cx).is_some(),
                Some((
                    "Same switch as Settings > Computer",
                    SettingsSection::Computer,
                )),
            ),
            ToolGroup::Devices => (
                self.devices(cx).is_some_and(|(hub, _)| hub),
                Some((
                    if self.devices(cx).is_some_and(|(hub, _)| hub) {
                        "Same switch as agent access in Settings > Device hub"
                    } else {
                        "Turn on Device hub first"
                    },
                    SettingsSection::DeviceHub,
                )),
            ),
            _ => (self.settings.ready().is_some(), None),
        };
        let switch_ready = switch_ready && self.switch_task.is_none();
        let mut meta = vec![div().child(info.summary).into_any_element()];
        if let Some((label, section)) = linked {
            meta.push(
                div()
                    .id(SharedString::from(format!("tools-link-{group:?}")))
                    .cursor_pointer()
                    .underline()
                    .text_color(theme.text)
                    .child(label)
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.stop_propagation();
                        cx.emit(ToolsPageEvent::OpenSection(section));
                    }))
                    .into_any_element(),
            );
        }
        let header = widgets::card_row(theme, true)
            .id(SharedString::from(format!("tools-group-{group:?}")))
            .cursor_pointer()
            .on_click(cx.listener(move |page, _, _, cx| {
                if !page.open.remove(&group) {
                    page.open.insert(group);
                }
                cx.notify();
            }))
            .child(
                crate::icons::icon(if open {
                    icons::ALT_ARROW_DOWN
                } else {
                    icons::ALT_ARROW_RIGHT
                })
                .size(px(14.0))
                .text_color(theme.text_muted),
            )
            .child(widgets::row_tile(theme, group_icon(group)))
            .child(
                div()
                    .flex_1()
                    .min_w(px(160.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(widgets::row_title(theme, info.title))
                            .child(badge),
                    )
                    .child(widgets::meta_line(theme, meta)),
            )
            .child(
                div()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(activate(
                        switch(
                            theme,
                            format!("tools-group-switch-{group:?}"),
                            info.title,
                            on,
                        ),
                        switch_ready,
                        cx,
                        move |page: &mut Self, cx| {
                            cx.stop_propagation();
                            page.set_group(group, !on, cx)
                        },
                    )),
            );
        let mut card = widgets::section_card(theme).mt(px(10.0)).child(header);
        if open {
            if !on {
                card = card.child(
                    div()
                        .mx(px(16.0))
                        .pl(px(54.0))
                        .py(px(8.0))
                        .border_t_1()
                        .border_color(widgets::row_divider(theme))
                        .text_size(crate::typography::ui_rems(widgets::ROW_DESCRIPTION_SIZE))
                        .text_color(theme.text_muted)
                        .child("The whole group is off. Your choices below are kept for when you turn it back on."),
                );
            }
            for tool in tools {
                let need_off = tool.needs.is_some_and(|need| disabled.contains(need));
                let editable = on && !need_off && self.settings.ready().is_some();
                let chosen = settings.tool_on(tool.name);
                let name = tool.name;
                let mut text = div().flex_1().min_w_0().child(
                    div()
                        .font_family(theme.font_mono.clone())
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.text)
                        .child(tool.name),
                );
                if !tool.summary.is_empty() {
                    text = text.child(
                        div()
                            .mt(px(2.0))
                            .text_size(crate::typography::ui_rems(widgets::ROW_DESCRIPTION_SIZE))
                            .text_color(theme.text_muted)
                            .child(tool.summary),
                    );
                }
                if let Some(need) = tool.needs.filter(|_| on && need_off) {
                    text = text.child(
                        div()
                            .mt(px(2.0))
                            .text_size(crate::typography::ui_rems(widgets::ROW_DESCRIPTION_SIZE))
                            .text_color(theme.warning)
                            .child(SharedString::from(format!("Off while {need} is off"))),
                    );
                }
                card = card.child(
                    div()
                        .mx(px(16.0))
                        .pl(px(54.0))
                        .py(px(8.0))
                        .min_h(px(if tool.summary.is_empty() { 36.0 } else { 48.0 }))
                        .border_t_1()
                        .border_color(widgets::row_divider(theme))
                        .flex()
                        .items_center()
                        .gap(px(12.0))
                        .child(text.when(!on || need_off, |el| el.opacity(0.5)))
                        .child(activate(
                            switch(theme, format!("tools-switch-{name}"), name, chosen),
                            editable,
                            cx,
                            move |page: &mut Self, cx| {
                                page.edit(cx, |settings| settings.set_tool(name, !chosen))
                            },
                        )),
                );
            }
        }
        card.into_any_element()
    }
}

fn group_icon(group: ToolGroup) -> &'static str {
    match group {
        ToolGroup::Threads => icons::CHAT_ROUND_LINE,
        ToolGroup::Workspace => icons::TERMINAL,
        ToolGroup::Delegation => icons::BOT,
        ToolGroup::Devices => icons::SMARTPHONE,
        ToolGroup::Computer => icons::CURSOR,
    }
}

fn switch(
    theme: &Theme,
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    on: bool,
) -> Stateful<gpui::Div> {
    let id: SharedString = id.into();
    let selector = id.clone();
    div()
        .id(id.clone())
        .debug_selector(move || selector.to_string())
        .flex_none()
        .tab_index(0)
        .role(gpui::Role::Switch)
        .aria_label(label.into())
        .aria_toggled(if on {
            gpui::Toggled::True
        } else {
            gpui::Toggled::False
        })
        .child(widgets::toggle_switch(theme, on, id))
}

impl popover::ScrollRailHost for ToolsPage {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        self.scroll.rail_bar()
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.scroll.rail_scroll()
    }
}

impl Render for ToolsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_settings_surface();
        let mut column = widgets::page_column();
        match self.settings.ready().cloned() {
            Some(settings) => {
                let disabled = self.disabled(&settings, cx);
                let total: usize = TOOL_GROUPS.iter().map(|info| info.tools().len()).sum();
                column = column
                    .child(widgets::page_header(&theme, "Tools", None))
                    .child(widgets::page_subtitle(&theme, SUBTITLE))
                    .child(widgets::page_subtitle(
                        &theme,
                        format!("{} of {total} on", total - disabled.len()),
                    ))
                    .when_some(self.error.clone(), |el, error| {
                        el.child(
                            div()
                                .mt(px(12.0))
                                .child(widgets::error_strip(&theme, error)),
                        )
                    })
                    .child(div().mt(px(14.0)));
                for info in TOOL_GROUPS {
                    column = column.child(self.group_card(&theme, info, &settings, &disabled, cx));
                }
                column = column.child(
                    div()
                        .mt(px(14.0))
                        .px(px(4.0))
                        .text_size(crate::typography::ui_rems(widgets::ROW_DESCRIPTION_SIZE))
                        .text_color(theme.text_muted)
                        .child("Applies to every chat on this engine."),
                );
            }
            None => {
                let note = match &self.settings {
                    Loadable::Error(error) => error.clone(),
                    _ => "Loading…".to_string(),
                };
                column = column
                    .child(widgets::page_header(&theme, "Tools", None))
                    .child(widgets::page_subtitle(&theme, note));
            }
        }
        let scrollbar = popover::rail(self, "tools-page-scrollbar", &theme, cx);
        div()
            .id("tools-page-host")
            .relative()
            .size_full()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .child(
                crate::edge_fade::edge_faded(
                    16.0,
                    true,
                    true,
                    div()
                        .id("tools-page")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll.scroll)
                        .child(column),
                )
                .fade_overflow_y(&self.scroll.scroll),
            )
            .children(scrollbar)
    }
}
