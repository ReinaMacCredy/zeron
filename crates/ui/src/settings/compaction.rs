use gpui::{AnyElement, Context, Entity, SharedString, Subscription, Task, div, prelude::*, px};
use serde::de::DeserializeOwned;
use serde_json::Value;
use zeron_engine::registry::CompactionSettings;
use zeron_proto::tools::{ToolSettings, ToolSettingsState};
use zeron_rpc::methods;

use crate::popover::Loadable;
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;

const IDLE_TITLE: &str = "Compact idle chats before the cache expires";
const AGENT_TITLE: &str = "Let agents compact themselves";
const COMPACT_TOOL: &str = "compact";

pub struct CompactionCard {
    state: Entity<AppState>,
    settings: Loadable<CompactionSettings>,
    tools: Loadable<ToolSettingsState>,
    error: Option<String>,
    tasks: Vec<Task<()>>,
    _state: Subscription,
}

impl CompactionCard {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
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
    div()
        .flex_1()
        .min_w(px(160.0))
        .child(widgets::row_title(theme, title))
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

impl Render for CompactionCard {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_settings_surface();
        let card = widgets::section_card(&theme);
        let (Some(settings), Some(tools)) = (self.settings.ready().copied(), self.tools.ready())
        else {
            let note = match (&self.settings, &self.tools) {
                (Loadable::Error(error), _) | (_, Loadable::Error(error)) => error.clone(),
                _ => "Loading…".to_string(),
            };
            return card.child(
                widgets::card_row(&theme, true).child(
                    div()
                        .text_color(theme.text_muted)
                        .child(SharedString::from(note)),
                ),
            );
        };
        let agent_on = !tools.disabled.iter().any(|tool| tool == COMPACT_TOOL);
        let entity = cx.entity();
        let pick = move |mutate: fn(&mut CompactionSettings, u64)| {
            let entity = entity.clone();
            move |value: u64, _: &mut gpui::Window, cx: &mut gpui::App| {
                entity.update(cx, |card, cx| {
                    let mut next = settings;
                    mutate(&mut next, value);
                    card.save(next, cx);
                });
            }
        };
        let rows: Vec<AnyElement> = vec![
            widgets::card_row(&theme, true)
                .child(row_text(
                    &theme,
                    IDLE_TITLE,
                    "Summarize a long idle conversation just before its 1-hour prompt cache expires, so coming back reprocesses the summary instead of the whole chat.",
                ))
                .child(
                    switch(&theme, settings.enabled, "idle-compaction", IDLE_TITLE).on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.save(
                                CompactionSettings {
                                    enabled: !settings.enabled,
                                    ..settings
                                },
                                cx,
                            )
                        }),
                    ),
                )
                .into_any_element(),
            widgets::card_row(&theme, false)
                .child(row_text(
                    &theme,
                    "Idle threshold",
                    "Smaller chats are left alone.",
                ))
                .child(widgets::segmented(
                    &theme,
                    "idle-compaction-threshold",
                    &[(100_000, "100K"), (150_000, "150K"), (200_000, "200K")],
                    settings.min_tokens,
                    pick(|next, value| next.min_tokens = value),
                ))
                .into_any_element(),
            widgets::card_row(&theme, false)
                .child(row_text(
                    &theme,
                    AGENT_TITLE,
                    "Agents get the compact tool, and a reminder once their context fills up.",
                ))
                .child(switch(&theme, agent_on, "agent-compaction", AGENT_TITLE).on_click(
                    cx.listener(move |this, _, _, cx| this.save_tool(!agent_on, cx)),
                ))
                .into_any_element(),
            widgets::card_row(&theme, false)
                .child(row_text(
                    &theme,
                    "Remind at",
                    "Context used, as the footer ring shows it. A second reminder follows at 85%.",
                ))
                .child(widgets::segmented(
                    &theme,
                    "agent-compaction-nudge",
                    &[(0, "Off"), (60, "60%"), (70, "70%"), (85, "85%")],
                    u64::from(settings.nudge_percent),
                    pick(|next, value| next.nudge_percent = value as u8),
                ))
                .into_any_element(),
        ];
        card.children(rows)
            .when_some(self.error.clone(), |card, error| {
                card.child(
                    widgets::card_row(&theme, false).child(widgets::error_strip(&theme, error)),
                )
            })
    }
}
