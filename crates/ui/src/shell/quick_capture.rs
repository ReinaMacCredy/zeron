use super::*;
use crate::quick_capture::{
    self as qc, BarEvent, Capture, CaptureToast, QuickCaptureBar, ToastPanel,
};
use crate::typography::ui_rems;
use gpui::KeyDownEvent;

#[derive(Default)]
pub(super) struct Host {
    bar: Option<Entity<QuickCaptureBar>>,
    bar_window: Option<gpui::WindowHandle<QuickCaptureBar>>,
    toast_window: Option<gpui::WindowHandle<ToastPanel>>,
    previous_app: Option<i32>,
    shell_window: Option<gpui::AnyWindowHandle>,
    toast: Option<CaptureToast>,
    rail: popover::Popup<()>,
    pub(super) rail_input: Option<Entity<ComposerInput>>,
    rail_added: Vec<SharedString>,
    subscriptions: Vec<Subscription>,
}

impl Shell {
    pub(crate) fn toggle_quick_capture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(handle) = self.quick_capture.bar_window {
            if handle.update(cx, |_, _, _| ()).is_ok() {
                self.dismiss_quick_capture(cx);
                return;
            }
            self.quick_capture.bar_window = None;
        }
        self.quick_capture.shell_window = Some(window.window_handle());
        self.close_capture_toast_window(cx);
        #[cfg(target_os = "macos")]
        {
            self.quick_capture.previous_app =
                qc::macos::frontmost_pid().filter(|pid| *pid != std::process::id() as i32);
        }
        let bar = match &self.quick_capture.bar {
            Some(bar) => bar.clone(),
            None => {
                let state = self.state.clone();
                let bar = cx.new(|cx| QuickCaptureBar::new(state, cx));
                let events = cx.subscribe(&bar, Self::on_quick_capture_event);
                self.quick_capture.subscriptions.push(events);
                self.quick_capture.bar = Some(bar.clone());
                bar
            }
        };
        let size = gpui::size(px(qc::CARD_WIDTH), px(qc::BAR_HEIGHT));
        let Some(options) = panel_options(size, true, cx) else {
            return;
        };
        let opened = cx.open_window(options, |window, cx| {
            #[cfg(target_os = "macos")]
            qc::macos::style_panel(window, 16.0);
            bar.update(cx, |bar, cx| bar.opened(window, cx));
            bar.clone()
        });
        let Ok(handle) = opened else {
            return;
        };
        self.quick_capture.bar_window = Some(handle);
    }

    fn dismiss_quick_capture(&mut self, cx: &mut Context<Self>) {
        self.close_bar_window(cx);
        self.return_focus(cx);
    }

    fn close_bar_window(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.quick_capture.bar_window.take() {
            cx.defer(move |cx| {
                handle
                    .update(cx, |_, window, _| window.remove_window())
                    .ok();
            });
        }
    }

    fn return_focus(&mut self, _cx: &mut Context<Self>) {
        #[cfg(target_os = "macos")]
        if let Some(pid) = self.quick_capture.previous_app {
            qc::macos::activate(pid);
        }
    }

    fn on_quick_capture_event(
        &mut self,
        _bar: Entity<QuickCaptureBar>,
        event: &BarEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            BarEvent::Dismiss => self.dismiss_quick_capture(cx),
            BarEvent::Blurred => {
                self.quick_capture.previous_app = None;
                self.close_bar_window(cx);
            }
            BarEvent::Saved { capture, open } => {
                self.file_capture(capture, cx);
                let (capture, open) = (capture.clone(), *open);
                cx.spawn(async move |shell, cx| {
                    cx.background_executor().timer(qc::TICK + qc::FADE).await;
                    shell
                        .update(cx, |shell, cx| shell.after_capture(capture, open, cx))
                        .ok();
                })
                .detach();
            }
        }
    }

    fn file_capture(&mut self, capture: &Capture, cx: &mut Context<Self>) {
        self.create_captured_task(capture, cx);
        let on_tasks = matches!(self.route, Route::Tasks);
        self.state.update(cx, |state, cx| {
            state.quick_captured.record(&capture.task_id, on_tasks);
            cx.notify();
        });
    }

    fn after_capture(&mut self, capture: Capture, open: bool, cx: &mut Context<Self>) {
        self.close_bar_window(cx);
        let toast = CaptureToast {
            task_id: capture.task_id.clone(),
            title: capture.title.clone().into(),
            shown: motion::now(),
        };
        if open {
            self.quick_capture.previous_app = None;
            crate::activate_main_window(cx);
            let Some(shell_window) = self.quick_capture.shell_window else {
                return;
            };
            let shell = cx.weak_entity();
            cx.defer(move |cx| {
                shell_window
                    .update(cx, |_, window, cx| {
                        window.activate_window();
                        shell
                            .update(cx, |shell, cx| {
                                shell.open_captured_task(capture.task_id, capture.title, window, cx)
                            })
                            .ok();
                    })
                    .ok();
            });
            return;
        }
        match self.quick_capture.previous_app {
            Some(_) => {
                self.return_focus(cx);
                self.open_capture_toast_window(toast, cx);
            }
            None => self.show_capture_toast(toast, cx),
        }
    }

    fn show_capture_toast(&mut self, toast: CaptureToast, cx: &mut Context<Self>) {
        let shown = toast.shown;
        self.quick_capture.toast = Some(toast);
        cx.notify();
        cx.spawn(async move |shell, cx| {
            cx.background_executor().timer(qc::TOAST_FOR).await;
            shell
                .update(cx, |shell, cx| {
                    if shell.quick_capture.toast.as_ref().map(|t| t.shown) == Some(shown) {
                        shell.quick_capture.toast = None;
                        cx.notify();
                    }
                })
                .ok();
        })
        .detach();
    }

    fn open_capture_toast_window(&mut self, toast: CaptureToast, cx: &mut Context<Self>) {
        let size = gpui::size(px(qc::TOAST_WIDTH), px(qc::TOAST_HEIGHT));
        let Some(options) = panel_options(size, false, cx) else {
            return;
        };
        let shell = cx.weak_entity();
        let (undo_shell, open_shell) = (shell.clone(), shell.clone());
        let (undo_id, open_id, title) = (
            toast.task_id.clone(),
            toast.task_id.clone(),
            toast.title.to_string(),
        );
        let panel = ToastPanel {
            toast,
            on_undo: std::rc::Rc::new(move |_, cx| {
                undo_shell
                    .update(cx, |shell, cx| shell.undo_capture(&undo_id, cx))
                    .ok();
            }),
            on_open: std::rc::Rc::new(move |_, cx| {
                let (task_id, title) = (open_id.clone(), title.clone());
                open_shell
                    .update(cx, |shell, cx| shell.open_capture(task_id, title, cx))
                    .ok();
            }),
        };
        let opened = cx.open_window(options, |window, cx| {
            #[cfg(target_os = "macos")]
            qc::macos::style_panel(window, 10.0);
            cx.new(|_| panel)
        });
        let Ok(handle) = opened else {
            return;
        };
        self.quick_capture.toast_window = Some(handle);
        cx.spawn(async move |shell, cx| {
            cx.background_executor().timer(qc::TOAST_FOR).await;
            shell
                .update(cx, |shell, cx| {
                    if shell.quick_capture.toast_window == Some(handle) {
                        shell.close_capture_toast_window(cx);
                    }
                })
                .ok();
        })
        .detach();
    }

    fn close_capture_toast_window(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.quick_capture.toast_window.take() {
            cx.defer(move |cx| {
                handle
                    .update(cx, |_, window, _| window.remove_window())
                    .ok();
            });
        }
    }

    fn undo_capture(&mut self, task_id: &str, cx: &mut Context<Self>) {
        self.delete_task(task_id, cx);
        self.state.update(cx, |state, cx| {
            state.quick_captured.forget(task_id);
            cx.notify();
        });
        self.quick_capture.rail_added.clear();
        self.quick_capture.toast = None;
        self.close_capture_toast_window(cx);
        cx.notify();
    }

    fn open_capture(&mut self, task_id: String, title: String, cx: &mut Context<Self>) {
        self.quick_capture.toast = None;
        self.close_capture_toast_window(cx);
        crate::activate_main_window(cx);
        let Some(shell_window) = self.quick_capture.shell_window else {
            return;
        };
        let shell = cx.weak_entity();
        cx.defer(move |cx| {
            shell_window
                .update(cx, |_, window, cx| {
                    window.activate_window();
                    shell
                        .update(cx, |shell, cx| {
                            shell.open_captured_task(task_id, title, window, cx)
                        })
                        .ok();
                })
                .ok();
        });
    }

    pub(super) fn quick_capture_tasks_opened(&mut self, cx: &mut Context<Self>) {
        self.state
            .update(cx, |state, _| state.quick_captured.tasks_opened());
    }

    pub(super) fn quick_capture_unseen(&self, cx: &App) -> usize {
        let state = self.state.read(cx);
        state
            .quick_captured
            .unseen_count(|id| state.tasks.iter().any(|task| task.id == id))
    }

    pub(super) fn render_capture_toast(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let toast = self.quick_capture.toast.clone()?;
        let theme = Theme::of(cx).for_popup();
        let (undo_id, open_id, title) = (
            toast.task_id.clone(),
            toast.task_id.clone(),
            toast.title.to_string(),
        );
        let undo_shell = cx.weak_entity();
        let open_shell = cx.weak_entity();
        let card = qc::toast_card(
            toast.title.clone(),
            &theme,
            move |_, _, cx| {
                undo_shell
                    .update(cx, |shell, cx| shell.undo_capture(&undo_id, cx))
                    .ok();
            },
            move |_, window, cx| {
                let (task_id, title) = (open_id.clone(), title.clone());
                open_shell
                    .update(cx, |shell, cx| {
                        shell.quick_capture.toast = None;
                        shell.open_captured_task(task_id, title, window, cx)
                    })
                    .ok();
            },
        )
        .occlude();
        Some(
            div()
                .absolute()
                .left(px(self.content_left()))
                .right_0()
                .bottom(px(132.0))
                .flex()
                .justify_center()
                .child(motion::fade_in(
                    SharedString::from(format!("capture-toast-{}", toast.task_id)),
                    card,
                ))
                .into_any_element(),
        )
    }

    pub(super) fn render_rail_capture(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let popup_theme = theme.for_popup();
        let open = self.quick_capture.rail.is_open();
        let wash = self.rail_wash(&theme);
        let mut trigger = app_rail::button(
            "rail-quick-capture",
            "Quick capture to Tasks",
            icons::PLUS,
            open,
            wash,
            &theme,
        )
        .aria_expanded(open)
        .relative()
        .tooltip(crate::settings::widgets::text_tooltip(
            "Quick capture to Tasks",
        ))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, _| this.quick_capture.rail.note_trigger_press()),
        )
        .on_click(cx.listener(|this, _, window, cx| this.toggle_rail_capture(window, cx)));
        if self.quick_capture.rail.get().is_some()
            && let Some(input) = self.quick_capture.rail_input.clone()
        {
            let state = self.state.read(cx);
            let project = qc::project_label(
                state,
                state.selected_space_row().map(|space| space.id.as_str()),
            );
            let added = self.quick_capture.rail_added.clone();
            let card = div()
                .id("rail-quick-capture-popover")
                .debug_selector(|| "rail-quick-capture-popover".into())
                .w(px(360.0))
                .p(px(10.0))
                .flex()
                .flex_col()
                .rounded(px(12.0))
                .border_1()
                .border_color(popup_theme.border)
                .bg(popover::surface_bg(&popup_theme))
                .shadow_lg()
                .text_color(popup_theme.text)
                .text_size(ui_rems(13.0))
                .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_rail_capture(cx)))
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    if event.keystroke.key == "escape" {
                        cx.stop_propagation();
                        this.close_rail_capture(cx);
                    }
                }))
                .child(
                    div()
                        .h(px(32.0))
                        .px(px(10.0))
                        .flex()
                        .items_center()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(popup_theme.accent.opacity(0.7))
                        .bg(popup_theme.text.opacity(0.04))
                        .child(div().flex_1().min_w_0().child(input)),
                )
                .child(
                    div()
                        .mt(px(8.0))
                        .flex()
                        .justify_between()
                        .text_size(ui_rems(11.5))
                        .text_color(popup_theme.text_faint)
                        .child(div().truncate().child(format!("To do · {project}")))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.0))
                                .child(
                                    div()
                                        .px(px(5.0))
                                        .rounded(px(4.0))
                                        .bg(popup_theme.text.opacity(0.10))
                                        .text_color(popup_theme.text_muted)
                                        .child("Esc"),
                                )
                                .child("close"),
                        ),
                )
                .when(!added.is_empty(), |el| {
                    el.child(
                        div()
                            .mt(px(8.0))
                            .flex()
                            .flex_col()
                            .text_size(ui_rems(12.0))
                            .text_color(popup_theme.text_muted)
                            .children(added.into_iter().rev().map(|title| {
                                div()
                                    .py(px(4.0))
                                    .px(px(2.0))
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0))
                                    .child(
                                        icon(icons::CHECK)
                                            .size(px(12.0))
                                            .flex_none()
                                            .text_color(popup_theme.success),
                                    )
                                    .child(div().truncate().child(title))
                            })),
                    )
                })
                .into_any_element();
            trigger = trigger.child(popover::anchored_menu_right(
                "rail-quick-capture-menu",
                card,
                self.quick_capture.rail.closing_since(),
            ));
        }
        trigger.into_any_element()
    }

    fn toggle_rail_capture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.quick_capture.rail.take_press_was_open() || self.quick_capture.rail.is_open() {
            self.close_rail_capture(cx);
            return;
        }
        let input = match &self.quick_capture.rail_input {
            Some(input) => input.clone(),
            None => {
                let input = cx.new(|cx| {
                    ComposerInput::new("New idea…  Enter to save", cx)
                        .with_single_line()
                        .with_max_chars(zeron_doc::TASK_TITLE_MAX_CHARS)
                });
                let events = cx.subscribe(&input, |this, input, event: &ComposerInputEvent, cx| {
                    if matches!(event, ComposerInputEvent::Submitted) {
                        this.submit_rail_capture(&input, cx);
                    }
                });
                self.quick_capture.subscriptions.push(events);
                self.quick_capture.rail_input = Some(input.clone());
                input
            }
        };
        self.quick_capture.rail_added.clear();
        self.quick_capture.rail.open(());
        window.focus(&input.focus_handle(cx), cx);
        cx.notify();
    }

    fn submit_rail_capture(&mut self, input: &Entity<ComposerInput>, cx: &mut Context<Self>) {
        let Some((title, _)) = qc::split_capture(input.read(cx).text()) else {
            return;
        };
        let capture = Capture {
            task_id: uuid::Uuid::new_v4().to_string(),
            title: title.clone(),
            notes: String::new(),
            space_id: self
                .state
                .read(cx)
                .selected_space_row()
                .map(|space| space.id.clone()),
            due_on: None,
            images: Vec::new(),
        };
        input.update(cx, |input, cx| input.set_text("", cx));
        self.file_capture(&capture, cx);
        self.quick_capture.rail_added.push(title.into());
        cx.notify();
    }

    pub(super) fn close_rail_capture(&mut self, cx: &mut Context<Self>) {
        if self.quick_capture.rail.begin_close() {
            popover::reap_popup(cx, |shell: &mut Self| &mut shell.quick_capture.rail);
            cx.notify();
        }
    }

    pub(super) fn rail_capture_open(&self) -> bool {
        self.quick_capture.rail.is_open()
    }
}

fn panel_options(size: gpui::Size<Pixels>, focus: bool, cx: &App) -> Option<gpui::WindowOptions> {
    #[cfg(target_os = "macos")]
    let wanted = qc::macos::active_display().map(gpui::DisplayId::new);
    #[cfg(not(target_os = "macos"))]
    let wanted: Option<gpui::DisplayId> = None;
    let display = wanted
        .and_then(|id| cx.displays().into_iter().find(|display| display.id() == id))
        .or_else(|| cx.primary_display())?;
    let screen = display.bounds().size;
    let origin = gpui::point(
        (screen.width - size.width) / 2.0,
        screen.height * qc::FROM_TOP,
    );
    Some(gpui::WindowOptions {
        window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds::new(
            origin, size,
        ))),
        titlebar: None,
        focus,
        show: true,
        kind: gpui::WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        display_id: Some(display.id()),
        window_background: gpui::WindowBackgroundAppearance::Blurred,
        ..Default::default()
    })
}
