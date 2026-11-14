use gpui::{AnyElement, Context, Entity, SharedString, Subscription, Task, div, prelude::*, px};
use serde::de::DeserializeOwned;
use serde_json::Value;
use zeron_engine::registry::CompactionSettings;
use zeron_proto::tools::{ToolGroup, ToolSettings, ToolSettingsState};
use zeron_rpc::methods;

use crate::popover;
use crate::popover::Loadable;
use crate::settings::search::{SearchAnchor, SettingId, Target};
use crate::settings::widgets;
use crate::shell::SettingsSection;
use crate::state::AppState;
use crate::theme::Theme;

const IDLE_TITLE: &str = "Compact idle chats before the cache expires";
const AGENT_TITLE: &str = "Let agents compact themselves";
const MISSIONS_TITLE: &str = "In tasks";
const COMPACT_TOOL: &str = "compact";

pub struct CompactionPage {
    state: Entity<AppState>,
    settings: Loadable<CompactionSettings>,
    tools: Loadable<ToolSettingsState>,
    error: Option<String>,
    scroll: widgets::PageScroll,
    /// Lab > Experimental rather than Agents > Compaction.
    lab: bool,
    tasks: Vec<Task<()>>,
    _state: Subscription,
}

impl CompactionPage {
    pub fn new(state: Entity<AppState>, lab: bool, cx: &mut Context<Self>) -> Self {
        let state_sub = cx.observe(&state, |this: &mut Self, _, cx| {
            if matches!(this.settings, Loadable::Idle) || matches!(this.tools, Loadable::Idle) {
                this.load(cx);
            }
        });
        let mut card = Self {
            state,
            settings: Loadable::Idle,
            tools: Loadable::Idle,
            error: None,
            scroll: widgets::PageScroll::default(),
            lab,
            tasks: Vec::new(),
            _state: state_sub,
        };
        card.load(cx);
        card
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        self.call(
            methods::GET_COMPACTION_SETTINGS,
            Value::Null,
            |card| &mut card.settings,
            cx,
        );
        self.call(
            methods::GET_TOOL_SETTINGS,
            Value::Null,
            |card| &mut card.tools,
            cx,
        );
    }

    fn on_scroll_hovered(&mut self, hovered: &bool, _: &mut gpui::Window, cx: &mut Context<Self>) {
        if self.scroll.set_list_hovered(*hovered) {
            cx.notify();
        }
    }

    fn save(&mut self, settings: CompactionSettings, cx: &mut Context<Self>) {
        let params = serde_json::to_value(settings).unwrap_or_default();
        self.call(
            methods::SET_COMPACTION_SETTINGS,
            params,
            |card| &mut card.settings,
            cx,
        );
    }

    fn save_tool(&mut self, on: bool, cx: &mut Context<Self>) {
        let Some(mut tools) = self.tools.ready().map(|state| state.settings.clone()) else {
            return;
        };
        tools.set_tool(COMPACT_TOOL, on);
        let params = serde_json::to_value::<ToolSettings>(tools).unwrap_or_default();
        self.call(
            methods::SET_TOOL_SETTINGS,
            params,
            |card| &mut card.tools,
            cx,
        );
    }

    fn save_tools(&mut self, change: impl FnOnce(&mut ToolSettings), cx: &mut Context<Self>) {
        let Some(mut tools) = self.tools.ready().map(|state| state.settings.clone()) else {
            return;
        };
        change(&mut tools);
        let params = serde_json::to_value::<ToolSettings>(tools).unwrap_or_default();
        self.call(
            methods::SET_TOOL_SETTINGS,
            params,
            |card| &mut card.tools,
            cx,
        );
    }

