use gpui::{AnyElement, Context, MouseButton, SharedString, div, prelude::*, px};

use super::Composer;
use crate::quotes::Quote;
use crate::theme::Theme;

pub(super) const ANNOTATION_CHIP_WIDTH: f32 = 130.0;
pub(super) const ANNOTATION_CHIP_HEIGHT: f32 = 32.0;
const CHIP_RADIUS: f32 = 12.0;
const REMOVE_SIZE: f32 = 19.0;
const CARD_WIDTH: f32 = 385.0;
const CARD_MAX_HEIGHT: f32 = 420.0;
const CARD_RADIUS: f32 = 12.0;
const CARD_GAP: f32 = 4.5;
const CARD_TEXT_SIZE: f32 = 13.0;
const CARD_LINE_HEIGHT: f32 = 18.5;
const NUMBER_WIDTH: f32 = 17.5;
const ROW_ICON: f32 = 16.0;

/// The pointer is on the chip or on its card; moving between the two keeps the
/// card up, since both report in one dispatch before the next render.
#[derive(Default)]
pub(crate) struct AnnotationHover {
    chip: bool,
    card: bool,
}

impl Composer {
    pub(super) fn render_annotation_chip(
        &self,
        annotations: &[Quote],
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label = zeron_proto::badges::annotation_label(annotations.len());
        let chip_hovered = self.annotation_hover.chip;
        let card_open = chip_hovered || self.annotation_hover.card;
        let key = self.current_key.clone();
        let chip = div()
            .id("composer-annotations")
            .debug_selector(|| "composer-annotations".to_string())
            .role(gpui::Role::Group)
            .aria_label(label.clone())
            .h(px(ANNOTATION_CHIP_HEIGHT))
            .min_w(px(ANNOTATION_CHIP_WIDTH))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(5.0))
            .pl(px(10.0))
            .pr(px(4.0))
            .rounded(px(CHIP_RADIUS))
            .border_1()
            .border_color(theme.border)
            .text_size(crate::typography::ui_rems(13.0))
            .text_color(theme.text)
            .on_hover(cx.listener(|this, hovered, _, cx| {
                this.annotation_hover.chip = *hovered;
                cx.notify();
            }))
            .child(
                crate::icons::icon(crate::icons::CHAT_ROUND_LINE)
                    .size(px(12.0))
                    .text_color(theme.text_muted),
            )
            .child(div().flex_1().child(label))
            .child(
                div()
                    .id("composer-annotations-remove")
                    .debug_selector(|| "composer-annotations-remove".to_string())
                    .role(gpui::Role::Button)
                    .aria_label("Remove annotations")
                    .flex_none()
                    .size(px(REMOVE_SIZE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .border_1()
                    .border_color(theme.bg)
                    .bg(theme.solid)
                    .cursor_pointer()
                    .when(!chip_hovered, |el| el.opacity(0.0))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.annotation_hover = AnnotationHover::default();
                        this.state.update(cx, |state, cx| {
                            state.remove_annotations(&key);
                            cx.notify();
                        });
                        cx.notify();
                    }))
                    .child(
                        crate::icons::icon(crate::icons::LUCIDE_X)
                            .size(px(10.0))
                            .text_color(theme.on_solid),
                    ),
            );
        let card = card_open.then(|| self.render_annotation_card(annotations, theme, cx));
        div()
            .relative()
            .flex_none()
            .child(chip)
            .children(card)
            .into_any_element()
    }

    fn annotation_row_action(
        id: SharedString,
        label: String,
        icon: &'static str,
        popup: &Theme,
    ) -> gpui::Stateful<gpui::Div> {
        let selector = id.to_string();
        div()
            .id(id)
            .debug_selector(move || selector.clone())
            .role(gpui::Role::Button)
            .aria_label(label)
            .flex_none()
            .size(px(ROW_ICON + 4.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.0))
            .cursor_pointer()
            .hover(|style| style.bg(popup.glass_hover()))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                crate::icons::icon(icon)
                    .size(px(ROW_ICON))
                    .text_color(popup.text_muted),
            )
    }

    fn render_annotation_row(
        &self,
        number: usize,
        quote: &Quote,
        popup: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let label = |text: String| {
            div()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(popup.text_muted)
                .child(text)
        };
        let comment = quote.comment.clone().unwrap_or_default();
        let selected = match &quote.source {
            Some(path) => format!("Selected text from {path}:"),
            None => "Selected text:".to_string(),
        };
        let can_edit = self.state.read(cx).annotation_anchor(&quote.id).is_some();
        let key = self.current_key.clone();
        let id = quote.id.clone();
        let edit = can_edit.then(|| {
            let (key, id) = (key.clone(), id.clone());
            Self::annotation_row_action(
                format!("annotation-edit-{number}").into(),
                format!("Edit annotation {number}"),
                crate::icons::PEN,
                popup,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.annotation_hover = AnnotationHover::default();
                this.state.update(cx, |state, cx| {
                    state.request_annotation_edit(&key, &id);
                    cx.notify();
                });
                cx.notify();
            }))
        });
        let remove = Self::annotation_row_action(
            format!("annotation-remove-{number}").into(),
            format!("Remove annotation {number}"),
            crate::icons::TRASH_BIN_MINIMALISTIC,
            popup,
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            this.state.update(cx, |state, cx| {
                state.remove_quote(&key, &id);
                cx.notify();
            });
            cx.notify();
        }));
        div()
            .flex()
            .flex_row()
            .child(
                div()
                    .flex_none()
                    .w(px(NUMBER_WIDTH))
                    .text_color(popup.text_muted)
                    .child(format!("{number}.")),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_between()
                            .gap(px(8.0))
                            .child(div().min_w_0().child(label(selected)))
                            .child(
                                div()
                                    .flex_none()
                                    .flex()
                                    .flex_row()
                                    .gap(px(4.5))
                                    .children(edit)
                                    .child(remove),
                            ),
                    )
                    .child(
                        div()
                            .pr(px(20.0))
                            .child(SharedString::from(quote.text.clone())),
                    )
                    .when(!comment.is_empty(), |el| {
                        el.child(div().mt(px(5.0)).child(label("User comment:".to_string())))
                            .child(
                                div()
                                    .pr(px(20.0))
                                    .child(SharedString::from(comment.clone())),
                            )
                    }),
            )
    }

    fn render_annotation_card(
        &self,
        annotations: &[Quote],
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let popup = theme.for_popup();
        let rows: Vec<_> = annotations
            .iter()
            .enumerate()
            .map(|(ix, quote)| self.render_annotation_row(ix + 1, quote, &popup, cx))
            .collect();
        let card = crate::popover::popover_card_flush(&popup)
            .id("composer-annotations-card")
            .debug_selector(|| "composer-annotations-card".to_string())
            .w(px(CARD_WIDTH))
            .max_h(px(CARD_MAX_HEIGHT))
            .rounded(px(CARD_RADIUS))
            .bg(crate::annotations::surface_fill(&popup))
            .overflow_y_scroll()
            .on_scroll_wheel(crate::swipe::claim_any)
            .flex()
            .flex_col()
            .gap(px(12.0))
            .pt(px(13.0))
            .pb(px(13.0))
            .pl(px(14.0))
            .pr(px(14.0))
            .text_size(crate::typography::ui_rems(CARD_TEXT_SIZE))
            .line_height(px(CARD_LINE_HEIGHT))
            .children(rows);
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_0()
            .child(
                gpui::deferred(
                    gpui::anchored()
                        .anchor(gpui::Anchor::BottomLeft)
                        .snap_to_window_with_margin(px(8.0))
                        .child(
                            div()
                                .id("composer-annotations-card-zone")
                                .occlude()
                                .pb(px(CARD_GAP))
                                .on_hover(cx.listener(|this, hovered, _, cx| {
                                    this.annotation_hover.card = *hovered;
                                    cx.notify();
                                }))
                                .child(crate::frost::layered(card)),
                        ),
                )
                .priority(1),
            )
            .into_any_element()
    }
}
