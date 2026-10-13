//! Shared inline review cards for diffs and Markdown file previews.
use crate::changes::ACCENT_BAR_WIDTH;
use crate::{
    comments::{self, ReviewComment},
    composer::ComposerInput,
    motion,
    theme::Theme,
};
use gpui::{AnyElement, Context, Entity, SharedString, div, prelude::*, px};

/// Align review controls with a centered document column while keeping the card full width.
#[derive(Clone, Copy)]
pub(crate) struct CommentContentColumn {
    pub max_width: f32,
    pub gutter: f32,
}

pub(crate) fn render_comment_adder<T: 'static>(
    id: SharedString,
    theme: &Theme,
    cx: &Context<T>,
    open: impl Fn(&mut T, &mut gpui::Window, &mut Context<T>) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .size(px(crate::changes::COMMENT_ADDER_SIZE))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .bg(theme.solid)
        .cursor_pointer()
        .role(gpui::Role::Button)
        .aria_label("Add comment")
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |this, _, window, cx| {
            cx.stop_propagation();
            open(this, window, cx);
        }))
        .child(
            crate::icons::icon(crate::icons::PLUS)
                .size(px(11.0))
                .text_color(theme.on_solid),
        )
        .into_any_element()
}

pub(crate) fn render_comment_card<T: 'static>(
    comment: &ReviewComment,
    theme: &Theme,
    cx: &Context<T>,
    edit: fn(&mut T, &str, &mut gpui::Window, &mut Context<T>),
    remove: fn(&mut T, &str, &mut Context<T>),
    column: Option<CommentContentColumn>,
) -> AnyElement {
    let group: SharedString = format!("cmt-card-{}", comment.id).into();
    let id = comment.id.clone();
    div()
        .group(group.clone())
        .w_full()
        .flex_none()
        .flex()
        .flex_row()
        .bg(crate::theme::ink(0.05))
        // A bar, not a border: it must match ACCENT_BAR_WIDTH exactly or the
        // card's edge steps in and out of the column.
        .when_some(column, |el, column| {
            el.relative().justify_center().px(px(column.gutter))
        })
        .child(
            comment_accent_bar(theme.solid.opacity(0.35)).when(column.is_some(), |el| {
                el.absolute().left(px(0.0)).top(px(0.0))
            }),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .px(px(Theme::SPACE_LG))
                .when_some(column, |el, column| {
                    el.max_w(px(column.max_width)).px(px(0.0))
                })
                .py(px(comments::CARD_PAD_V / 2.0))
                .child(
                    div()
                        .h(px(comments::CARD_HEADER_HEIGHT))
                        .flex_none()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            crate::icons::icon(crate::icons::CHAT_ROUND_LINE)
                                .size(px(12.0))
                                .text_color(theme.text_faint),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_family(theme.font_mono.clone())
                                .text_size(px(11.0))
                                .text_color(theme.text_faint)
                                .child(SharedString::from(comment.location())),
                        )
                        .child(render_comment_edit(comment, group.clone(), theme, cx, edit))
                        .child(
                            div()
                                .id(SharedString::from(format!("cmt-remove-{}", comment.id)))
                                .flex_none()
                                .size(px(16.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(4.0))
                                .cursor_pointer()
                                .opacity(0.0)
                                .group_hover(group, |s| s.opacity(1.0))
                                .on_click(cx.listener(move |this, _, _, cx| remove(this, &id, cx)))
                                .tooltip(crate::settings::widgets::text_tooltip("Remove comment"))
                                .child(
                                    crate::icons::icon(crate::icons::CLOSE_CIRCLE)
                                        .size(px(12.0))
                                        .text_color(theme.text_muted),
                                ),
                        ),
                )
                .child(
                    div()
                        .max_h(px(comments::CARD_BODY_MAX_HEIGHT))
                        .max_w(px(comments::CARD_BODY_MAX_WIDTH))
                        .overflow_hidden()
                        .text_size(px(12.0))
                        .line_height(px(comments::CARD_LINE_HEIGHT))
                        .text_color(theme.text_dim)
                        .child(SharedString::from(comment.body.clone())),
                ),
        )
        .into_any_element()
}

pub(crate) fn render_finding_card<T: 'static>(
    comment: &ReviewComment,
    severity: crate::review::Severity,
    theme: &Theme,
    cx: &Context<T>,
    fix: fn(&mut T, &str, &mut Context<T>),
    dismiss: fn(&mut T, &str, &mut Context<T>),
) -> AnyElement {
    let (tint, _) = severity.colors(theme);
    let (_, title, detail) = crate::review::split_finding(&comment.body);
    let title = title.to_string();
    let action = |label: &'static str, primary: bool, run: fn(&mut T, &str, &mut Context<T>)| {
        let id = comment.id.clone();
        div()
            .id(SharedString::from(format!(
                "finding-{}-{}",
                label.to_lowercase(),
                comment.id
            )))
            .flex_none()
            .h(px(22.0))
            .px(px(9.0))
            .flex()
            .items_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .text_size(px(11.0))
            .font_weight(gpui::FontWeight::MEDIUM)
            .role(gpui::Role::Button)
            .aria_label(format!("{label} {title}"))
            .when(primary, |el| {
                el.border_1()
                    .border_color(theme.border)
                    .bg(crate::theme::ink(0.06))
                    .text_color(theme.text)
                    .hover(|s| s.bg(crate::theme::ink(0.12)))
            })
            .when(!primary, |el| {
                el.text_color(theme.text_muted)
                    .hover(|s| s.bg(theme.glass_hover()).text_color(theme.text))
            })
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                run(this, &id, cx);
            }))
            .child(label)
    };
    div()
        .w_full()
        .flex_none()
        .flex()
        .flex_row()
        .bg(crate::theme::ink(0.05))
        .child(comment_accent_bar(tint))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .px(px(Theme::SPACE_LG))
                .py(px(comments::CARD_PAD_V / 2.0))
                .child(
                    div()
                        .h(px(comments::CARD_HEADER_HEIGHT))
                        .flex_none()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .flex_none()
                                .flex()
                                .items_center()
                                .gap(px(5.0))
                                .text_size(px(11.0))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(tint)
                                .child(div().size(px(6.0)).rounded_full().bg(tint))
                                .child(severity.word()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.5))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(theme.text)
                                .child(SharedString::from(title.clone())),
                        )
                        .child(action("Fix", true, fix))
                        .child(action("Dismiss", false, dismiss)),
                )
                .when(!detail.is_empty(), |el| {
                    el.child(
                        div()
                            .max_h(px(comments::CARD_BODY_MAX_HEIGHT))
                            .max_w(px(comments::CARD_BODY_MAX_WIDTH))
                            .overflow_hidden()
                            .text_size(px(12.0))
                            .line_height(px(comments::CARD_LINE_HEIGHT))
                            .text_color(theme.text_muted)
                            .child(SharedString::from(detail.to_string())),
                    )
                }),
        )
        .into_any_element()
}

