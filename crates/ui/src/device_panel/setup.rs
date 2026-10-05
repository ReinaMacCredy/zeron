use gpui::{AnyElement, App, ClickEvent, SharedString, Window, div, prelude::*, px};

use crate::settings::widgets;
use crate::theme::Theme;

pub fn render_gate(
    theme: &Theme,
    pending: bool,
    on_enable: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .p(px(16.0))
        .child(
            div()
                .w_full()
                .max_w(px(320.0))
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(
                    div()
                        .text_size(crate::typography::ui_rems(14.0))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(theme.text)
                        .child("Set up devices"),
                )
                .child(
                    div()
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(
                            "Enable this environment to open iOS Simulators. Zeron installs the \
                             device hub once and runs it on this Mac only while it is enabled.",
                        )),
                )
                .child(
                    div()
                        .id("device-hub-enable")
                        .w_full()
                        .h(px(44.0))
                        .px(px(14.0))
                        .rounded(px(10.0))
                        .border_1()
                        .border_color(theme.border)
                        .bg(crate::theme::ink(0.02))
                        .flex()
                        .items_center()
                        .justify_between()
                        .cursor_pointer()
                        .child(
                            div()
                                .text_size(crate::typography::ui_rems(13.0))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(theme.text)
                                .child("Enable device hub"),
                        )
                        .child(widgets::toggle_switch(theme, pending, "device-hub-enable"))
                        .when(!pending, |row| row.on_click(on_enable)),
                ),
        )
        .into_any_element()
}
