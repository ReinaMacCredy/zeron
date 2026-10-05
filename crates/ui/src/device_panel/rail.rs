use gpui::{AnyElement, Context, Entity, MouseButton, Stateful, div, prelude::*, px};
use zeron_proto::device_hub::DeviceButton;

use super::geometry::RAIL_WIDTH;
use super::stream_view::DeviceStreamView;
use super::{DevicePanel, DevicePanelEvent};
use crate::icons::{self, icon};
use crate::popover;
use crate::theme::Theme;

fn rail_button(
    id: &'static str,
    glyph: &'static str,
    label: &'static str,
    enabled: bool,
    theme: &Theme,
) -> Stateful<gpui::Div> {
    let button = div()
        .id(id)
        .debug_selector(move || id.into())
        .role(gpui::Role::Button)
        .aria_label(label)
        .size(px(28.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(8.0))
        .tooltip(crate::settings::widgets::text_tooltip(label))
        .child(icon(glyph).size(px(16.0)).text_color(theme.text_muted));
    if enabled {
        button
            .cursor_pointer()
            .hover(|s| s.bg(crate::theme::ink(0.06)))
    } else {
        button.opacity(0.4).cursor_default()
    }
}

fn divider(theme: &Theme) -> gpui::Div {
    div()
        .my(px(4.0))
        .h(px(1.0))
        .w(px(20.0))
        .flex_none()
        .bg(theme.border.opacity(0.7))
}

impl DevicePanel {
    pub(super) fn render_rail(
        &self,
        view: &Entity<DeviceStreamView>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let input_live = view.read(cx).input_live();
        let (home, rotate) = (view.clone(), view.clone());
        div()
            .absolute()
            .top_0()
            .bottom_0()
            .right_0()
            .w(px(RAIL_WIDTH))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .py(px(12.0))
            .pr(px(8.0))
            .child(
                div()
                    .id("device-rail")
                    .occlude()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(4.0))
                    .p(px(8.0))
                    .rounded_full()
                    .border_1()
                    .border_color(theme.border.opacity(0.5))
                    .bg(theme.bg.opacity(0.8))
                    .child(
                        rail_button("device-home", icons::HOME, "Home", input_live, theme).when(
                            input_live,
                            |button| {
                                button.on_click(move |_, _, cx| {
                                    home.update(cx, |view, _| view.press_button(DeviceButton::Home))
                                })
                            },
                        ),
                    )
                    .child(
                        rail_button(
                            "device-rotate",
                            icons::LUCIDE_ROTATE_CCW,
                            "Rotate device",
                            input_live,
                            theme,
                        )
                        .when(input_live, |button| {
                            button.on_click(move |_, _, cx| {
                                rotate.update(cx, |view, _| view.rotate())
                            })
                        }),
                    )
                    .child(divider(theme))
                    .child(self.render_more(theme, cx)),
            )
            .into_any_element()
    }

    fn render_more(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let trigger = rail_button(
            "device-more",
            icons::LUCIDE_ELLIPSIS,
            "More device actions",
            true,
            theme,
        )
        .relative()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|panel, _, _, _| panel.more.note_trigger_press()),
        )
        .on_click(cx.listener(|panel, _, _, cx| {
            if panel.more.take_press_was_open() {
                panel.close_more(cx);
            } else {
                panel.more.open(());
                cx.notify();
            }
        }));
        if self.more.get().is_none() {
            return trigger.into_any_element();
        }
        let popup = theme.for_popup();
        let row = |id: &'static str, glyph: &'static str, label: &'static str, danger: bool| {
            let color = if danger {
                popup.danger
            } else {
                popup.text_muted
            };
            popover::menu_row(&popup, false, id)
                .id(id)
                .debug_selector(move || id.into())
                .role(gpui::Role::MenuItem)
                .aria_label(label)
                .when(danger, |row| row.text_color(popup.danger))
                .child(icon(glyph).size(px(14.0)).text_color(color))
                .child(label)
        };
        let menu = popover::popover_card(&popup)
            .bg(popup.surface_dialog)
            .min_w(px(200.0))
            .flex()
            .flex_col()
            .gap(px(popover::MENU_GAP))
            .on_mouse_down_out(cx.listener(|panel, _, _, cx| panel.close_more(cx)))
            .child(
                row(
                    "device-close-panel",
                    icons::LUCIDE_X,
                    "Close device panel",
                    false,
                )
                .on_click(cx.listener(|panel, _, _, cx| {
                    panel.close_more(cx);
                    cx.emit(DevicePanelEvent::Close);
                })),
            )
            .child(popover::menu_separator())
            .child(
                row(
                    "device-power-off",
                    icons::LUCIDE_POWER,
                    "Power off device",
                    true,
                )
                .on_click(cx.listener(|panel, _, _, cx| {
                    panel.close_more(cx);
                    panel.power_off(cx);
                })),
            )
            .into_any_element();
        trigger
            .child(popover::anchored_menu_below_end(
                "device-more-menu",
                menu,
                self.more.closing_since(),
            ))
            .into_any_element()
    }

    fn close_more(&mut self, cx: &mut Context<Self>) {
        if self.more.begin_close() {
            popover::reap_popup(cx, |panel: &mut Self| &mut panel.more);
        }
        cx.notify();
    }
}
