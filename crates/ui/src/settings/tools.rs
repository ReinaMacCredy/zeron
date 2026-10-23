use std::collections::{BTreeSet, HashSet};

use gpui::{
    AnyElement, Context, Entity, EventEmitter, SharedString, Stateful, Subscription, Task, Window,
    div, prelude::*, px,
};
use zeron_proto::tools::{TOOL_GROUPS, ToolGroup, ToolGroupInfo, ToolSettings, ToolSettingsState};
use zeron_rpc::methods;

use super::widgets::{self, ActionTone};

mod art;
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
    pending: HashSet<ToolGroup>,
    wires: Option<[art::Wire; 6]>,
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
            pending: HashSet::new(),
            wires: None,
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
                page.pending
                    .retain(|group| matches!(group, ToolGroup::Computer | ToolGroup::Devices));
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

    fn set_groups(&mut self, groups: &[ToolGroup], on: bool, cx: &mut Context<Self>) {
        let plain: Vec<ToolGroup> = groups
            .iter()
            .copied()
            .filter(|group| !matches!(group, ToolGroup::Computer | ToolGroup::Devices))
            .collect();
        if !plain.is_empty() && self.settings.ready().is_some() {
            self.pending.extend(&plain);
            self.edit(cx, |settings| {
                for group in &plain {
                    settings.set_group(*group, on);
                }
            });
        }
        let mut computer = None;
        let mut writes = Vec::new();
        if groups.contains(&ToolGroup::Computer)
            && let Some(mut next) = self.state.read(cx).computer_settings.clone()
        {
            next.computer_control_enabled = on;
            writes.push((
                ToolGroup::Computer,
                methods::SET_COMPUTER_SETTINGS,
                serde_json::to_value(&next).unwrap_or_default(),
            ));
            computer = Some(next);
        }
        if groups.contains(&ToolGroup::Devices) && self.devices(cx).is_some_and(|(hub, _)| hub) {
            writes.push((
                ToolGroup::Devices,
                methods::CONFIGURE_DEVICE_HUB,
                serde_json::json!({ "agentAccessEnabled": on }),
            ));
        }
        if writes.is_empty() {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.pending
            .extend(writes.iter().map(|(group, _, _)| *group));
        self.switch_task = Some(cx.spawn(async move |this, cx| {
            let mut computer_saved = false;
            let mut failed = None;
            for (group, method, params) in writes {
                match engine.client().call(method, params).await {
                    Ok(_) => computer_saved |= group == ToolGroup::Computer,
                    Err(error) => failed = Some(error.to_string()),
                }
            }
            this.update(cx, |page, cx| {
                page.switch_task = None;
                page.pending.remove(&ToolGroup::Computer);
                page.pending.remove(&ToolGroup::Devices);
                if let Some(next) = computer.filter(|_| computer_saved) {
                    page.state
                        .update(cx, |state, _| state.computer_settings = Some(next));
                }
                if let Some(error) = failed {
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

    fn group_state(
        &self,
        settings: &ToolSettings,
        info: &'static ToolGroupInfo,
        disabled: &BTreeSet<&'static str>,
        cx: &gpui::App,
    ) -> GroupState {
        let tools = info.tools();
        let live = tools.iter().filter(|t| !disabled.contains(t.name)).count();
        if !self.group_on(settings, info.group, cx) || live == 0 {
            GroupState::Off
        } else if live < tools.len() {
            GroupState::Partial(live)
        } else {
            GroupState::On
        }
    }

    fn turn_all(&mut self, on: bool, cx: &mut Context<Self>) {
        if on {
            self.edit(cx, |settings| {
                for info in TOOL_GROUPS {
                    for tool in info.tools() {
                        settings.set_tool(tool.name, true);
                    }
                }
            });
        }
        let groups: Vec<ToolGroup> = TOOL_GROUPS.iter().map(|info| info.group).collect();
        self.set_groups(&groups, on, cx);
    }

    fn hero(
        &self,
        theme: &Theme,
        wires: [art::Wire; 6],
        live: usize,
        total: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let saving = self.pending.len();
        let (title, body) = if saving > 0 {
            (
                format!("Saving {saving} group{}", if saving == 1 { "" } else { "s" }),
                "Zeron is updating its MCP server. A running chat picks up the change on its next turn.".to_string(),
            )
        } else if live == total {
            (
                "Every tool is live".to_string(),
                format!(
                    "Agents on this engine can call all {total} Zeron tools across {} groups.",
                    TOOL_GROUPS.len()
                ),
            )
        } else if live > 0 {
            (
                format!("{live} of {total} tools on"),
                "Open a group to pick single tools. A call to a tool you turn off is refused right away.".to_string(),
            )
        } else {
            (
                "No Zeron tools".to_string(),
                "Agents see no Zeron MCP tools. Turn a group on and calls to it work again."
                    .to_string(),
            )
        };
        let ready = self.settings.ready().is_some() && self.switch_task.is_none();
        div()
            .mt(px(14.0))
            .rounded(px(12.0))
            .bg(widgets::block_fill(theme))
            .p(px(6.0))
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(210.0))
                    .rounded(px(9.0))
                    .border_1()
                    .border_color(widgets::row_divider(theme))
                    .bg(art::backdrop(theme))
                    .overflow_hidden()
                    .child(art::hero(theme, wires, live as f32 / total.max(1) as f32).size_full()),
            )
            .child(
                div()
                    .px(px(40.0))
                    .pt(px(12.0))
                    .flex()
                    .flex_col()
                    .items_center()
                    .text_center()
                    .child(
                        div()
                            .text_size(crate::typography::ui_rems(13.5))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child(title),
                    )
                    .child(
                        div()
                            .mt(px(4.0))
                            .text_size(crate::typography::ui_rems(widgets::ROW_DESCRIPTION_SIZE))
                            .text_color(theme.text_muted)
                            .child(body),
                    ),
            )
            .child(
                div()
                    .pt(px(10.0))
                    .pb(px(10.0))
                    .flex()
                    .justify_center()
                    .gap(px(7.0))
                    .child(activate(
                        action(theme, "tools-all-on", "Turn all on", ActionTone::Solid),
                        ready && live < total,
                        cx,
                        |page: &mut Self, cx| page.turn_all(true, cx),
                    ))
                    .child(activate(
                        action(theme, "tools-all-off", "Turn all off", ActionTone::Outlined),
                        ready && live > 0,
                        cx,
                        |page: &mut Self, cx| page.turn_all(false, cx),
                    )),
            )
            .into_any_element()
    }

    fn group_card(
        &self,
        theme: &Theme,
        info: &'static ToolGroupInfo,
        settings: &ToolSettings,
        disabled: &BTreeSet<&'static str>,
        first: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let group = info.group;
        let tools = info.tools();
        let on = self.group_on(settings, group, cx);
        let open = self.open.contains(&group);
        let state = self.group_state(settings, info, disabled, cx);
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
        let count = match state {
            GroupState::Partial(live) => format!("{live}/{} TOOLS", tools.len()),
            _ => format!("{} TOOLS", tools.len()),
        };
        let control = activate(
            switch(
                theme,
                format!("tools-group-switch-{group:?}"),
                info.title,
                on,
            ),
            switch_ready && !self.pending.contains(&group),
            cx,
            move |page: &mut Self, cx| {
                cx.stop_propagation();
                page.set_groups(&[group], !on, cx)
            },
        );
        let tile_tone = if on { theme.text } else { theme.text_muted };
        let header = widgets::card_row(theme, first)
            .id(SharedString::from(format!("tools-group-{group:?}")))
            .gap(px(12.0))
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
            .child(
                div()
                    .flex_none()
                    .size(px(28.0))
                    .rounded(px(7.0))
                    .border_1()
                    .border_color(theme.border)
                    .bg(art::backdrop(theme).opacity(0.6))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        crate::icons::icon(group_icon(group))
                            .size(px(14.0))
                            .text_color(tile_tone),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(160.0))
                    .child(widgets::row_title(theme, info.title))
                    .child(widgets::meta_line(theme, meta)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .font_family(theme.font_mono.clone())
                            .text_size(crate::typography::ui_rems(10.0))
                            .text_color(theme.text_faint)
                            .child(count),
                    )
                    .child(control),
            );
        let mut card = div().flex().flex_col().child(header);
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
                        .ml(px(70.0))
                        .py(px(7.0))
                        .min_h(px(if tool.summary.is_empty() { 32.0 } else { 44.0 }))
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
            card = card.child(div().h(px(6.0)));
        }
        card.into_any_element()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GroupState {
    Off,
    Partial(usize),
    On,
}

impl GroupState {
    fn lit(self) -> art::Lit {
        match self {
            Self::Off => art::Lit::Off,
            Self::Partial(_) => art::Lit::Partial,
            Self::On => art::Lit::On,
        }
    }
}

fn action(
    theme: &Theme,
    id: &'static str,
    label: &'static str,
    tone: ActionTone,
) -> Stateful<gpui::Div> {
    widgets::text_action(theme, tone, label)
        .id(id)
        .debug_selector(move || id.to_string())
        .tab_index(0)
        .role(gpui::Role::Button)
        .aria_label(label)
}

fn group_icon(group: ToolGroup) -> &'static str {
    match group {
        ToolGroup::Threads => icons::CHAT_ROUND_LINE,
        ToolGroup::Workspace => icons::TERMINAL,
        ToolGroup::UiDriver => icons::MONITOR,
        ToolGroup::Delegation => icons::BOT,
        ToolGroup::Pages => icons::GLOBE,
        ToolGroup::Devices => icons::SMARTPHONE,
        ToolGroup::Computer => icons::CURSOR,
    }
}

pub(super) fn switch(
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
                let live = total - disabled.len();
                let states: Vec<GroupState> = TOOL_GROUPS
                    .iter()
                    .map(|info| self.group_state(&settings, info, &disabled, cx))
                    .collect();
                let animate = self.wires.is_some();
                let mut wires = self.wires.unwrap_or_default();
                for (wire, state) in wires.iter_mut().zip(&states) {
                    wire.set(state.lit(), animate);
                }
                self.wires = Some(wires);
                let groups_on = states
                    .iter()
                    .filter(|state| matches!(state, GroupState::On | GroupState::Partial(_)))
                    .count();
                let status = if !self.pending.is_empty() {
                    "SAVING"
                } else if live == total {
                    "ALL LIVE"
                } else if live > 0 {
                    "PARTIAL"
                } else {
                    "OFF"
                };
                column = column
                    .child(widgets::page_header(&theme, "Tools", None))
                    .child(widgets::page_subtitle(&theme, SUBTITLE))
                    .when_some(self.error.clone(), |el, error| {
                        el.child(
                            div()
                                .mt(px(12.0))
                                .child(widgets::error_strip(&theme, error)),
                        )
                    })
                    .child(self.hero(&theme, wires, live, total, cx));
                let mut groups = widgets::section_card(&theme).mt(px(6.0));
                for (index, info) in TOOL_GROUPS.iter().enumerate() {
                    groups = groups.child(self.group_card(
                        &theme,
                        info,
                        &settings,
                        &disabled,
                        index == 0,
                        cx,
                    ));
                }
                column = column.child(groups).child(
                    div()
                        .mt(px(12.0))
                        .px(px(4.0))
                        .flex()
                        .flex_wrap()
                        .justify_between()
                        .gap(px(8.0))
                        .text_size(crate::typography::ui_rems(widgets::ROW_DESCRIPTION_SIZE))
                        .text_color(theme.text_muted)
                        .child(
                            div()
                                .font_family(theme.font_mono.clone())
                                .text_size(crate::typography::ui_rems(10.5))
                                .child(SharedString::from(format!(
                                    "{groups_on} GROUPS · {live} / {total} TOOLS · {status}"
                                ))),
                        )
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
