use gpui::{Context, Window, div, prelude::*, px};

use crate::icons::{self, icon};
use crate::theme::Theme;

pub struct GitTab;

impl Render for GitTab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        div()
            .id("pull-requests-git")
            .role(gpui::Role::Group)
            .aria_label("Git")
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(12.0))
            .px(px(16.0))
            .child(
                div()
                    .size(px(36.0))
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(theme.hairline(0.09))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        icon(icons::LUCIDE_GIT_BRANCH)
                            .size(px(16.0))
                            .text_color(theme.text),
                    ),
            )
            .child(
                div()
                    .text_size(px(14.0))
                    .text_color(theme.text_muted)
                    .child("The commit graph and working tree of a project appear here."),
            )
    }
}
