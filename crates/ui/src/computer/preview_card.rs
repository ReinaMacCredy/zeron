use gpui::{
    Context, Entity, FontWeight, Hsla, ImageSource, InteractiveElement as _, IntoElement,
    ObjectFit, ParentElement as _, Pixels, Render, SharedString, Size,
    StatefulInteractiveElement as _, Styled as _, StyledImage as _, Subscription, Task, Window,
    div, img, prelude::FluentBuilder as _, px,
};
use zeron_proto::computer::{ComputerPreviewSize, ComputerSettings};
use zeron_proto::view::Indicator;

use super::preview::{
    self, BadgeTone, ComputerPreviewStore, PreviewPhase, PreviewView, STILL_POLL_INTERVAL,
    StatusBadge,
};
use crate::state::AppState;
use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardCaps {
    pub min_width: f32,
    pub max_width: f32,
}

pub fn card_caps(size: ComputerPreviewSize) -> CardCaps {
    match size {
        ComputerPreviewSize::Compact => CardCaps {
            min_width: 240.0,
            max_width: 400.0,
        },
        ComputerPreviewSize::Large => CardCaps {
            min_width: 240.0,
            max_width: 560.0,
        },
    }
}

const SLOT_TOP: f32 = 16.0;
const SLOT_BOTTOM_RESERVE: f32 = 120.0;
const CARD_TOP: f32 = Theme::TITLEBAR_HEIGHT + 12.0;
const CARD_RIGHT: f32 = 16.0;
const FALLBACK_ASPECT: f32 = 16.0 / 10.0;

pub fn budget_width(caps: CardCaps, slot_width: f32) -> f32 {
    (slot_width - 520.0 - 24.0).min(caps.max_width).max(200.0)
}

pub fn fit_width(caps: CardCaps, slot: Size<Pixels>, aspect: f32) -> f32 {
    let budget = budget_width(caps, f32::from(slot.width));
    let by_height = (f32::from(slot.height) - SLOT_TOP - SLOT_BOTTOM_RESERVE) * aspect;
    budget
        .min(caps.max_width)
        .min(by_height)
        .max(caps.min_width)
}

pub struct ComputerPreviewCard {
    state: Entity<AppState>,
    store: Entity<ComputerPreviewStore>,
    chat_id: Option<String>,
    slot: Size<Pixels>,
    hovered: bool,
    still_poll: Option<(String, Task<()>)>,
    settings_fetch: Option<Task<()>>,
    seen_settings: Option<ComputerSettings>,
    _observers: [Subscription; 2],
}

impl ComputerPreviewCard {
    pub fn new(
        state: Entity<AppState>,
        store: Entity<ComputerPreviewStore>,
        cx: &mut Context<Self>,
    ) -> Self {
        let observers = [
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.observe(&state, |_, _, cx| cx.notify()),
        ];
        Self {
            state,
            store,
            chat_id: None,
            slot: Size::default(),
            hovered: false,
            still_poll: None,
            settings_fetch: None,
            seen_settings: None,
            _observers: observers,
        }
    }

    pub fn set_target(
        &mut self,
        chat_id: Option<String>,
        slot: Size<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.chat_id != chat_id || self.slot != slot {
            if self.chat_id != chat_id {
                self.hovered = false;
            }
            self.chat_id = chat_id;
            self.slot = slot;
            cx.notify();
        }
    }

    pub fn hide(&mut self, cx: &mut Context<Self>) {
        let Some(chat) = self.chat_id.clone() else {
            return;
        };
        preview::update_store(&self.store, cx, |store| store.hide_for_task(&chat));
    }

    fn working(&self, chat: &str, cx: &Context<Self>) -> bool {
        self.state.read(cx).indicator_for(chat, chrono::Utc::now()) == Indicator::Working
    }

