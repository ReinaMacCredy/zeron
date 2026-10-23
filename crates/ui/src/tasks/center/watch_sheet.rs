use super::*;
use crate::supervisor::{SupervisorPage, Watch, ok_tone, tree_watch};
use zeron_proto::supervisor::SupervisorSettings;
use zeron_rpc::methods;

pub(super) struct WatchTarget<'a> {
    pub root: &'a str,
    pub task: Option<&'a zeron_proto::Task>,
    pub title: &'a str,
}

fn watched_eye(theme: &Theme, size: f32) -> AnyElement {
    icon(icons::LUCIDE_EYE)
        .size(px(size))
        .text_color(ok_tone(theme))
        .into_any_element()
}

impl TaskCenter {
    fn open_watch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.watch.open(());
        window.focus(&self.watch_focus, cx);
        self.load_authority(cx);
        cx.notify();
    }

    pub(super) fn close_watch(&mut self, cx: &mut Context<Self>) {
        if self.watch.begin_close() {
            popover::reap_popup(cx, |center: &mut Self| &mut center.watch);
            cx.notify();
        }
    }

    fn load_authority(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        if self.authority.ready().is_none() {
            self.authority = popover::Loadable::Loading;
        }
        self.authority_task = Some(cx.spawn(async move |center, cx| {
            let result = engine
                .client()
                .call_as::<SupervisorSettings>(
                    methods::GET_SUPERVISOR_SETTINGS,
                    serde_json::json!({}),
                )
                .await
                .map_err(|err| err.to_string());
            center
                .update(cx, |center, cx| {
                    center.authority = match result {
                        Ok(settings) => popover::Loadable::Ready(settings),
                        Err(error) => popover::Loadable::Error(error),
                    };
                    cx.notify();
                })
                .ok();
        }));
    }

    fn save_authority(&mut self, settings: SupervisorSettings, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let previous = self.authority.ready().copied();
        let patch = zeron_proto::supervisor::settings_patch(previous, settings);
        self.authority = popover::Loadable::Ready(settings);
        self.authority_task = Some(cx.spawn(async move |center, cx| {
            let saved = engine
                .client()
                .call_as::<SupervisorSettings>(methods::SET_SUPERVISOR_SETTINGS, patch)
                .await;
            center
                .update(cx, |center, cx| {
                    center.authority = match (saved, previous) {
                        (Ok(stored), _) => popover::Loadable::Ready(stored),
                        (Err(_), Some(previous)) => popover::Loadable::Ready(previous),
                        (Err(err), None) => popover::Loadable::Error(err.to_string()),
                    };
                    cx.notify();
                })
                .ok();
        }));
        cx.notify();
    }

    pub(super) fn render_watch_button(
        &self,
        target: WatchTarget<'_>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (watch, elsewhere) = {
            let state = self.state.read(cx);
            state.supervisor.as_ref()?;
            let watch = tree_watch(state, target.root);
            let elsewhere = match &watch {
                Watch::Elsewhere(device) => Some(
                    state
                        .device_name(device)
                        .unwrap_or("another device")
                        .to_owned(),
                ),
                _ => None,
            };
            (watch, elsewhere)
        };
        if let Some(name) = elsewhere {
            return Some(
                div()
                    .debug_selector(|| "task-center-watch-elsewhere".into())
                    .flex_none()
                    .text_size(ui_rems(12.0))
                    .text_color(theme.text_faint)
                    .child(format!("on {name}, not watched"))
                    .into_any_element(),
            );
        }
        let on = watch == Watch::Watched;
        let tone = ok_tone(theme);
        let open = self.watch.get().is_some();
        let label = if on { "Watched" } else { "Watch" };
        let button = div()
            .id("task-center-watch")
            .debug_selector(|| "task-center-watch".into())
            .role(gpui::Role::Button)
            .aria_label(label)
            .aria_expanded(self.watch.is_open())
            .relative()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.0))
            .h(px(28.0))
            .pl(px(9.0))
            .pr(px(12.0))
            .rounded_full()
            .border_1()
            .border_color(if on {
                tone.opacity(0.55)
            } else {
                theme.border_strong
            })
            .when(on, |el| el.bg(tone.opacity(0.08)))
            .when(open, |el| el.bg(theme.wash(0.06)))
            .cursor_pointer()
            .hover(|s| s.bg(theme.wash(0.06)))
            .text_size(ui_rems(12.5))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(theme.text)
            .child(if on {
                watched_eye(theme, 13.0)
            } else {
                icon(icons::LUCIDE_SHIELD)
                    .size(px(13.0))
                    .text_color(theme.text_muted)
                    .into_any_element()
            })
            .child(label)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|center, _, _, _| center.watch.note_trigger_press()),
            )
            .on_click(cx.listener(|center, event: &ClickEvent, window, cx| {
                cx.stop_propagation();
                let was_open = if matches!(event, ClickEvent::Keyboard(_)) {
                    center.watch.is_open()
                } else {
                    center.watch.take_press_was_open()
                };
                if was_open {
                    center.close_watch(cx);
                } else {
                    center.open_watch(window, cx);
                }
            }))
            .when(open, |el| {
                el.child(popover::anchored_menu_below_end(
                    "task-center-watch-sheet",
                    self.render_watch_sheet(&target, on, theme, cx),
                    self.watch.closing_since(),
                ))
            });
        Some(button.into_any_element())
    }

    fn render_watch_sheet(
        &self,
        target: &WatchTarget<'_>,
        on: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let popup = theme.for_popup();
        let plan = target.task.is_some();
        let (last, by_everything) = {
            let state = self.state.read(cx);
            let snapshot = state.supervisor.as_ref();
            let task_id = target.task.map(|task| task.id.as_str());
            let last = snapshot.and_then(|snapshot| {
                snapshot.wakes.iter().rev().find_map(|wake| {
                    wake.events
                        .iter()
                        .rev()
                        .find(|event| {
                            event.root_chat_id.as_deref() == Some(target.root)
                                || (task_id.is_some() && event.task_id.as_deref() == task_id)
                        })
                        .map(|event| {
                            let at = wake.at.with_timezone(&chrono::Local).format("%H:%M");
                            format!("{at} \u{b7} {}", event.label)
                        })
                })
            });
            let by_everything = !plan
                && snapshot.is_some_and(|snapshot| {
                    snapshot.watch_everything
                        && !snapshot.watched_roots.iter().any(|id| id == target.root)
                });
            (last, by_everything)
        };
        let heading = div()
            .flex()
            .items_center()
            .gap(px(7.0))
            .px(px(8.0))
            .pt(px(4.0))
            .text_size(ui_rems(13.0))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(popup.text)
            .child(if on {
                watched_eye(&popup, 14.0)
            } else {
                icon(icons::LUCIDE_SHIELD)
                    .size(px(14.0))
                    .text_color(popup.text)
                    .into_any_element()
            })
            .child(div().min_w_0().truncate().child(format!(
                "{} {}",
                if on { "Watching" } else { "Watch" },
                target.title
            )));
        let line = |key: &'static str, value: String| {
            div()
                .flex()
                .gap(px(10.0))
                .text_size(ui_rems(12.0))
                .text_color(popup.text_muted)
                .child(
                    div()
                        .flex_none()
                        .w(px(34.0))
                        .text_color(popup.text_faint)
                        .child(key),
                )
                .child(div().min_w_0().truncate().child(value))
        };
        let about = if on {
            div()
                .px(px(8.0))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(line(
                    "Last",
                    last.unwrap_or_else(|| "nothing from this tree yet".into()),
                ))
                .child(line("Next", "on the next ask, failure or finish".into()))
        } else {
            div()
                .px(px(8.0))
                .text_size(ui_rems(12.0))
                .line_height(ui_rems(18.0))
                .text_color(popup.text_muted)
                .child(
                    "The Supervisor wakes when anything in this tree asks, fails or finishes, at any depth, and acts inside its authority. It reads every thread in the tree.",
                )
        };
        let authority = self.render_watch_authority(plan, &popup, cx);
        let action = |label: &'static str, key: &'static str, tone: widgets::ActionTone| {
            widgets::text_action(&popup, tone, label)
                .id(key)
                .debug_selector(move || key.into())
                .role(gpui::Role::Button)
                .aria_label(label)
                .flex_none()
                .min_h(px(28.0))
                .rounded_full()
        };
        let root = target.root.to_string();
        let task_id = target.task.map(|task| task.id.clone());
        let send = move |on: bool| {
            let (root, task_id) = (root.clone(), task_id.clone());
            move |center: &mut Self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>| {
                cx.emit(TaskCenterEvent::Watch {
                    root: root.clone(),
                    task_id: task_id.clone(),
                    on,
                });
                center.close_watch(cx);
            }
        };
        let footer = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(4.0))
            .pt(px(8.0))
            .border_t_1()
            .border_color(popup.border);
        let footer = if on {
            footer
                .child(
                    action(
                        "Open Supervisor",
                        "task-center-watch-open-supervisor",
                        widgets::ActionTone::Quiet,
                    )
                    .on_click(cx.listener(|center, _: &ClickEvent, _, cx| {
                        center.close_watch(cx);
                        cx.emit(TaskCenterEvent::OpenSupervisor);
                    })),
                )
                .child(div().flex_1())
                .child(if by_everything {
                    div()
                        .debug_selector(|| "task-center-watch-everything".into())
                        .px(px(8.0))
                        .text_size(ui_rems(12.0))
                        .text_color(popup.text_faint)
                        .child("Watch everything is on")
                        .into_any_element()
                } else {
                    action(
                        "Stop watching",
                        "task-center-watch-stop",
                        widgets::ActionTone::Quiet,
                    )
                    .text_color(popup.danger)
                    .on_click(cx.listener(send(false)))
                    .into_any_element()
                })
        } else {
            footer
                .child(div().flex_1())
                .child(
                    action(
                        "Cancel",
                        "task-center-watch-cancel",
                        widgets::ActionTone::Quiet,
                    )
                    .on_click(cx.listener(|center, _: &ClickEvent, _, cx| center.close_watch(cx))),
                )
                .child(
                    action(
                        "Start watching",
                        "task-center-watch-start",
                        widgets::ActionTone::Solid,
                    )
                    .on_click(cx.listener(send(true))),
                )
        };
        popover::popover_card(&popup)
            .id("task-center-watch-card")
            .debug_selector(|| "task-center-watch-sheet".into())
            .track_focus(&self.watch_focus)
            .role(gpui::Role::Dialog)
            .aria_label("Watch with the Supervisor")
            .w(px(340.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(|_, _, cx| cx.stop_propagation())
            .on_mouse_down_out(cx.listener(|center, _, _, cx| center.close_watch(cx)))
            .on_key_down(cx.listener(|center, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    center.close_watch(cx);
                }
            }))
            .child(heading)
            .child(about)
            .child(authority)
            .child(footer)
            .into_any_element()
    }

    fn render_watch_authority(
        &self,
        plan: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let list = div()
            .debug_selector(|| "task-center-watch-authority".into())
            .flex()
            .flex_col()
            .child(popover::menu_heading(theme, "Authority"));
        let settings = match &self.authority {
            popover::Loadable::Ready(settings) => *settings,
            popover::Loadable::Error(error) => {
                return list
                    .child(div().px(px(4.0)).child(widgets::error_strip(
                        theme,
                        format!("Could not read the Supervisor's authority: {error}"),
                    )))
                    .into_any_element();
            }
            _ => {
                return list
                    .child(
                        div()
                            .px(px(8.0))
                            .py(px(5.0))
                            .text_size(ui_rems(12.0))
                            .text_color(theme.text_muted)
                            .child("Loading…"),
                    )
                    .into_any_element();
            }
        };
        let row = |key: &'static str,
                   title: &'static str,
                   body: &'static str,
                   on: bool,
                   enabled: bool| {
            div()
                .flex()
                .items_start()
                .gap(px(12.0))
                .px(px(8.0))
                .py(px(5.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .text_size(ui_rems(12.5))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(if enabled {
                                    theme.text
                                } else {
                                    theme.text_faint
                                })
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(ui_rems(11.5))
                                .line_height(ui_rems(16.0))
                                .text_color(if enabled {
                                    theme.text_muted
                                } else {
                                    theme.text_faint
                                })
                                .child(body),
                        ),
                )
                .child(
                    div()
                        .mt(px(2.0))
                        .when(!enabled, |el| el.opacity(0.45))
                        .child(SupervisorPage::render_switch(
                            theme,
                            key.into(),
                            title.into(),
                            on,
                        )),
                )
        };
        let decide = row(
            "task-center-watch-decide",
            "Decide for me",
            if plan {
                "Answers the Lead's Needs you decisions and approves plans. Each choice shows in the feed with why, and you can change it."
            } else {
                "Answers what this tree's agents ask you. With no plan there is nothing to approve yet."
            },
            settings.decide_for_me,
            true,
        )
        .id("task-center-watch-decide-row")
        .cursor_pointer()
        .on_click(cx.listener(move |center, _: &ClickEvent, _, cx| {
            center.save_authority(
                SupervisorSettings {
                    decide_for_me: !settings.decide_for_me,
                    ..settings
                },
                cx,
            )
        }));
        let goal = row(
            "task-center-watch-goal",
            "Change a mission's goal or spend",
            if plan {
                "Lets it decide those too."
            } else {
                "Applies once this tree has a plan."
            },
            settings.goal_and_spend,
            plan,
        )
        .id("task-center-watch-goal-row");
        let goal = if plan {
            goal.cursor_pointer()
                .on_click(cx.listener(move |center, _: &ClickEvent, _, cx| {
                    center.save_authority(
                        SupervisorSettings {
                            goal_and_spend: !settings.goal_and_spend,
                            ..settings
                        },
                        cx,
                    )
                }))
                .into_any_element()
        } else {
            goal.into_any_element()
        };
        list.child(decide)
            .child(goal)
            .child(
                div()
                    .px(px(8.0))
                    .text_size(ui_rems(11.0))
                    .text_color(theme.text_faint)
                    .child("Both switches apply to everything the Supervisor watches."),
            )
            .into_any_element()
    }
}
