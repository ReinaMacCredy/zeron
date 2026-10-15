use gpui::{Context, Entity, SharedString, Subscription, Task, div, prelude::*, px};
use zeron_engine::registry::IdleCompactionSettings;
use zeron_rpc::methods;

use crate::popover::Loadable;
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;

const TITLE: &str = "Compact idle Claude chats";

pub struct IdleCompactionCard {
    state: Entity<AppState>,
    settings: Loadable<IdleCompactionSettings>,
    error: Option<String>,
    task: Option<Task<()>>,
    _state: Subscription,
}

impl IdleCompactionCard {
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

    fn call(&mut self, save: Option<IdleCompactionSettings>, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let (method, params) = match save {
            Some(settings) => (
                methods::SET_IDLE_COMPACTION_SETTINGS,
                serde_json::to_value(settings).unwrap_or_default(),
            ),
            None => (methods::GET_IDLE_COMPACTION_SETTINGS, serde_json::json!({})),
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
                    serde_json::from_value::<IdleCompactionSettings>(v).map_err(|e| e.to_string())
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

impl Render for IdleCompactionCard {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_settings_surface();
        let card = widgets::section_card(&theme);
        let Some(settings) = self.settings.ready().copied() else {
            let note = match &self.settings {
                Loadable::Error(error) => error.clone(),
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
        let enabled = settings.enabled;
        card.child(
            widgets::card_row(&theme, true)
                .child(
                    div()
                        .flex_1()
                        .min_w(px(160.0))
                        .child(widgets::row_title(&theme, TITLE))
                        .child(widgets::meta_line(
                            &theme,
                            vec![
                                div()
                                    .child(
                                        "Summarize a long idle conversation just before its 1-hour prompt cache expires, so coming back reprocesses the summary instead of the whole chat.",
                                    )
                                    .into_any_element(),
                            ],
                        )),
                )
                .child(
                    widgets::toggle_switch(&theme, enabled, "idle-compaction-switch")
                        .id("idle-compaction")
                        .cursor_pointer()
                        .tab_index(0)
                        .role(gpui::Role::Switch)
                        .aria_label(TITLE)
                        .aria_toggled(if enabled {
                            gpui::Toggled::True
                        } else {
                            gpui::Toggled::False
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.call(Some(IdleCompactionSettings { enabled: !enabled }), cx)
                        })),
                ),
        )
        .when_some(self.error.clone(), |card, error| {
            card.child(widgets::card_row(&theme, false).child(widgets::error_strip(&theme, error)))
        })
    }
}