    fn reconcile(&mut self, chat: &str, view: &PreviewView, cx: &mut Context<Self>) {
        if view.phase == Some(PreviewPhase::Armed) {
            let store = self.store.clone();
            let chat = chat.to_owned();
            cx.defer(move |cx| preview::update_store(&store, cx, |store| store.mark_live(&chat)));
        }
        let shared = self.state.read(cx).computer_settings.clone();
        if let Some(settings) = shared.clone().filter(|_| shared != self.seen_settings) {
            self.seen_settings = shared;
            let store = self.store.clone();
            cx.defer(move |cx| {
                preview::update_store(&store, cx, |store| store.set_settings(settings))
            });
        }
        let engine = self.state.read(cx).engine().cloned();
        if self.store.read(cx).settings_stale()
            && self.settings_fetch.is_none()
            && let Some(engine) = engine.clone()
        {
            let store = self.store.clone();
            let fetch = preview::fetch_settings(engine, store, cx);
            self.settings_fetch = Some(cx.spawn(async move |this, cx| {
                fetch.await;
                this.update(cx, |this, _| this.settings_fetch = None).ok();
            }));
        }
        let wanted = view.wants_stills && (view.open || self.working(chat, cx));
        match (&self.still_poll, wanted, engine) {
            (Some((polled, _)), true, _) if polled == chat => {}
            (_, true, Some(engine)) => {
                let store = self.store.clone();
                let polled = chat.to_owned();
                let task = cx.spawn(async move |this, cx| {
                    loop {
                        let frame = match preview::read_still(&engine, &polled).await {
                            Some((seq, bytes)) => cx
                                .background_executor()
                                .spawn(async move { preview::decode_frame(seq, &bytes) })
                                .await
                                .map(Some),
                            None => Some(None),
                        };
                        if this.upgrade().is_none() {
                            return;
                        }
                        if let Some(frame) = frame {
                            cx.update(|cx| {
                                preview::update_store(&store, cx, |store| {
                                    store.accept_still(&polled, frame)
                                })
                            });
                        }
                        cx.background_executor().timer(STILL_POLL_INTERVAL).await;
                    }
                });
                self.still_poll = Some((chat.to_owned(), task));
            }
            (_, false, _) | (_, true, None) => self.still_poll = None,
        }
    }
}

