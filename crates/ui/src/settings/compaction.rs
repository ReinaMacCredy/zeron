use gpui::{AnyElement, Context, Entity, SharedString, Subscription, Task, div, prelude::*, px};
use serde::de::DeserializeOwned;
use serde_json::Value;
use zeron_engine::jev::{CompactionSettingsState, CompactionSettingsUpdate};
use zeron_engine::registry::{CompactionMode, CompactionSettings, JevSource, TitleSettings};
use zeron_proto::tools::{ToolSettings, ToolSettingsState};
use zeron_rpc::methods;

use crate::pickers::{Pickers, TitleModelPicked};
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
const AHEAD_TITLE: &str = "Compact ahead of time";
const COMPACT_TOOL: &str = "compact";

pub struct CompactionPage {
    state: Entity<AppState>,
    settings: Loadable<CompactionSettingsState>,
    tools: Loadable<ToolSettingsState>,
    picker: Entity<Pickers>,
    error: Option<String>,
    scroll: widgets::PageScroll,
    tasks: Vec<Task<()>>,
    _picked: Subscription,
    _state: Subscription,
}

impl CompactionPage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let picker = {
            let state = state.clone();
            cx.new(|cx| Pickers::new_for_titles(state, TitleSettings::default(), cx))
        };
        let picked = cx.subscribe(
            &picker,
            |this: &mut Self, _, event: &TitleModelPicked, cx| {
                let Some(current) = this.current() else {
                    return;
                };
                this.save(
                    CompactionSettings {
                        classifier: event.0.clone(),
                        ..current
                    },
                    cx,
                );
            },
        );
        let state_sub = cx.observe(&state, |this: &mut Self, _, cx| {
            if matches!(this.settings, Loadable::Idle) || matches!(this.tools, Loadable::Idle) {
                this.load(cx);
            }
        });
        let mut card = Self {
            state,
            settings: Loadable::Idle,
            tools: Loadable::Idle,
            picker,
            error: None,
            scroll: widgets::PageScroll::default(),
            tasks: Vec::new(),
            _picked: picked,
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

    fn current(&self) -> Option<CompactionSettings> {
        self.settings.ready().map(|state| state.settings.clone())
    }

    fn save(&mut self, settings: CompactionSettings, cx: &mut Context<Self>) {
        self.update(
            CompactionSettingsUpdate {
                settings,
                jev_api_key: None,
            },
            cx,
        );
    }

    fn save_key(&mut self, key: String, cx: &mut Context<Self>) {
        let Some(settings) = self.current() else {
            return;
        };
        self.update(
            CompactionSettingsUpdate {
                settings,
                jev_api_key: Some(key),
            },
            cx,
        );
    }

    fn paste_key(&mut self, cx: &mut Context<Self>) {
        let key = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty());
        match key {
            Some(key) => self.save_key(key, cx),
            None => {
                self.error = Some("Copy the API key first, then press Paste key.".into());
                cx.notify();
            }
        }
    }

    fn update(&mut self, update: CompactionSettingsUpdate, cx: &mut Context<Self>) {
        let params = serde_json::to_value(update).unwrap_or_default();
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
                        if let Some(classifier) = card
                            .settings
                            .ready()
                            .map(|state| state.settings.classifier.clone())
                        {
                            card.picker
                                .update(cx, |picker, cx| picker.set_title_settings(classifier, cx));
                        }
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

fn key_row(
    theme: &Theme,
    source: JevSource,
    has_key: bool,
    cx: &mut Context<CompactionPage>,
) -> gpui::Div {
    let status = match (has_key, source) {
        (true, _) => "Key saved in the Keychain.",
        (false, JevSource::TypeSafe) => "No key. Get one at console.typesafe.ai/keys",
        (false, JevSource::OpenRouter) => "No key. Get one at openrouter.ai/settings/keys",
    };
    widgets::card_row(theme, false).child(
        div()
            .flex_1()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .child(
                div()
                    .flex_1()
                    .text_color(theme.text_muted)
                    .text_size(px(12.0))
                    .child(status),
            )
            .child(
                widgets::text_action(theme, widgets::ActionTone::Quiet, "Paste key")
                    .id("compaction-jev-paste-key")
                    .tab_index(0)
                    .role(gpui::Role::Button)
                    .aria_label("Save the API key on the clipboard to the Keychain")
                    .focus_visible(|s| s.border_2().border_color(theme.accent))
                    .on_click(cx.listener(|card, _, _, cx| card.paste_key(cx))),
            )
            .when(has_key, |row| {
                row.child(
                    widgets::text_action(theme, widgets::ActionTone::Quiet, "Remove key")
                        .id("compaction-jev-remove-key")
                        .tab_index(0)
                        .role(gpui::Role::Button)
                        .focus_visible(|s| s.border_2().border_color(theme.accent))
                        .on_click(cx.listener(|card, _, _, cx| card.save_key(String::new(), cx))),
                )
            }),
    )
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
        let (Some(state), Some(tools)) = (self.settings.ready().cloned(), self.tools.ready())
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
                    .search_anchor(Target::Page(SettingsSection::Compaction))
                    .into_any_element(),
            ];
        };
        let CompactionSettingsState {
            settings,
            has_jev_key,
        } = state;
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
        let pick_mode = {
            let entity = entity.clone();
            let settings = settings.clone();
            move |mode: CompactionMode, _: &mut gpui::Window, cx: &mut gpui::App| {
                entity.update(cx, |page, cx| {
                    page.save(
                        CompactionSettings {
                            mode,
                            ..settings.clone()
                        },
                        cx,
                    );
                });
            }
        };
        let pick_source = {
            let entity = entity.clone();
            let settings = settings.clone();
            move |jev: Option<JevSource>, _: &mut gpui::Window, cx: &mut gpui::App| {
                entity.update(cx, |page, cx| {
                    page.save(
                        CompactionSettings {
                            jev,
                            ..settings.clone()
                        },
                        cx,
                    );
                });
            }
        };
        let classifier = settings.mode == CompactionMode::Classifier;
        let toggled = CompactionSettings {
            enabled: !settings.enabled,
            ..settings.clone()
        };
        let missions_toggled = CompactionSettings {
            missions: !settings.missions,
            ..settings.clone()
        };
        let ahead_toggled = CompactionSettings {
            compact_ahead: !settings.compact_ahead,
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
        // The Jev key lives in the macOS Keychain; elsewhere it could never be stored.
        let source_row = (classifier && cfg!(target_os = "macos")).then(|| {
            widgets::card_row(theme, false)
                .child(row_text(
                    theme,
                    "Classifier",
                    "An agent's model, or TypeSafe's Jev classifier with your API key. Jev receives the recent conversation and each new message past the check.",
                ))
                .child(widgets::segmented(
                    theme,
                    "compaction-classifier-source",
                    &[
                        (None, "Agent model"),
                        (Some(JevSource::TypeSafe), "Jev · TypeSafe"),
                        (Some(JevSource::OpenRouter), "Jev · OpenRouter"),
                    ],
                    settings.jev,
                    pick_source,
                ))
                .search_anchor(SettingId::CompactionClassifier)
        });
        let detail_row = classifier.then(|| match settings.jev {
            None => widgets::card_row(theme, false)
                .child(row_text(
                    theme,
                    "Classifier model",
                    "Sonnet when Claude classifies, Codex's default model when Codex does; a small model compacted follow-ups in testing.",
                ))
                .child(self.picker.clone())
                .search_anchor(SettingId::CompactionClassifierModel),
            Some(source) => key_row(theme, source, has_jev_key, cx)
                .search_anchor(SettingId::CompactionClassifierModel),
        });
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
                        "Compact new tasks",
                        if classifier {
                            "A model reads each message and compacts first when it starts a new task."
                        } else {
                            "Each message carries a reminder; the agent decides whether to compact."
                        },
                    ))
                    .child(widgets::segmented(
                        theme,
                        "compaction-mode",
                        &[
                            (CompactionMode::Threshold, "Reminder"),
                            (CompactionMode::Classifier, "Classifier"),
                        ],
                        settings.mode,
                        pick_mode,
                    ))
                    .search_anchor(SettingId::CompactionMode),
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
                        if classifier { "Check from" } else { "Remind at" },
                        if classifier {
                            "Context a compaction could free, above what the last one left; a chat with a smaller window is checked from 70% of its free room."
                        } else {
                            "Context a compaction could free, above what the last one left; a chat with a smaller window is reminded at 70% of its free room. Every message past it carries the reminder until the chat compacts."
                        },
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
            .children(source_row)
            .children(detail_row)
            .when_some(self.error.clone(), |card, error| {
                card.child(widgets::card_row(theme, false).child(widgets::error_strip(theme, error)))
            });
        let ahead_card = widgets::section_card(theme).mt_0().child(
            widgets::card_row(theme, true)
                .child(row_text_with(
                    theme,
                    widgets::row_title(theme, AHEAD_TITLE)
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .child(widgets::beta_badge(theme)),
                    "Claude compacts in place right after a turn. Codex summarizes a copy in the background and swaps it in at the next pause; turns after the copy come back as text.",
                ))
                .child(
                    switch(theme, settings.compact_ahead, "compact-ahead", AHEAD_TITLE).on_click(
                        cx.listener(move |this, _, _, cx| this.save(ahead_toggled.clone(), cx)),
                    ),
                )
                .search_anchor(SettingId::CompactAhead),
        );
        vec![
            widgets::section(theme, "When idle", idle_card)
                .mt_0()
                .into_any_element(),
            widgets::section(theme, "During a chat", agents_card)
                .search_anchor(Target::Page(SettingsSection::Compaction))
                .into_any_element(),
            widgets::section(theme, "Ahead of time", ahead_card).into_any_element(),
        ]
    }
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