pub(crate) fn render_comment_edit<T: 'static>(
    comment: &ReviewComment,
    group: SharedString,
    theme: &Theme,
    cx: &Context<T>,
    edit: fn(&mut T, &str, &mut gpui::Window, &mut Context<T>),
) -> AnyElement {
    let id = comment.id.clone();
    div()
        .id(SharedString::from(format!("cmt-edit-{id}")))
        .flex_none()
        .size(px(16.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .cursor_pointer()
        .role(gpui::Role::Button)
        .aria_label("Edit comment")
        .opacity(0.0)
        .group_hover(group, |style| style.opacity(1.0))
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |this, _, window, cx| {
            cx.stop_propagation();
            edit(this, &id, window, cx);
        }))
        .tooltip(crate::settings::widgets::text_tooltip("Edit comment"))
        .child(
            crate::icons::icon(crate::icons::PEN)
                .size(px(12.0))
                .text_color(theme.text_muted),
        )
        .into_any_element()
}

fn comment_accent_bar(color: gpui::Hsla) -> gpui::Div {
    div().w(px(ACCENT_BAR_WIDTH)).h_full().flex_none().bg(color)
}

/// Fixed height, so an open draft never fights the fold tween.
pub(crate) fn render_comment_draft<T: 'static>(
    path: &str,
    line: u32,
    input: Entity<ComposerInput>,
    editing: bool,
    theme: &Theme,
    cx: &Context<T>,
    cancel: fn(&mut T, &mut Context<T>),
    commit: fn(&mut T, &mut Context<T>),
    column: Option<CommentContentColumn>,
) -> AnyElement {
    div()
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
            if event.keystroke.key == "escape" {
                cx.stop_propagation();
                cancel(this, cx);
            }
        }))
        .h(px(comments::DRAFT_CARD_HEIGHT))
        .w_full()
        .flex_none()
        .flex()
        .flex_row()
        .bg(crate::theme::ink(0.08))
        .when_some(column, |el, column| {
            el.relative().justify_center().px(px(column.gutter))
        })
        .child(
            comment_accent_bar(theme.solid.opacity(0.7)).when(column.is_some(), |el| {
                el.absolute().left(px(0.0)).top(px(0.0))
            }),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .px(px(Theme::SPACE_LG))
                .when_some(column, |el, column| {
                    el.max_w(px(column.max_width)).px(px(0.0))
                })
                .py(px(10.0))
                .child(
                    div()
                        .h(px(comments::CARD_HEADER_HEIGHT))
                        .flex_none()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            crate::icons::icon(crate::icons::CHAT_ROUND_LINE)
                                .size(px(12.0))
                                .text_color(theme.text_faint),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_family(theme.font_mono.clone())
                                .text_size(px(11.0))
                                .text_color(theme.text_faint)
                                .child(SharedString::from(format!("{path}:{line}"))),
                        ),
                )
                .child(
                    div()
                        .h(px(46.0))
                        .flex_none()
                        .overflow_hidden()
                        .text_size(px(12.0))
                        .child(input.into_any_element()),
                )
                .child(
                    div()
                        .h(px(28.0))
                        .flex_none()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_end()
                        .gap(px(6.0))
                        .child(
                            comment_action("cmt-cancel", "Cancel", false, theme)
                                .on_click(cx.listener(move |this, _, _, cx| cancel(this, cx))),
                        )
                        .child(
                            comment_action(
                                "cmt-commit",
                                if editing { "Save" } else { "Comment" },
                                true,
                                theme,
                            )
                            .on_click(cx.listener(move |this, _, _, cx| commit(this, cx))),
                        ),
                ),
        )
        .into_any_element()
}

fn comment_action(
    id: &'static str,
    label: &'static str,
    primary: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(22.0))
        .px(px(10.0))
        .flex()
        .items_center()
        .rounded(px(6.0))
        .text_size(px(11.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .cursor_pointer()
        .when(primary, |el| el.bg(theme.solid).text_color(theme.on_solid))
        .when(!primary, |el| {
            el.text_color(motion::hover_blend(id, theme.text_muted, theme.text))
                .bg(motion::hover_blend(
                    id,
                    gpui::transparent_black(),
                    theme.element_hover,
                ))
                .on_hover(motion::hover_listener(id))
        })
        .child(SharedString::from(label))
}