fn pill(color: Hsla, dot: Hsla, label: String) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .max_w_full()
        .px(px(10.0))
        .py(px(4.0))
        .rounded_full()
        .border_1()
        .border_color(gpui::white().opacity(0.15))
        .bg(gpui::black().opacity(0.55))
        .text_size(px(11.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(color)
        .child(div().flex_none().size(px(6.0)).rounded_full().bg(dot))
        .child(div().truncate().child(label))
}

fn badge_colors(badge: &StatusBadge, theme: &Theme) -> Hsla {
    match badge.tone {
        BadgeTone::Danger => theme.danger,
        BadgeTone::Warning => theme.warning,
        BadgeTone::Muted => gpui::white().opacity(0.72),
        BadgeTone::Active => theme.success,
    }
}

impl Render for ComputerPreviewCard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(chat) = self.chat_id.clone() else {
            self.still_poll = None;
            return div().into_any_element();
        };
        let (view, size) = {
            let store = self.store.read(cx);
            (store.view(&chat), store.preview_size())
        };
        self.reconcile(&chat, &view, cx);
        if !view.visually_open() {
            return div().into_any_element();
        }
        let theme = Theme::of(cx).clone();
        let aspect = view
            .frame
            .as_ref()
            .map(|frame| frame.width as f32 / frame.height as f32)
            .unwrap_or(FALLBACK_ASPECT);
        let width = fit_width(card_caps(size), self.slot, aspect);
        let height = width / aspect;
        let body = match &view.frame {
            Some(frame) => img(ImageSource::Render(frame.image.clone()))
                .size_full()
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .px(px(12.0))
                .text_size(px(12.0))
                .text_color(theme.text_muted)
                .child(SharedString::from(
                    view.failure
                        .as_ref()
                        .map(|failure| failure.message.clone())
                        .unwrap_or_else(|| "Waiting for the window the agent is using…".into()),
                ))
                .into_any_element(),
        };
        let badge = view
            .badge
            .clone()
            .filter(|badge| badge.tone != BadgeTone::Active);
        let hovered = self.hovered;
        div()
            .id("computer-preview-card")
            .debug_selector(|| "computer-preview-card".into())
            .absolute()
            .top(px(CARD_TOP))
            .right(px(CARD_RIGHT))
            .w(px(width))
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(16.0))
            .border_1()
            .border_color(gpui::white().opacity(0.1))
            .bg(theme.surface_overlay.opacity(0.95))
            .shadow_lg()
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.hovered = *hovered;
                cx.notify();
            }))
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(height))
                    .overflow_hidden()
                    .bg(gpui::black())
                    .child(body)
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .border_1()
                            .border_color(gpui::black().opacity(0.45)),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .right_0()
                            .h(gpui::relative(0.45))
                            .bg(gpui::linear_gradient(
                                180.0,
                                gpui::linear_color_stop(gpui::white().opacity(0.09), 0.0),
                                gpui::linear_color_stop(gpui::white().opacity(0.0), 1.0),
                            )),
                    )
                    .when_some(badge, |viewport, badge| {
                        let color = badge_colors(&badge, &theme);
                        viewport.child(
                            div()
                                .absolute()
                                .top(px(8.0))
                                .left(px(8.0))
                                .max_w(px(width - 56.0))
                                .child(pill(color, color, badge.label)),
                        )
                    })
                    .when_some(view.status_label.clone(), |viewport, label| {
                        viewport.child(
                            div()
                                .absolute()
                                .bottom(px(8.0))
                                .left(px(8.0))
                                .max_w(px(width - 16.0))
                                .child(pill(gpui::white(), theme.text_muted, label)),
                        )
                    })
                    .when(hovered, |viewport| {
                        viewport.child(
                            div()
                                .absolute()
                                .top(px(8.0))
                                .right(px(8.0))
                                .p(px(4.0))
                                .rounded_full()
                                .border_1()
                                .border_color(gpui::white().opacity(0.2))
                                .bg(gpui::white().opacity(0.12))
                                .child(
                                    div()
                                        .id("computer-preview-hide")
                                        .debug_selector(|| "computer-preview-hide".into())
                                        .size(px(28.0))
                                        .rounded_full()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .cursor_pointer()
                                        .hover(|s| s.bg(gpui::white().opacity(0.2)))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.hide(cx);
                                        }))
                                        .child(
                                            crate::icons::icon(crate::icons::CLOSE)
                                                .size(px(16.0))
                                                .text_color(gpui::white()),
                                        ),
                                ),
                        )
                    }),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use gpui::{AppContext as _, TestAppContext, VisualTestContext, size};
    use zeron_proto::computer::ComputerEvent;
    use zeron_rpc::methods;

    use super::*;
    use crate::computer::preview::PreviewFailure;
    use crate::computer::preview::tests::{driving, image};

    struct Fixture(Entity<ComputerPreviewCard>);

    impl Render for Fixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .relative()
                .w(px(1200.0))
                .h(px(800.0))
                .child(self.0.clone())
        }
    }

    struct Wire(tokio::sync::mpsc::Receiver<String>);

    impl Wire {
        fn methods(&mut self, cx: &mut VisualTestContext) -> Vec<(String, serde_json::Value)> {
            cx.run_until_parked();
            let mut methods = Vec::new();
            while let Ok(frame) = self.0.try_recv() {
                let frame: serde_json::Value = serde_json::from_str(&frame).unwrap();
                methods.push((
                    frame["method"].as_str().unwrap_or_default().to_owned(),
                    frame["params"].clone(),
                ));
            }
            methods
        }
    }

    fn mount<'a>(
        cx: &'a mut TestAppContext,
        dir: &std::path::Path,
    ) -> (
        Entity<ComputerPreviewStore>,
        Entity<ComputerPreviewCard>,
        Wire,
        &'a mut VisualTestContext,
    ) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
            crate::settings::init(crate::settings::UiSettings::default(), dir, cx);
        });
        let (out, frames) = tokio::sync::mpsc::channel::<String>(64);
        let (replies, inbound) = tokio::sync::mpsc::channel::<String>(64);
        // A closed inbound fails the pending call from a tokio thread.
        std::mem::forget(replies);
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| {
            state.set_test_engine(crate::state::EngineHandle::from_test_client(
                zeron_rpc::RpcClient::new(out, inbound),
            ))
        });
        let store = cx.new(|_| ComputerPreviewStore::default());
        let card = cx.new(|cx| ComputerPreviewCard::new(state, store.clone(), cx));
        card.update(cx, |card, cx| {
            card.set_target(Some("a".into()), size(px(1200.0), px(800.0)), cx)
        });
        let (_, cx) = cx.add_window_view(|_, _| Fixture(card.clone()));
        (store, card, Wire(frames), cx)
    }

    fn change(
        store: &Entity<ComputerPreviewStore>,
        cx: &mut VisualTestContext,
        change: impl FnOnce(&mut ComputerPreviewStore) -> bool,
    ) {
        cx.update(|_, cx| preview::update_store(store, cx, change));
        cx.run_until_parked();
    }

    fn phase(
        store: &Entity<ComputerPreviewStore>,
        cx: &mut VisualTestContext,
    ) -> Option<PreviewPhase> {
        cx.update(|_, cx| store.read(cx).view("a").phase)
    }

    #[gpui::test]
    fn the_card_appears_with_its_first_frame_and_sizes_from_settings(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        let (store, _card, mut wire, cx) = mount(cx, dir.path());
        change(&store, cx, |store| {
            store.set_events_live(true);
            false
        });
        assert!(cx.debug_bounds("computer-preview-card").is_none());
        change(&store, cx, |store| {
            driving(store, "a");
            true
        });
        assert_eq!(
            phase(&store, cx),
            Some(PreviewPhase::Live),
            "rendering the chat views it"
        );
        assert!(
            cx.debug_bounds("computer-preview-card").is_none(),
            "no empty box before the first frame"
        );
        let asked: Vec<_> = wire.methods(cx).into_iter().map(|(m, _)| m).collect();
        assert!(asked.contains(&methods::GET_COMPUTER_SETTINGS.to_owned()));
        change(&store, cx, |store| {
            store.accept_tap_frame("a", image(1, 960, 600))
        });
        let compact = cx
            .debug_bounds("computer-preview-card")
            .expect("card painted");
        assert_eq!(f32::from(compact.size.width), 400.0);
        change(&store, cx, |store| {
            store.set_settings(ComputerSettings {
                computer_preview_size: ComputerPreviewSize::Large,
                ..ComputerSettings::default()
            })
        });
        let large = cx
            .debug_bounds("computer-preview-card")
            .expect("card painted");
        assert_eq!(f32::from(large.size.width), 560.0);
        assert_eq!(f32::from(large.size.height).round(), 352.0);
        change(&store, cx, |store| {
            store.accept_tap_failure(
                "a",
                PreviewFailure {
                    code: "target_unavailable".into(),
                    message: "The selected window is no longer available.".into(),
                },
            )
        });
        assert!(
            cx.debug_bounds("computer-preview-card").is_some(),
            "a closed window keeps the last frame up"
        );
        change(&store, cx, |store| {
            store.set_settings(ComputerSettings {
                auto_open_computer_pane: false,
                ..ComputerSettings::default()
            })
        });
        assert!(cx.debug_bounds("computer-preview-card").is_none());
    }

    #[gpui::test]
    fn hiding_closes_the_card_and_leaves_the_task_running(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        let (store, _card, mut wire, cx) = mount(cx, dir.path());
        change(&store, cx, |store| {
            store.set_events_live(true);
            driving(store, "a");
            store.accept_tap_frame("a", image(1, 960, 600))
        });
        wire.methods(cx);
        let card = cx
            .debug_bounds("computer-preview-card")
            .expect("card painted");
        assert!(
            cx.debug_bounds("computer-preview-hide").is_none(),
            "controls reveal on hover"
        );
        cx.simulate_mouse_move(card.center(), None, gpui::Modifiers::default());
        cx.run_until_parked();
        let hide = cx
            .debug_bounds("computer-preview-hide")
            .expect("hide on hover");
        cx.simulate_click(hide.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("computer-preview-card").is_none());
        assert_eq!(phase(&store, cx), Some(PreviewPhase::HiddenForTask));
        assert!(
            wire.methods(cx).is_empty(),
            "hiding sends nothing to the engine"
        );
        change(&store, cx, |store| {
            store.accept_tap_frame("a", image(2, 960, 600))
        });
        assert!(
            cx.debug_bounds("computer-preview-card").is_none(),
            "frames keep flowing into a hidden card without reopening it"
        );
        change(&store, cx, |store| {
            store.apply_event(&ComputerEvent::OpenPaneRequested {
                thread_id: "a".into(),
            })
        });
        assert_eq!(
            phase(&store, cx),
            Some(PreviewPhase::Live),
            "the next lease re-arms"
        );
    }

    #[gpui::test]
    fn a_viewer_without_the_tap_polls_the_still_frame_for_its_chat(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        let (store, _card, mut wire, cx) = mount(cx, dir.path());
        cx.run_until_parked();
        assert!(
            wire.methods(cx).is_empty(),
            "an idle remote chat does not poll"
        );
        change(&store, cx, |store| {
            store.apply_event(&ComputerEvent::OpenPaneRequested {
                thread_id: "a".into(),
            })
        });
        let calls = wire.methods(cx);
        let still = calls
            .iter()
            .find(|(method, _)| method == methods::GET_COMPUTER_STILL_FRAME)
            .expect("the still fallback is read");
        assert_eq!(still.1["chatId"], "a");
        change(&store, cx, |store| {
            store.accept_still("a", Some(image(3, 640, 400)))
        });
        let card = cx
            .debug_bounds("computer-preview-card")
            .expect("the still is shown");
        assert_eq!(f32::from(card.size.width), 400.0);
    }
}
