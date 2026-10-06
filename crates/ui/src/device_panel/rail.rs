use gpui::{AnyElement, Context, Entity, MouseButton, SharedString, Stateful, div, prelude::*, px};
use zeron_proto::device_hub::{
    DeviceAction, DeviceAppearance, DeviceButton, DeviceOrientation, DevicePlatform, FoldPosture,
};

use super::fold;
use super::geometry::RAIL_WIDTH;
use super::stream_view::DeviceStreamView;
use super::tools::TEXT_SIZES;
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
    tipped_button(id, glyph, label, label, enabled, theme)
}

fn tipped_button(
    id: &'static str,
    glyph: &'static str,
    label: &'static str,
    tip: &'static str,
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
        .tooltip(crate::settings::widgets::text_tooltip(tip))
        .child(icon(glyph).size(px(16.0)).text_color(theme.text_muted));
    if enabled {
        button
            .cursor_pointer()
            .hover(|s| s.bg(crate::theme::ink(0.06)))
    } else {
        button.opacity(0.4).cursor_default()
    }
}

fn pressed(button: Stateful<gpui::Div>, on: bool) -> Stateful<gpui::Div> {
    button
        .aria_toggled(if on {
            gpui::Toggled::True
        } else {
            gpui::Toggled::False
        })
        .when(on, |button| button.bg(crate::theme::ink(0.08)))
}