    fn call<T: DeserializeOwned + 'static>(
        &mut self,
        method: &'static str,
        params: Value,
        slot: fn(&mut Self) -> &mut Loadable<T>,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        if matches!(slot(self), Loadable::Idle) {
            *slot(self) = Loadable::Loading;
        }
        self.tasks.retain(|task| !task.is_ready());
        self.tasks.push(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(method, params)
                .await
                .map_err(|e| e.to_string())
                .and_then(|v| serde_json::from_value::<T>(v).map_err(|e| e.to_string()));
            this.update(cx, |card, cx| {
                match result {
                    Ok(value) => {
                        *slot(card) = Loadable::Ready(value);
                        card.error = None;
                    }
                    Err(error) if matches!(slot(card), Loadable::Ready(_)) => {
                        card.error = Some(error);
                    }
                    Err(error) => *slot(card) = Loadable::Error(error),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }
}

fn row_text(theme: &Theme, title: &'static str, meta: &'static str) -> gpui::Div {
    row_text_with(theme, widgets::row_title(theme, title), meta)
}

fn row_text_with(theme: &Theme, title: gpui::Div, meta: &'static str) -> gpui::Div {
    div()
        .flex_1()
        .min_w(px(160.0))
        .child(title)
        .child(widgets::meta_line(
            theme,
            vec![div().child(meta).into_any_element()],
        ))
}

fn switch(
    theme: &Theme,
    on: bool,
    key: &'static str,
    title: &'static str,
) -> gpui::Stateful<gpui::Div> {
    widgets::toggle_switch(theme, on, key)
        .id(key)
        .cursor_pointer()
        .tab_index(0)
        .role(gpui::Role::Switch)
        .aria_label(title)
        .aria_toggled(if on {
            gpui::Toggled::True
        } else {
            gpui::Toggled::False
        })
}

impl popover::ScrollRailHost for CompactionPage {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        self.scroll.rail_bar()
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.scroll.rail_scroll()
    }
}

impl CompactionPage {
    fn render_sections(&mut self, theme: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let (Some(settings), Some(tools)) = (self.settings.ready().cloned(), self.tools.ready())
        else {
            let note = match (&self.settings, &self.tools) {
                (Loadable::Error(error), _) | (_, Loadable::Error(error)) => error.clone(),
                _ => "Loading…".to_string(),
            };
            let card = widgets::section_card(theme).mt_0().child(
                widgets::card_row(theme, true).child(
                    div()
                        .text_color(theme.text_muted)
                        .child(SharedString::from(note)),
                ),
            );
            return vec![
                widgets::section(theme, "Compaction", card)
                    .mt_0()
                    .into_any_element(),
            ];
        };
        let agent_on = !tools.disabled.iter().any(|tool| tool == COMPACT_TOOL);
        let entity = cx.entity();
        let pick = |mutate: fn(&mut CompactionSettings, u64)| {
            let entity = entity.clone();
            let settings = settings.clone();
            move |value: u64, _: &mut gpui::Window, cx: &mut gpui::App| {
                entity.update(cx, |page, cx| {
                    let mut next = settings.clone();
                    mutate(&mut next, value);
                    page.save(next, cx);
                });
            }
        };
        let toggled = CompactionSettings {
            enabled: !settings.enabled,
            ..settings.clone()
        };
        let missions_toggled = CompactionSettings {
            missions: !settings.missions,
            ..settings.clone()
        };
        let idle_card = widgets::section_card(theme)
            .mt_0()
            .child(
                widgets::card_row(theme, true)
                    .child(row_text(
                        theme,
                        IDLE_TITLE,
                        "Summarize a long idle conversation just before its 1-hour prompt cache expires, so coming back reprocesses the summary instead of the whole chat.",
                    ))
                    .child(
                        switch(theme, settings.enabled, "idle-compaction", IDLE_TITLE).on_click(
                            cx.listener(move |this, _, _, cx| this.save(toggled.clone(), cx)),
                        ),
                    )
                    .search_anchor(SettingId::IdleCompaction),
            )
            .child(
                widgets::card_row(theme, false)
                    .child(row_text(
                        theme,
                        "Idle threshold",
                        "Smaller chats are left alone.",
                    ))
                    .child(widgets::segmented(
                        theme,
                        "idle-compaction-threshold",
                        &[(100_000, "100K"), (150_000, "150K"), (200_000, "200K")],
                        settings.min_tokens,
                        pick(|next, value| next.min_tokens = value),
                    ))
                    .search_anchor(SettingId::IdleThreshold),
            );
        let agents_card = widgets::section_card(theme)
            .mt_0()
            .child(
                widgets::card_row(theme, true)
                    .child(row_text(
                        theme,
                        AGENT_TITLE,
                        "Agents get the compact tool, and a reminder once their context fills up.",
                    ))
                    .child(switch(theme, agent_on, "agent-compaction", AGENT_TITLE).on_click(
                        cx.listener(move |this, _, _, cx| this.save_tool(!agent_on, cx)),
                    ))
                    .search_anchor(SettingId::AgentCompaction),
            )
            .child(
                widgets::card_row(theme, false)
                    .child(row_text(
                        theme,
                        MISSIONS_TITLE,
                        "Lead and step threads compact by the same rule. Off leaves tasks alone.",
                    ))
                    .child(
                        switch(
                            theme,
                            settings.missions,
                            "mission-compaction",
                            MISSIONS_TITLE,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.save(missions_toggled.clone(), cx)
                        })),
                    )
                    .search_anchor(SettingId::TaskCompaction),
            )
            .child(
                widgets::card_row(theme, false)
                    .child(row_text(
                        theme,
                        "Remind at",
                        "Context a compaction could free, above what the last one left; a chat with a smaller window is reminded at 70% of its free room. Every message past it carries the reminder until the chat compacts.",
                    ))
                    .child(widgets::segmented(
                        theme,
                        "agent-compaction-nudge",
                        &[(0, "Off"), (100_000, "100K"), (150_000, "150K"), (200_000, "200K")],
                        settings.nudge_tokens,
                        pick(|next, value| next.nudge_tokens = value),
                    ))
                    .search_anchor(SettingId::CompactionNudge),
            )
            .when_some(self.error.clone().filter(|_| !self.lab), |card, error| {
                card.child(widgets::card_row(theme, false).child(widgets::error_strip(theme, error)))
            });
        if self.lab {
            let full_token = self
                .tools
                .ready()
                .is_some_and(|state| state.settings.agents_use_full_token);
            let token_card = widgets::section_card(theme)
                .mt_0()
                .child(
                    widgets::card_row(theme, false)
                        .child(row_text(
                            theme,
                            "Agents use the full token",
                            "New runs reach everything the app can, including allowing plugins and signing in, with no question to you. Only turn this on if a tool you need is refused.",
                        ))
                        .child(
                            switch(theme, full_token, "agents-full-token", "Agents use the full token")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.save_tools(|tools| tools.agents_use_full_token = !full_token, cx)
                                })),
                        ),
                )
                .when_some(self.error.clone(), |card, error| {
                    card.child(
                        widgets::card_row(theme, false).child(widgets::error_strip(theme, error)),
                    )
                });
            let app_api = self
                .tools
                .ready()
                .is_none_or(|state| state.settings.group_on(ToolGroup::UiDriver));
            let app_api_card = widgets::section_card(theme).mt_0().child(
                widgets::card_row(theme, false)
                    .child(row_text(
                        theme,
                        "App API",
                        "Lets agents read and drive Zeron's windows, including text you are typing, with app_inspect and app_drive. The zeron app command in a shell is not affected.",
                    ))
                    .child(
                        switch(theme, app_api, "app-api", "App API").on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.save_tools(
                                    |tools| tools.set_group(ToolGroup::UiDriver, !app_api),
                                    cx,
                                )
                            },
                        )),
                    ),
            );
            let beta = zeron_beta_app();
            let beta_card = widgets::section_card(theme).mt_0().child(
                widgets::card_row(theme, false)
                    .child(row_text(
                        theme,
                        "Zeron Beta",
                        if beta.is_some() {
                            "The new interface, beside this app on the same engine and chats. It never restarts or updates the engine."
                        } else {
                            "The new interface, beside this app on the same engine and chats. Not installed: scripts/package-zgx-beta.sh builds it, for /Applications."
                        },
                    ))
                    .when_some(beta, |row, app| {
                        row.child(
                            widgets::text_action(theme, widgets::ActionTone::Quiet, "Open")
                                .id("open-zeron-beta")
                                .tab_index(0)
                                .role(gpui::Role::Button)
                                .aria_label("Open Zeron Beta")
                                .focus_visible(|s| s.border_2().border_color(theme.accent))
                                .on_click(move |_, _, _| {
                                    let mut open = std::process::Command::new("open");
                                    // `open` drops the environment, and a Dev bundle's Beta must not reach the real engine.
                                    for key in ["ZERON_DATA_DIR", "ZERON_IPC_PORT"] {
                                        if let Ok(value) = std::env::var(key) {
                                            open.arg("--env").arg(format!("{key}={value}"));
                                        }
                                    }
                                    if let Err(error) = open.arg(&app).spawn() {
                                        tracing::warn!(%error, "could not open Zeron Beta");
                                    }
                                }),
                        )
                    }),
            );
            return vec![
                widgets::section(
                    theme,
                    "Agent access",
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(token_card)
                        .child(app_api_card),
                )
                .mt_0()
                .search_anchor(Target::Page(SettingsSection::Experimental))
                .into_any_element(),
                widgets::section(theme, "New interface", beta_card).into_any_element(),
            ];
        }
        vec![
            widgets::section(theme, "When idle", idle_card)
                .mt_0()
                .into_any_element(),
            widgets::section(theme, "During a chat", agents_card)
                .search_anchor(Target::Page(SettingsSection::Compaction))
                .into_any_element(),
        ]
    }
}

fn zeron_beta_app() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from)?;
    [
        std::path::PathBuf::from("/Applications"),
        home.join("Applications"),
    ]
    .into_iter()
    .map(|dir| dir.join("Zeron Beta.app"))
    .find(|app| app.exists())
}

impl Render for CompactionPage {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_settings_surface();
        let sections = self.render_sections(&theme, cx);
        let scrollbar = popover::rail(self, "compaction-settings-scrollbar", &theme, cx);
        div()
            .id("compaction-settings-host")
            .relative()
            .size_full()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .child(
                crate::edge_fade::edge_faded(
                    16.0,
                    true,
                    true,
                    div()
                        .id("compaction-settings-page")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll.scroll)
                        .child(widgets::settings_page().children(sections)),
                )
                .fade_overflow_y(&self.scroll.scroll),
            )
            .child(widgets::page_reveal(&self.scroll.scroll))
            .children(scrollbar)
    }
}
