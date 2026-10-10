use gpui::{AnyElement, Context, Entity, Subscription, Task, div, prelude::*, px};
use zeron_engine::registry::{PredictionSettings, TitleSettings};
use zeron_rpc::methods;

use crate::pickers::{Pickers, TitleModelPicked};
use crate::popover::Loadable;
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;

pub struct PredictionsCard {
    state: Entity<AppState>,
    settings: Loadable<PredictionSettings>,
    picker: Entity<Pickers>,
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
                let Some(current) = this.settings.ready().cloned() else {
                    return;
                };
                this.call(
                    Some(PredictionSettings {
                        harness: event.0.harness,
                        model: event.0.model.clone(),
                        ..current
                    }),
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
            error: None,
            task: None,
            _picked: picked,
            _state: state_sub,
        };
        card.call(None, cx);
        card
    }

    fn call(&mut self, save: Option<PredictionSettings>, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let (method, params) = match save {
            Some(settings) => (
                methods::SET_PREDICTION_SETTINGS,
                serde_json::to_value(settings).unwrap_or_default(),
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
                    serde_json::from_value::<PredictionSettings>(v).map_err(|e| e.to_string())
                });
            this.update(cx, |card, cx| {
                match result {
                    Ok(settings) => {
                        card.picker.update(cx, |picker, cx| {
                            picker.set_title_settings(
                                TitleSettings {
                                    harness: settings.harness,
                                    model: settings.model.clone(),
                                },
                                cx,
                            )
                        });
                        card.settings = Loadable::Ready(settings);
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
}

impl Render for PredictionsCard {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_settings_surface();
        let settings = self.settings.ready().cloned();
        let enabled = settings.as_ref().is_some_and(|s| s.enabled);
        let follows_session = settings.as_ref().is_some_and(|s| s.harness.is_none());
        let switch: AnyElement = match &settings {
            Some(current) => {
                let next = PredictionSettings {
                    enabled: !enabled,
                    ..current.clone()
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
                    .on_click(cx.listener(move |card, _, _, cx| card.call(Some(next.clone()), cx)))
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
        let model_row = enabled.then(|| {
            let description = if follows_session {
                "Each chat's own agent, on its cheapest model."
            } else {
                "Every chat uses this model."
            };
            widgets::card_row(&theme, false)
                .child(
                    div()
                        .flex_1()
                        .min_w(px(160.0))
                        .child(widgets::row_title(&theme, "Prediction model"))
                        .child(widgets::meta_line(
                            &theme,
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
                                widgets::text_action(&theme, widgets::ActionTone::Quiet, "Reset")
                                    .id("composer-predictions-reset")
                                    .tab_index(0)
                                    .role(gpui::Role::Button)
                                    .aria_label("Predict with each chat's own agent")
                                    .focus_visible(|s| s.border_2().border_color(theme.accent))
                                    .on_click(cx.listener(|card, _, _, cx| {
                                        card.call(
                                            Some(PredictionSettings {
                                                enabled: true,
                                                ..Default::default()
                                            }),
                                            cx,
                                        )
                                    })),
                            )
                        })
                        .child(self.picker.clone()),
                )
        });
        widgets::section_card(&theme)
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
            .children(model_row)
            .when_some(self.error.clone(), |card, error| {
                card.child(
                    widgets::card_row(&theme, false).child(widgets::error_strip(&theme, error)),
                )
            })
    }
}