fn pill(id: &'static str, label: &'static str, theme: &Theme) -> Stateful<gpui::Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .role(gpui::Role::Group)
        .aria_label(label)
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
        let android = view.read(cx).platform() == DevicePlatform::Android;
        let (home, rotate) = (view.clone(), view.clone());
        let press = |id: &'static str, glyph, label, button: DeviceButton| {
            let view = view.clone();
            rail_button(id, glyph, label, input_live, theme).when(input_live, |el| {
                el.on_click(move |_, _, cx| view.update(cx, |view, _| view.press_button(button)))
            })
        };
        let turn = if android {
            vec![
                press(
                    "device-back",
                    icons::LUCIDE_CHEVRON_LEFT,
                    "Back",
                    DeviceButton::Back,
                )
                .into_any_element(),
                press(
                    "device-recents",
                    icons::LUCIDE_SQUARE,
                    "Recents",
                    DeviceButton::AppSwitcher,
                )
                .into_any_element(),
                self.render_rotate_menu(theme, cx),
            ]
        } else {
            let hinge = self.fold().cloned();
            vec![
                rail_button(
                    "device-rotate",
                    icons::LUCIDE_ROTATE_CCW,
                    "Rotate device",
                    input_live,
                    theme,
                )
                .when(input_live, |button| {
                    button.on_click(move |_, _, cx| {
                        let (hinged, current) = {
                            let screen = rotate.read(cx).screen();
                            (
                                fold::hinged(screen),
                                screen.map_or(DeviceOrientation::Portrait, |screen| {
                                    screen.orientation
                                }),
                            )
                        };
                        match hinge.clone().filter(|_| hinged) {
                            Some(hinge) => hinge.update(cx, |hinge, cx| hinge.rotate(current, cx)),
                            None => rotate.update(cx, |view, _| view.rotate()),
                        }
                    })
                })
                .into_any_element(),
            ]
        };
        div()
            .absolute()
            .top_0()
            .bottom_0()
            .right_0()
            .w(px(RAIL_WIDTH))
            .flex()
            .flex_col()
            .flex_wrap_reverse()
            .content_start()
            .items_center()
            .justify_center()
            .gap(px(8.0))
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
                    .children(turn)
                    .child(divider(theme))
                    .children(self.render_settings_buttons(theme, cx))
                    .child(self.render_more(theme, cx))
                    .child(divider(theme))
                    .children(self.render_presentation(view, theme, cx)),
            )
            .children(self.render_fold(view, theme, cx))
            .into_any_element()
    }

    fn render_presentation(
        &self,
        view: &Entity<DeviceStreamView>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let (streaming, shown) = {
            let view = view.read(cx);
            (view.has_frame(), view.three_d_shown())
        };
        let blocked = self.three_d_blocked(cx);
        let can_3d = streaming && blocked.is_none();
        let mut buttons = vec![
            pressed(
                tipped_button(
                    "device-view-3d",
                    icons::LUCIDE_BOX,
                    "3D view",
                    blocked.unwrap_or("3D view"),
                    can_3d,
                    theme,
                ),
                shown,
            )
            .when(can_3d, |button| {
                button.on_click(cx.listener(|panel, _, _, cx| panel.set_three_d(true, cx)))
            })
            .into_any_element(),
            pressed(
                rail_button(
                    "device-view-flat",
                    icons::SMARTPHONE,
                    "Flat view",
                    streaming,
                    theme,
                ),
                !shown,
            )
            .when(streaming, |button| {
                button.on_click(cx.listener(|panel, _, _, cx| panel.set_three_d(false, cx)))
            })
            .into_any_element(),
        ];
        if shown && streaming {
            let ready = self.restore_ready(cx);
            buttons.push(
                rail_button(
                    "device-view-restore",
                    icons::LUCIDE_MAXIMIZE_2,
                    "Restore 3D view",
                    ready,
                    theme,
                )
                .when(ready, |button| {
                    button.on_click(cx.listener(|panel, _, _, cx| panel.restore_three_d(cx)))
                })
                .into_any_element(),
            );
        }
        buttons
    }

    fn render_fold(
        &self,
        view: &Entity<DeviceStreamView>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(controls) = self.fold().cloned() else {
            return Vec::new();
        };
        let (live, screen, android) = {
            let view = view.read(cx);
            (
                view.input_live(),
                view.screen().cloned(),
                view.platform() == DevicePlatform::Android,
            )
        };
        let (state, pending, error) = {
            let controls = controls.read(cx);
            (
                controls.android().cloned(),
                controls.android_pending(),
                controls.error().cloned(),
            )
        };
        let mut groups = Vec::new();
        if let Some(screen) = screen.filter(|screen| !android && fold::hinged(Some(screen))) {
            for (id, label, buttons) in [
                ("device-fold-shape", "Fold shape", &fold::FOLD_SHAPES[..]),
                ("device-stance", "Device stance", &fold::STANCES[..]),
            ] {
                let buttons = buttons.iter().map(|button| {
                    let (pose, controls) = (button.pose, controls.clone());
                    pressed(
                        tipped_button(
                            button.id,
                            button.glyph,
                            button.label,
                            button.tip,
                            live,
                            theme,
                        ),
                        fold::duo_selected(&screen, pose),
                    )
                    .when(live, |button| {
                        button.on_click(move |_, _, cx| {
                            controls.update(cx, |controls, cx| controls.set_pose(pose, cx))
                        })
                    })
                });
                groups.push(pill(id, label, theme).children(buttons).into_any_element());
            }
        }
        if let Some(state) = state.filter(|_| android) {
            let enabled = live && !pending;
            let buttons = [
                (
                    FoldPosture::Closed,
                    "device-fold",
                    "Fold device",
                    icons::DUO_CLOSED,
                ),
                (
                    FoldPosture::Opened,
                    "device-unfold",
                    "Unfold device",
                    icons::DUO_OPEN,
                ),
            ]
            .map(|(posture, id, label, glyph)| {
                let controls = controls.clone();
                pressed(
                    rail_button(id, glyph, label, enabled, theme),
                    state.posture == Some(posture),
                )
                .when(enabled, |button| {
                    button.on_click(move |_, _, cx| {
                        controls.update(cx, |controls, cx| controls.set_android(posture, cx))
                    })
                })
            });
            groups.push(
                pill("device-android-fold", "Android fold controls", theme)
                    .children(buttons)
                    .into_any_element(),
            );
        }
        if let Some(error) = error.filter(|_| !groups.is_empty()) {
            groups.push(
                div()
                    .id("device-fold-error")
                    .debug_selector(|| "device-fold-error".into())
                    .role(gpui::Role::Alert)
                    .aria_label(error.clone())
                    .tooltip(crate::settings::widgets::text_tooltip(error))
                    .px(px(6.0))
                    .text_size(crate::typography::ui_rems(12.0))
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(theme.danger)
                    .child("!")
                    .into_any_element(),
            );
        }
        groups
    }

    fn render_settings_buttons(&self, theme: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(controls) = self.controls().cloned() else {
            return Vec::new();
        };
        let (settings, ready, tools_open) = {
            let controls = controls.read(cx);
            (
                controls.settings().cloned(),
                !controls.disabled(),
                controls.drawer_open(),
            )
        };
        let appearance = settings.as_ref().and_then(|settings| settings.appearance);
        let next = match appearance {
            Some(DeviceAppearance::Dark) => DeviceAppearance::Light,
            _ => DeviceAppearance::Dark,
        };
        let (glyph, label) = match next {
            DeviceAppearance::Light => (icons::SUN, "Switch device to light mode"),
            DeviceAppearance::Dark => (icons::MOON, "Switch device to dark mode"),
        };
        let can_flip = ready && appearance.is_some();
        let flip = controls.clone();
        let tools = controls.clone();
        let shooting = self.screenshot_task.is_some();
        let streaming = self
            .view
            .as_ref()
            .is_some_and(|view| view.read(cx).has_frame());
        let can_shoot = streaming && !shooting;
        vec![
            rail_button("device-appearance", glyph, label, can_flip, theme)
                .when(can_flip, |button| {
                    button.on_click(move |_, _, cx| {
                        flip.update(cx, |controls, cx| {
                            controls.act(DeviceAction::SetAppearance { value: next }, cx)
                        })
                    })
                })
                .into_any_element(),
            self.render_text_size(
                &controls,
                ready && settings.as_ref().is_some_and(|s| s.text_size.is_some()),
                theme,
                cx,
            ),
            rail_button(
                "device-tools-toggle",
                icons::LUCIDE_SLIDERS_HORIZONTAL,
                "Device tools",
                true,
                theme,
            )
            .aria_toggled(if tools_open {
                gpui::Toggled::True
            } else {
                gpui::Toggled::False
            })
            .when(tools_open, |button| button.bg(crate::theme::ink(0.08)))
            .on_click(move |_, _, cx| {
                tools.update(cx, |controls, cx| controls.set_drawer(!tools_open, cx))
            })
            .into_any_element(),
            rail_button(
                "device-screenshot",
                icons::LUCIDE_CAMERA,
                if shooting {
                    "Capturing screenshot"
                } else {
                    "Save screenshot"
                },
                can_shoot,
                theme,
            )
            .when(can_shoot, |button| {
                button.on_click(cx.listener(|panel, _, _, cx| panel.save_screenshot(cx)))
            })
            .into_any_element(),
        ]
    }

    fn render_text_size(
        &self,
        controls: &Entity<super::tools::DeviceControls>,
        enabled: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let trigger = rail_button(
            "device-text-size",
            icons::LUCIDE_TYPE,
            "Device text size",
            enabled,
            theme,
        )
        .relative();
        if !enabled {
            return trigger.into_any_element();
        }
        let trigger = trigger
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|panel, _, _, _| panel.text_size.note_trigger_press()),
            )
            .on_click(cx.listener(|panel, _, _, cx| {
                if panel.text_size.take_press_was_open() {
                    panel.close_text_size(cx);
                } else {
                    panel.text_size.open(());
                    cx.notify();
                }
            }));
        if self.text_size.get().is_none() {
            return trigger.into_any_element();
        }
        let popup = theme.for_popup();
        let current = controls
            .read(cx)
            .settings()
            .and_then(|settings| settings.text_size);
        let rows = TEXT_SIZES
            .into_iter()
            .enumerate()
            .map(|(i, (value, label))| {
                let id = SharedString::from(format!("device-text-size-{i}"));
                let selector = id.clone();
                let selected = current == Some(value);
                let controls = controls.clone();
                popover::menu_row(&popup, selected, id.clone())
                    .id(id)
                    .debug_selector(move || selector.to_string())
                    .role(gpui::Role::MenuItemRadio)
                    .aria_label(label)
                    .aria_toggled(if selected {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .child(div().flex_1().child(label))
                    .when(selected, |row| {
                        row.child(icon(icons::CHECK).size(px(14.0)).text_color(popup.text))
                    })
                    .on_click(cx.listener(move |panel, _, _, cx| {
                        panel.close_text_size(cx);
                        controls.update(cx, |controls, cx| {
                            controls.act(DeviceAction::SetTextSize { value }, cx)
                        });
                    }))
            });
        let menu = popover::popover_card(&popup)
            .bg(popup.surface_dialog)
            .min_w(px(160.0))
            .flex()
            .flex_col()
            .gap(px(popover::MENU_GAP))
            .on_mouse_down_out(cx.listener(|panel, _, _, cx| panel.close_text_size(cx)))
            .children(rows)
            .into_any_element();
        trigger
            .child(popover::anchored_menu_below_end(
                "device-text-size-menu",
                menu,
                self.text_size.closing_since(),
            ))
            .into_any_element()
    }

    fn render_rotate_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let controls = self.controls().cloned();
        let enabled = controls
            .as_ref()
            .is_some_and(|controls| !controls.read(cx).disabled());
        let trigger = rail_button(
            "device-rotate",
            icons::LUCIDE_ROTATE_CCW,
            "Rotate device",
            enabled,
            theme,
        )
        .relative();
        let Some(controls) = controls.filter(|_| enabled) else {
            return trigger.into_any_element();
        };
        let trigger = trigger
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|panel, _, _, _| panel.rotate.note_trigger_press()),
            )
            .on_click(cx.listener(|panel, _, _, cx| {
                if panel.rotate.take_press_was_open() {
                    panel.close_rotate(cx);
                } else {
                    panel.rotate.open(());
                    cx.notify();
                }
            }));
        if self.rotate.get().is_none() {
            return trigger.into_any_element();
        }
        let popup = theme.for_popup();
        let rows = [
            (
                "device-rotate-portrait",
                "Portrait",
                DeviceOrientation::Portrait,
            ),
            (
                "device-rotate-landscape",
                "Landscape",
                DeviceOrientation::LandscapeLeft,
            ),
        ]
        .map(|(id, label, value)| {
            let controls = controls.clone();
            popover::menu_row(&popup, false, id)
                .id(id)
                .debug_selector(move || id.into())
                .role(gpui::Role::MenuItem)
                .aria_label(label)
                .child(label)
                .on_click(cx.listener(move |panel, _, _, cx| {
                    panel.close_rotate(cx);
                    controls.update(cx, |controls, cx| {
                        controls.act(DeviceAction::SetOrientation { value }, cx)
                    });
                }))
        });
        let menu = popover::popover_card(&popup)
            .bg(popup.surface_dialog)
            .min_w(px(160.0))
            .flex()
            .flex_col()
            .gap(px(popover::MENU_GAP))
            .on_mouse_down_out(cx.listener(|panel, _, _, cx| panel.close_rotate(cx)))
            .children(rows)
            .into_any_element();
        trigger
            .child(popover::anchored_menu_below_end(
                "device-rotate-menu",
                menu,
                self.rotate.closing_since(),
            ))
            .into_any_element()
    }

    fn close_rotate(&mut self, cx: &mut Context<Self>) {
        if self.rotate.begin_close() {
            popover::reap_popup(cx, |panel: &mut Self| &mut panel.rotate);
        }
        cx.notify();
    }

    fn close_text_size(&mut self, cx: &mut Context<Self>) {
        if self.text_size.begin_close() {
            popover::reap_popup(cx, |panel: &mut Self| &mut panel.text_size);
        }
        cx.notify();
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
        let physical = self.device(cx).is_some_and(|device| device.physical);
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
                    "device-float",
                    icons::LUCIDE_PICTURE_IN_PICTURE_2,
                    "Float device over chat",
                    false,
                )
                .on_click(cx.listener(|panel, _, _, cx| {
                    panel.close_more(cx);
                    cx.emit(DevicePanelEvent::Float);
                })),
            )
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
            .when(!physical, |menu| {
                menu.child(popover::menu_separator()).child(
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
            })
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
