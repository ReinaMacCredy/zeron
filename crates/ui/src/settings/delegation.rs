use gpui::{AnyElement, Context, Entity, SharedString, Subscription, Task, div, prelude::*, px};
use zeron_engine::registry::DelegationSettings;
use zeron_rpc::methods;

use crate::popover::Loadable;
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;

const DEPTH_MAX: u32 = 5;
const CHILDREN_MAX: u32 = 32;

pub struct DelegationCard {
    state: Entity<AppState>,
    settings: Loadable<DelegationSettings>,
    error: Option<String>,
    task: Option<Task<()>>,
    _state: Subscription,
}

impl DelegationCard {
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

    fn call(&mut self, save: Option<DelegationSettings>, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let (method, params) = match save {
            Some(settings) => (
                methods::SET_DELEGATION_SETTINGS,
                serde_json::to_value(settings).unwrap_or_default(),
            ),
            None => (methods::GET_DELEGATION_SETTINGS, serde_json::json!({})),
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
                    serde_json::from_value::<DelegationSettings>(v).map_err(|e| e.to_string())
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

    fn stepper(
        &self,
        theme: &Theme,
        id: &'static str,
        label: &'static str,
        value: u32,
        max: u32,
        apply: fn(DelegationSettings, u32) -> DelegationSettings,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let settings = self.settings.ready().copied().unwrap_or_default();
        let step = |glyph: &'static str, next: Option<u32>, key: &'static str| {
            let enabled = next.is_some();
            div()
                .id(SharedString::from(format!("{id}-{key}")))
                .tab_index(0)
                .role(gpui::Role::Button)
                .aria_label(SharedString::from(format!("{label} {key}")))
                .size(px(26.0))
                .rounded(px(7.0))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(14.0))
                .text_color(if enabled {
                    theme.text_muted
                } else {
                    theme.text_muted.opacity(0.35)
                })
                .when_some(next, |el, next| {
                    let text = theme.text;
                    let bg = theme.bg;
                    el.cursor_pointer()
                        .hover(move |s| s.bg(bg).text_color(text))
                        .on_click(cx.listener(move |card, _, _, cx| {
                            card.call(Some(apply(settings, next)), cx)
                        }))
                })
                .child(SharedString::from(glyph))
        };
        div()
            .id(id)
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .rounded(px(9.0))
            .p(px(2.0))
            .bg(crate::theme::ink(0.04))
            .child(step("−", (value > 1).then(|| value - 1), "down"))
            .child(
                div()
                    .min_w(px(36.0))
                    .px(px(6.0))
                    .flex()
                    .justify_center()
                    .text_size(px(12.0))
                    .text_color(theme.text)
                    .child(SharedString::from(value.to_string())),
            )
            .child(step("+", (value < max).then(|| value + 1), "up"))
            .into_any_element()
    }

    fn row(
        theme: &Theme,
        first: bool,
        title: &'static str,
        description: &'static str,
        control: AnyElement,
    ) -> gpui::Div {
        widgets::card_row(theme, first)
            .child(
                div()
                    .flex_1()
                    .min_w(px(160.0))
                    .child(widgets::row_title(theme, title))
                    .child(widgets::meta_line(
                        theme,
                        vec![div().child(description).into_any_element()],
                    )),
            )
            .child(control)
    }
}

impl Render for DelegationCard {
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
        let depth = self.stepper(
            &theme,
            "delegation-max-depth",
            "Delegation depth",
            settings.max_depth,
            DEPTH_MAX,
            |s, max_depth| DelegationSettings { max_depth, ..s },
            cx,
        );
        let children = self.stepper(
            &theme,
            "delegation-max-children",
            "Delegated tasks per chat",
            settings.max_running_children,
            CHILDREN_MAX,
            |s, max_running_children| DelegationSettings {
                max_running_children,
                ..s
            },
            cx,
        );
        card.child(Self::row(
            &theme,
            true,
            "Delegation depth",
            "How many levels of delegated tasks an agent may nest.",
            depth,
        ))
        .child(Self::row(
            &theme,
            false,
            "Delegated tasks per chat",
            "How many delegated tasks one chat may run at once.",
            children,
        ))
        .when_some(self.error.clone(), |card, error| {
            card.child(widgets::card_row(&theme, false).child(widgets::error_strip(&theme, error)))
        })
    }
}
