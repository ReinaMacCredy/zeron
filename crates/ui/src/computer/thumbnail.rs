use gpui::{
    Context, Entity, ImageSource, IntoElement, ObjectFit, ParentElement as _, Render, SharedString,
    Styled as _, StyledImage as _, Subscription, Window, div, img, prelude::FluentBuilder as _, px,
};

use super::preview::ComputerPreviewStore;
use super::preview_card::body_copy;
use crate::theme::Theme;

pub const THUMBNAIL_HEIGHT: f32 = 118.0;

pub struct ComputerThumbnail {
    store: Entity<ComputerPreviewStore>,
    chat_id: Option<String>,
    #[cfg(feature = "appshots-fixture")]
    painted: Option<gpui::Bounds<gpui::Pixels>>,
    _observer: Subscription,
}

impl ComputerThumbnail {
    pub fn new(store: Entity<ComputerPreviewStore>, cx: &mut Context<Self>) -> Self {
        Self {
            _observer: cx.observe(&store, |_, _, cx| cx.notify()),
            store,
            chat_id: None,
            #[cfg(feature = "appshots-fixture")]
            painted: None,
        }
    }

    #[cfg(feature = "appshots-fixture")]
    pub fn fixture_bounds(&self) -> Option<gpui::Bounds<gpui::Pixels>> {
        self.painted
    }

    pub fn set_chat(&mut self, chat_id: Option<String>, cx: &mut Context<Self>) {
        if self.chat_id != chat_id {
            self.chat_id = chat_id;
            cx.notify();
        }
    }
}

impl Render for ComputerThumbnail {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(chat) = self.chat_id.as_deref() else {
            return div().into_any_element();
        };
        let view = self.store.read(cx).view(chat);
        let theme = Theme::of(cx);
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
                .px(px(10.0))
                .text_size(px(11.0))
                .text_color(theme.text_muted)
                .child(SharedString::from(body_copy(&view).unwrap_or_default()))
                .into_any_element(),
        };
        let thumb = div()
            .relative()
            .w_full()
            .h(px(THUMBNAIL_HEIGHT))
            .rounded(px(9.0))
            .overflow_hidden()
            .border_1()
            .border_color(crate::theme::hairline(0.1))
            .bg(gpui::black())
            .child(body)
            .when_some(view.status_label, |thumb, label| {
                thumb.child(
                    div()
                        .absolute()
                        .left(px(6.0))
                        .bottom(px(6.0))
                        .max_w(px(200.0))
                        .flex()
                        .items_center()
                        .gap(px(5.0))
                        .px(px(8.0))
                        .py(px(2.0))
                        .rounded_full()
                        .bg(gpui::black().opacity(0.65))
                        .text_size(px(10.5))
                        .text_color(gpui::white().opacity(0.92))
                        .child(
                            div()
                                .flex_none()
                                .size(px(5.0))
                                .rounded_full()
                                .bg(theme.accent),
                        )
                        .child(div().truncate().child(label)),
                )
            });
        #[cfg(feature = "appshots-fixture")]
        let thumb = {
            let this = cx.entity().downgrade();
            thumb.child(
                gpui::canvas(
                    move |bounds, _, cx| {
                        let _ = this.update(cx, |this, _| this.painted = Some(bounds));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
        };
        thumb.into_any_element()
    }
}
