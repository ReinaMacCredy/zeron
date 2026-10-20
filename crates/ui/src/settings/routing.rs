use gpui::{Context, Entity, SharedString, Subscription, Task, Window, div, prelude::*, px};
use zeron_engine::routing::RoutingSettings;
use zeron_rpc::methods;

use crate::popover::{self, Loadable};
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;

pub struct RoutingPage {
    state: Entity<AppState>,
    scroll: widgets::PageScroll,
    settings: Loadable<RoutingSettings>,
    task: Option<Task<()>>,
    _state: Subscription,
}

impl RoutingPage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let state_sub = cx.observe(&state, |this: &mut Self, _, cx| {
            if matches!(this.settings, Loadable::Idle) {
                this.load(cx);
            }
        });
        let mut page = Self {
            state,
            scroll: widgets::PageScroll::default(),
            settings: Loadable::Idle,
            task: None,
            _state: state_sub,
        };
        page.load(cx);
        page
    }

    pub(crate) fn load(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        if matches!(self.settings, Loadable::Idle) {
            self.settings = Loadable::Loading;
        }
        self.task = Some(cx.spawn(async move |this, cx| {
            let settings = engine
                .client()
                .call_as::<RoutingSettings>(methods::GET_ROUTING_SETTINGS, serde_json::json!({}))
                .await
                .map_err(|e| e.to_string());
            this.update(cx, |page, cx| {
                page.settings = match settings {
                    Ok(settings) => Loadable::Ready(settings),
                    Err(error) => Loadable::Error(error),
                };
                cx.notify();
            })
            .ok();
        }));
    }

    fn on_scroll_hovered(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.scroll.set_list_hovered(*hovered) {
            cx.notify();
        }
    }
}

impl popover::ScrollRailHost for RoutingPage {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        self.scroll.rail_bar()
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.scroll.rail_scroll()
    }
}

impl Render for RoutingPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_settings_surface();
        let note = match &self.settings {
            Loadable::Error(error) => error.clone(),
            _ => "Loading…".to_string(),
        };
        let scrollbar = popover::rail(self, "routing-scrollbar", &theme, cx);
        div()
            .id("routing-host")
            .relative()
            .size_full()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .child(
                crate::edge_fade::edge_faded(
                    16.0,
                    true,
                    true,
                    div()
                        .id("routing-page")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll.scroll)
                        .child(
                            widgets::page_column()
                                .child(widgets::page_header(&theme, "Routing", None))
                                .child(
                                    widgets::section_card(&theme).child(
                                        widgets::card_row(&theme, true).child(
                                            div()
                                                .text_color(theme.text_muted)
                                                .child(SharedString::from(note)),
                                        ),
                                    ),
                                )
                                .mt(px(0.0)),
                        ),
                )
                .fade_overflow_y(&self.scroll.scroll),
            )
            .children(scrollbar)
    }
}
