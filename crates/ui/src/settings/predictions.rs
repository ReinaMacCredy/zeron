use gpui::{AnyElement, Context, Entity, Subscription, Task, div, prelude::*, px};
use zeron_engine::registry::PredictionSettings;
use zeron_rpc::methods;

use crate::popover::Loadable;
use crate::settings::search::{SearchAnchor, SettingId};
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;

pub struct PredictionsCard {
    state: Entity<AppState>,
    settings: Loadable<PredictionSettings>,
    error: Option<String>,
    task: Option<Task<()>>,
    _state: Subscription,
}

impl PredictionsCard {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let state_sub = cx.observe(&state, |this: &mut Self, _, cx| {
            if matches!(this.settings, Loadable::Idle) {
                this.call(None, cx);
            }
        });
        let mut card = Self {
            state,
            settings: Loadable::Idle,
            error: None,
            task: None,
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
        let enabled = self.settings.ready().is_some_and(|s| s.enabled);
        let switch: AnyElement = match &self.settings {
            Loadable::Ready(_) => div()
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
                .on_click(cx.listener(move |card, _, _, cx| {
                    card.call(Some(PredictionSettings { enabled: !enabled }), cx)
                }))
                .child(widgets::toggle_switch(
                    &theme,
                    enabled,
                    "composer-predictions",
                ))
                .into_any_element(),
            Loadable::Error(error) => div()
                .text_color(theme.text_muted)
                .child(error.clone())
                .into_any_element(),
            _ => div()
                .text_color(theme.text_muted)
                .child("Loading…")
                .into_any_element(),
        };
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
                                            "Suggest your next message after Claude and Codex turns. Tab accepts it. Each is one extra request that reuses the turn's cache.",
                                        )
                                        .into_any_element(),
                                ],
                            )),
                    )
                    .child(switch)
                    .search_anchor(SettingId::ShowPredictions),
            )
            .when_some(self.error.clone(), |card, error| {
                card.child(
                    widgets::card_row(&theme, false).child(widgets::error_strip(&theme, error)),
                )
            })
    }
}
