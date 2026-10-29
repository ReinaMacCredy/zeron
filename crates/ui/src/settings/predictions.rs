use gpui::{AnyElement, Context, Entity, SharedString, Subscription, Task, div, prelude::*, px};
use zeron_engine::predictions::{PredictionSettingsState, PredictionSettingsUpdate};
use zeron_engine::registry::{CustomPredictionApi, PredictionSettings, TitleSettings};
use zeron_rpc::methods;

use crate::composer::ComposerInput;
use crate::pickers::{Pickers, TitleModelPicked};
use crate::popover::Loadable;
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;

pub struct PredictionsCard {
    state: Entity<AppState>,
    settings: Loadable<PredictionSettingsState>,
    picker: Entity<Pickers>,
    custom_mode: bool,
    base_url: Entity<ComposerInput>,
    api_model: Entity<ComposerInput>,
    error: Option<String>,
    task: Option<Task<()>>,
    _picked: Subscription,
    _state: Subscription,
}

impl PredictionsCard {
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
                    PredictionSettings {
                        harness: event.0.harness,
                        model: event.0.model.clone(),
                        custom: None,
                        ..current
                    },
                    None,
                    cx,
                );
            },
        );
        let state_sub = cx.observe(&state, |this: &mut Self, _, cx| {
            if matches!(this.settings, Loadable::Idle) {
                this.call(None, cx);
            }
        });
        let mut card = Self {
            state,
            settings: Loadable::Idle,
            picker,
            custom_mode: false,
            base_url: cx.new(|cx| ComposerInput::new("https://api.openai.com/v1", cx)),
            api_model: cx.new(|cx| ComposerInput::new("Model id", cx)),
            error: None,
            task: None,
            _picked: picked,
            _state: state_sub,
        };
        card.call(None, cx);
        card
    }

    fn current(&self) -> Option<PredictionSettings> {
        self.settings.ready().map(|state| state.settings.clone())
    }

    fn save(
        &mut self,
        settings: PredictionSettings,
        api_key: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.call(Some(PredictionSettingsUpdate { settings, api_key }), cx);
    }

    fn save_custom(&mut self, cx: &mut Context<Self>) {
        let Some(current) = self.current() else {
            return;
        };
        let custom = CustomPredictionApi {
            base_url: self.base_url.read(cx).text().to_string(),
            model: self.api_model.read(cx).text().to_string(),
        };
        self.save(
            PredictionSettings {
                custom: Some(custom),
                ..current
            },
            None,
            cx,
        );
    }

    fn paste_key(&mut self, cx: &mut Context<Self>) {
        let Some(current) = self.current() else {
            return;
        };
        let key = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty());
        match key {
            Some(key) => self.save(current, Some(key), cx),
            None => {
                self.error = Some("Copy the API key first, then press Paste key.".into());
                cx.notify();
            }
        }
    }

    fn call(&mut self, save: Option<PredictionSettingsUpdate>, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let (method, params) = match save {
            Some(update) => (
                methods::SET_PREDICTION_SETTINGS,
                serde_json::to_value(update).unwrap_or_default(),
            ),
            None => (methods::GET_PREDICTION_SETTINGS, serde_json::json!({})),
        };
        if matches!(self.settings, Loadable::Idle) {
            self.settings = Loadable::Loading;
        }
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(method, params)
                .await
                .map_err(|e| e.to_string())
                .and_then(|v| {
                    serde_json::from_value::<PredictionSettingsState>(v).map_err(|e| e.to_string())
                });
            this.update(cx, |card, cx| {
                match result {
                    Ok(state) => {
                        let settings = &state.settings;
                        card.picker.update(cx, |picker, cx| {
                            picker.set_title_settings(
                                TitleSettings {
                                    harness: settings.harness,
                                    model: settings.model.clone(),
                                },
                                cx,
                            )
                        });
                        if let Some(custom) = &settings.custom {
                            card.custom_mode = true;
                            let (base_url, model) = (custom.base_url.clone(), custom.model.clone());
                            card.base_url
                                .update(cx, |input, cx| input.set_text(base_url, cx));
                            card.api_model
                                .update(cx, |input, cx| input.set_text(model, cx));
                        }
                        card.settings = Loadable::Ready(state);
                        card.error = None;
                    }
                    Err(error) if matches!(card.settings, Loadable::Ready(_)) => {
                        card.error = Some(error);
                    }
                    Err(error) => card.settings = Loadable::Error(error),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn render_source_choice(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("composer-predictions-source")
            .flex_none()
            .flex()
            .flex_row()
            .rounded(px(9.0))
            .p(px(2.0))
            .bg(crate::theme::ink(0.04))
            .children(
                [(false, "Agent"), (true, "Custom API")]
                    .into_iter()
                    .enumerate()
                    .map(|(ix, (custom, label))| {
                        let selected = self.custom_mode == custom;
                        div()
                            .id(("composer-predictions-source-option", ix))
                            .min_w(px(72.0))
                            .px(px(12.0))
                            .py(px(6.0))
                            .rounded(px(7.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(12.0))
                            .text_color(if selected {
                                theme.text
                            } else {
                                theme.text_muted
                            })
                            .when(selected, |el| {
                                el.bg(theme.bg)
                                    .border_1()
                                    .border_color(theme.border.opacity(0.8))
                            })
                            .when(!selected, |el| {
                                el.cursor_pointer()
                                    .hover(|s| s.text_color(theme.text))
                                    .on_click(cx.listener(move |card, _, _, cx| {
                                        card.custom_mode = custom;
                                        card.error = None;
                                        if let Some(current) =
                                            card.current().filter(|c| !custom && c.custom.is_some())
                                        {
                                            card.save(
                                                PredictionSettings {
                                                    custom: None,
                                                    ..current
                                                },
                                                None,
                                                cx,
                                            );
                                        }
                                        cx.notify();
                                    }))
                            })
                            .child(SharedString::from(label))
                    }),
            )
            .into_any_element()
    }

    fn render_agent_row(
        &self,
        theme: &Theme,
        follows_session: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let description = if follows_session {
            "Each chat's own agent, on its cheapest model."
        } else {
            "Every chat uses this model."
        };
        widgets::card_row(theme, false)
            .child(
                div()
                    .flex_1()
                    .min_w(px(160.0))
                    .child(widgets::row_title(theme, "Agent model"))
                    .child(widgets::meta_line(
                        theme,
                        vec![div().child(description).into_any_element()],
                    )),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.0))
                    .when(!follows_session, |row| {
                        row.child(
                            widgets::text_action(theme, widgets::ActionTone::Quiet, "Reset")
                                .id("composer-predictions-reset")
                                .tab_index(0)
                                .role(gpui::Role::Button)
                                .aria_label("Predict with each chat's own agent")
                                .focus_visible(|s| s.border_2().border_color(theme.accent))
                                .on_click(cx.listener(|card, _, _, cx| {
                                    card.save(
                                        PredictionSettings {
                                            enabled: true,
                                            ..Default::default()
                                        },
                                        None,
                                        cx,
                                    )
                                })),
                        )
                    })
                    .child(self.picker.clone()),
            )
    }

    fn render_custom_rows(
        &self,
        theme: &Theme,
        has_key: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let field = |label: &'static str, input: &Entity<ComposerInput>| {
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(widgets::field_label(theme, label))
                .child(crate::popover::dialog_field(
                    input.clone().into_any_element(),
                ))
        };
        let key_status = if has_key {
            "Key saved in the Keychain."
        } else {
            "No key. Local servers like Ollama need none."
        };
        widgets::card_row(theme, false).child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(
                    div()
                        .flex()
                        .gap(px(12.0))
                        .child(div().flex_1().child(field("Base URL", &self.base_url)))
                        .child(div().w(px(200.0)).child(field("Model", &self.api_model))),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .flex_1()
                                .text_color(theme.text_muted)
                                .text_size(px(12.0))
                                .child(key_status),
                        )
                        .child(
                            widgets::text_action(theme, widgets::ActionTone::Quiet, "Paste key")
                                .id("composer-predictions-paste-key")
                                .tab_index(0)
                                .role(gpui::Role::Button)
                                .aria_label("Save the API key on the clipboard to the Keychain")
                                .focus_visible(|s| s.border_2().border_color(theme.accent))
                                .on_click(cx.listener(|card, _, _, cx| card.paste_key(cx))),
                        )
                        .when(has_key, |row| {
                            row.child(
                                widgets::text_action(
                                    theme,
                                    widgets::ActionTone::Quiet,
                                    "Remove key",
                                )
                                .id("composer-predictions-remove-key")
                                .tab_index(0)
                                .role(gpui::Role::Button)
                                .focus_visible(|s| s.border_2().border_color(theme.accent))
                                .on_click(cx.listener(
                                    |card, _, _, cx| {
                                        if let Some(current) = card.current() {
                                            card.save(current, Some(String::new()), cx);
                                        }
                                    },
                                )),
                            )
                        })
                        .child(
                            widgets::text_action(theme, widgets::ActionTone::Filled, "Save")
                                .id("composer-predictions-save-api")
                                .tab_index(0)
                                .role(gpui::Role::Button)
                                .focus_visible(|s| s.border_2().border_color(theme.accent))
                                .on_click(cx.listener(|card, _, _, cx| card.save_custom(cx))),
                        ),
                ),
        )
    }
}

