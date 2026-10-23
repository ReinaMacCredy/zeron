use gpui::{AnyElement, Context, SharedString, div, prelude::*, px};

use super::Composer;
use crate::comments::ReviewComment;
use crate::popover;
use crate::theme::Theme;

impl Composer {
    pub(super) fn render_comments_badge(
        &self,
        comments: &[ReviewComment],
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let badge = crate::badges::render(
            "composer-comments",
            &crate::badges::MessageBadge {
                kind: crate::badges::BadgeKind::Comments,
                label: zeron_proto::badges::comment_label(comments.len()),
                details: Vec::new(),
            },
            theme,
        )
        .cursor_pointer()
        .role(gpui::Role::Button)
        .aria_label(format!(
            "{}, show list",
            zeron_proto::badges::comment_label(comments.len())
        ))
        .hover(|s| s.bg(crate::theme::ink(0.10)))
        .on_mouse_down(
            gpui::MouseButton::Left,
            cx.listener(|this, _, _, _| this.comments_menu.note_trigger_press()),
        )
        .on_click(cx.listener(|this, _, _, cx| {
            cx.stop_propagation();
            if this.comments_menu.take_press_was_open() {
                this.close_comments_menu(cx);
            } else {
                this.comments_menu.open(());
                cx.notify();
            }
        }));
        let menu = self
            .comments_menu
            .get()
            .is_some()
            .then(|| self.render_comments_menu(comments, theme, cx));
        div()
            .relative()
            .flex_none()
            .child(badge)
            .children(menu)
            .into_any_element()
    }

    pub(super) fn comments_menu_open(&self) -> bool {
        self.comments_menu.is_open()
    }

    fn close_comments_menu(&mut self, cx: &mut Context<Self>) {
        if self.comments_menu.begin_close() {
            popover::reap_popup(cx, |composer: &mut Self| &mut composer.comments_menu);
            cx.notify();
        }
    }

    fn remove_staged_comment(&mut self, id: &str, cx: &mut Context<Self>) {
        let key = self.current_key.clone();
        let emptied = self.state.update(cx, |state, cx| {
            state.remove_review_comment(&key, id);
            cx.notify();
            state.review_comments(&key).is_empty()
        });
        if emptied {
            self.close_comments_menu(cx);
        }
    }

    fn clear_staged_comments(&mut self, cx: &mut Context<Self>) {
        let key = self.current_key.clone();
        self.state.update(cx, |state, cx| {
            state.purge_review_comments(&key);
            cx.notify();
        });
        self.close_comments_menu(cx);
    }

    fn render_comments_menu(
        &self,
        comments: &[ReviewComment],
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let popup = theme.for_popup();
        let rows = comments.iter().map(|comment| {
            let (severity, title, detail) = crate::review::split_finding(&comment.body);
            let text = if title.is_empty() { detail } else { title };
            let text = text.lines().next().unwrap_or_default().to_string();
            let file = comment
                .cite_path()
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_string();
            let id = comment.id.clone();
            let row_id = SharedString::from(format!("composer-comment-{}", comment.id));
            popover::menu_row(&popup, false, row_id.clone())
                .id(row_id)
                .gap(px(8.0))
                .child(
                    div()
                        .flex_none()
                        .size(px(6.0))
                        .rounded_full()
                        .when_some(severity, |el, severity| {
                            el.bg(crate::review::severity_colors(severity, &popup).0)
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .when(severity.is_none(), |el| el.text_color(popup.text_muted))
                        .child(text.clone()),
                )
                .child(
                    div()
                        .flex_none()
                        .font_family(popup.font_mono.clone())
                        .text_size(crate::typography::ui_rems(10.5))
                        .text_color(popup.text_faint)
                        .child(format!("{file}:{}", comment.line)),
                )
                .child(
                    div()
                        .id(SharedString::from(format!(
                            "composer-comment-remove-{}",
                            comment.id
                        )))
                        .flex_none()
                        .size(px(18.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.0))
                        .cursor_pointer()
                        .role(gpui::Role::Button)
                        .aria_label(format!("Remove comment {text}"))
                        .hover(|s| s.bg(popup.glass_hover()))
                        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.remove_staged_comment(&id, cx);
                        }))
                        .child(
                            crate::icons::icon(crate::icons::CLOSE_CIRCLE)
                                .size(px(12.0))
                                .text_color(popup.text_muted),
                        ),
                )
        });
        let clear = popover::menu_row(&popup, false, "composer-comments-clear")
            .id("composer-comments-clear")
            .role(gpui::Role::MenuItem)
            .aria_label("Clear all comments")
            .text_color(popup.text_muted)
            .child("Clear all")
            .on_click(cx.listener(|this, _, _, cx| this.clear_staged_comments(cx)));
        let menu = popover::popover_card(&popup)
            .id("composer-comments-popup-card")
            .w(px(340.0))
            .p(px(4.0))
            .flex()
            .flex_col()
            .gap(px(popover::MENU_GAP))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_comments_menu(cx)))
            .children(rows)
            .child(popover::menu_separator())
            .child(clear)
            .into_any_element();
        popover::anchored_menu_above(
            "composer-comments-popup",
            menu,
            self.comments_menu.closing_since(),
        )
    }
}