impl Render for PredictionsCard {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_settings_surface();
        let state = self.settings.ready().cloned();
        let enabled = state.as_ref().is_some_and(|s| s.settings.enabled);
        let has_key = state.as_ref().is_some_and(|s| s.has_api_key);
        let follows_session = state.as_ref().is_some_and(|s| s.settings.harness.is_none());
        let switch: AnyElement = match &state {
            Some(current) => {
                let next = PredictionSettings {
                    enabled: !enabled,
                    ..current.settings.clone()
                };
                div()
                    .id("composer-predictions-switch")
                    .flex_none()
                    .tab_index(0)
                    .role(gpui::Role::Switch)
                    .aria_label("Show predictions")
                    .aria_toggled(if enabled {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .focus_visible(|s| s.border_2().border_color(theme.accent))
                    .cursor_pointer()
                    .on_click(cx.listener(move |card, _, _, cx| card.save(next.clone(), None, cx)))
                    .child(widgets::toggle_switch(
                        &theme,
                        enabled,
                        "composer-predictions",
                    ))
                    .into_any_element()
            }
            None => match &self.settings {
                Loadable::Error(error) => div()
                    .text_color(theme.text_muted)
                    .child(error.clone())
                    .into_any_element(),
                _ => div()
                    .text_color(theme.text_muted)
                    .child("Loading…")
                    .into_any_element(),
            },
        };
        let source_row = enabled.then(|| {
            widgets::card_row(&theme, false)
                .child(
                    div()
                        .flex_1()
                        .min_w(px(160.0))
                        .child(widgets::row_title(&theme, "Predict with"))
                        .child(widgets::meta_line(
                            &theme,
                            vec![
                                div()
                                    .child("A coding agent, or any OpenAI-compatible API.")
                                    .into_any_element(),
                            ],
                        )),
                )
                .child(self.render_source_choice(&theme, cx))
        });
        let detail_row = enabled.then(|| {
            if self.custom_mode {
                self.render_custom_rows(&theme, has_key, cx)
            } else {
                self.render_agent_row(&theme, follows_session, cx)
            }
        });
        widgets::section_card(&theme)
            .mt_0()
            .child(
                widgets::card_row(&theme, true)
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(160.0))
                            .child(widgets::row_title(&theme, "Show predictions"))
                            .child(widgets::meta_line(
                                &theme,
                                vec![
                                    div()
                                        .child(
                                            "Suggest your next message in the composer. Tab accepts it.",
                                        )
                                        .into_any_element(),
                                ],
                            )),
                    )
                    .child(switch),
            )
            .children(source_row)
            .children(detail_row)
            .when_some(self.error.clone(), |card, error| {
                card.child(
                    widgets::card_row(&theme, false).child(widgets::error_strip(&theme, error)),
                )
            })
    }
}
