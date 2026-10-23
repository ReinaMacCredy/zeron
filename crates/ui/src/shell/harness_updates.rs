//! Home's agent-update island: one bottom-anchored surface for the compact
//! summary and the expanded list. Geometry uses the shell's reversible RESIZE
//! tweens; content keeps its final width while the outer surface clips it.

use super::*;
use zeron_proto::{HarnessId, HarnessUpdatePhase as Phase, HarnessUpdateStatus};

pub(super) fn versionless_notification_key(device: &str, harness: HarnessId) -> String {
    format!("{device}:{harness:?}:versionless")
}

pub(super) fn notification_key(device: &str, status: &HarnessUpdateStatus) -> Option<String> {
    if status.phase != Phase::Available {
        return None;
    }
    Some(match status.latest_version.as_deref() {
        Some(version) => format!("{device}:{:?}:{version}", status.harness),
        None => versionless_notification_key(device, status.harness),
    })
}

const CHIP_HEIGHT: f32 = 38.0;
/// The summary row inside the card's 1px border, top and bottom. A row as
/// tall as the whole card overflows the border box and sits a point low.
const SUMMARY_HEIGHT: f32 = CHIP_HEIGHT - 2.0;
const ROW_HEIGHT: f32 = 64.0;
const MAX_VISIBLE_ROWS: f32 = 3.5;
const LIST_FADE_BAND: f32 = 32.0;
const LIST_WIDTH: f32 = 360.0;
const MARK_SIZE: f32 = 22.0;
const MARK_STEP: f32 = 14.0;
const MAX_MARKS: usize = 3;

#[derive(Default)]
pub(super) struct DeviceUpdates {
    online: bool,
    connected: bool,
    statuses: Vec<HarnessUpdateStatus>,
    watch: Option<Task<()>>,
    limit_watch: Option<Task<()>>,
}

#[derive(Clone)]
struct UpdateRow {
    device_id: String,
    device_name: String,
    connected: bool,
    status: HarnessUpdateStatus,
}

// Reconcile presence without throwing away a disconnected device's last known
// updates. Removed/unsupported devices leave the inventory entirely.
fn reconcile_devices(
    devices: &mut std::collections::BTreeMap<String, DeviceUpdates>,
    desired: &std::collections::BTreeMap<String, bool>,
) -> Vec<String> {
    devices.retain(|id, _| desired.contains_key(id));
    let mut start = Vec::new();
    for (id, online) in desired {
        let updates = devices.entry(id.clone()).or_default();
        updates.online = *online;
        if !online {
            updates.watch = None;
            updates.limit_watch = None;
            updates.connected = false;
        } else if updates.watch.is_none() {
            start.push(id.clone());
        }
    }
    start
}

fn visible_rows(
    devices: &std::collections::BTreeMap<String, DeviceUpdates>,
    device_name: impl Fn(&str) -> String,
) -> Vec<UpdateRow> {
    devices
        .iter()
        .flat_map(|(id, updates)| {
            let name = device_name(id);
            updates
                .statuses
                .iter()
                .filter(|status| status.show_update_notice())
                .map(move |status| UpdateRow {
                    device_id: id.clone(),
                    device_name: name.clone(),
                    connected: updates.online && updates.connected,
                    status: status.clone(),
                })
        })
        .collect()
}

fn active(status: &HarnessUpdateStatus) -> bool {
    matches!(
        status.phase,
        Phase::WaitingForIdle
            | Phase::Preparing
            | Phase::Downloading
            | Phase::Installing
            | Phase::Verifying
    )
}

fn action(status: &HarnessUpdateStatus) -> Option<(&'static str, Option<&'static str>)> {
    match status.phase {
        Phase::Available if status.can_apply => {
            Some(("Update", Some(methods::APPLY_HARNESS_UPDATE)))
        }
        Phase::Available => Some(("View steps", None)),
        Phase::WaitingForIdle | Phase::Preparing | Phase::Downloading => {
            Some(("Cancel", Some(methods::CANCEL_HARNESS_UPDATE)))
        }
        Phase::Failed => Some(("Check again", Some(methods::CHECK_HARNESS_UPDATES))),
        _ => None,
    }
}

fn right_inset(has_button: bool) -> f32 {
    if has_button { 5.0 } else { 12.0 }
}

fn detail(status: &HarnessUpdateStatus) -> String {
    match status.phase {
        Phase::Available => match (&status.installed_version, &status.latest_version) {
            (Some(installed), Some(latest)) => format!("{installed} → {latest}"),
            (_, Some(latest)) => format!("Version {latest} available"),
            _ => "Update available".into(),
        },
        Phase::WaitingForIdle => "Waiting for the current run".into(),
        Phase::Preparing => "Preparing update…".into(),
        Phase::Downloading => status
            .progress
            .as_ref()
            .and_then(|p| p.message.clone())
            .unwrap_or_else(|| "Downloading…".into()),
        Phase::Installing => "Installing…".into(),
        Phase::Verifying => "Verifying installation…".into(),
        Phase::Updated => status
            .installed_version
            .as_ref()
            .map(|v| format!("Updated to {v}"))
            .unwrap_or_else(|| "Updated".into()),
        Phase::Failed => status
            .error
            .as_ref()
            .map(|e| e.message.clone())
            .unwrap_or_else(|| "Couldn’t check for updates".into()),
        _ => "Checking for updates…".into(),
    }
}

fn mark_stack(statuses: &[UpdateRow], theme: &Theme) -> gpui::Div {
    // Use the same registry order as the list. A phase change must not
    // reshuffle the overlapping brand marks.
    let count = statuses.len().min(MAX_MARKS);
    let width = MARK_SIZE + MARK_STEP * count.saturating_sub(1) as f32;
    let background = crate::theme::flatten(theme.input_glass_bg(), theme.bg);
    div()
        .relative()
        .flex_none()
        .w(px(width))
        .h(px(MARK_SIZE))
        .children(
            statuses
                .iter()
                .take(MAX_MARKS)
                .enumerate()
                .rev()
                .map(|(index, row)| {
                    let status = &row.status;
                    let (mark, tint) = crate::pickers::harness_brand_icon(status.harness);
                    crate::frost::layered(
                        div()
                            .absolute()
                            .left(px(index as f32 * MARK_STEP))
                            .top_0()
                            .size(px(MARK_SIZE))
                            .rounded_full()
                            .when(count > 1, |el| {
                                el.bg(background)
                                    .border_1()
                                    .border_color(crate::theme::composer_surface_border(theme))
                            })
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                icon(mark)
                                    .size(px(16.0))
                                    .text_color(tint.unwrap_or(theme.text_muted)),
                            ),
                    )
                }),
        )
}

fn text_width(text: &str, size: f32, window: &Window, theme: &Theme) -> f32 {
    let mut font = gpui::font(theme.font_sans.clone());
    font.weight = gpui::FontWeight::MEDIUM;
    let line = window.text_system().shape_line(
        SharedString::from(text.to_owned()),
        crate::typography::ui_rems(size).to_pixels(window.rem_size()),
        &[gpui::TextRun {
            len: text.len(),
            font,
            color: theme.text,
            background_color: None,
            underline: None,
            strikethrough: None,
        }],
        None,
    );
    f32::from(line.width()).ceil()
}

impl Shell {
    pub(super) fn set_harness_updates_expanded(&mut self, expanded: bool, cx: &mut Context<Self>) {
        let expanded = expanded
            && self
                .harness_update_devices
                .values()
                .flat_map(|device| &device.statuses)
                .filter(|row| row.show_update_notice())
                .count()
                > 1;
        if self.harness_update_expanded == expanded {
            return;
        }
        if expanded {
            self.harness_update_scroll.reset();
        }
        let from = self.eval_tween(
            self.harness_update_transition,
            if self.harness_update_expanded {
                1.0
            } else {
                0.0
            },
        );
        self.harness_update_expanded = expanded;
        self.harness_update_transition =
            Some(WidthTween::new(from, if expanded { 1.0 } else { 0.0 }));
        cx.notify();
    }

    fn open_harness_update_steps(&mut self, device: String, cx: &mut Context<Self>) {
        let target = (self.state.read(cx).local_device_id.as_deref() != Some(device.as_str()))
            .then_some(device);
        self.set_harness_updates_expanded(false, cx);
        self.open_settings(SettingsSection::Harnesses, cx);
        let page = cx.new(|cx| HarnessesPage::new(self.state.clone(), cx));
        page.update(cx, |page, cx| page.set_target_device(target, cx));
        self.harnesses_page = Some(page);
    }

    fn render_harness_update_action(
        &mut self,
        row: &UpdateRow,
        interactive: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let status = &row.status;
        let device = row.device_id.clone();
        let key_device = device.clone();
        let (label, method) = action(status)?;
        let theme = Theme::of(cx).for_settings_surface();
        let harness = status.harness;
        // The settings/details link is local UI and remains usable offline.
        let available = method.is_none() || row.connected;
        let enabled = interactive && available;
        let primary = method == Some(methods::APPLY_HARNESS_UPDATE);
        Some(
            div()
                .id(SharedString::from(format!(
                    "harness-update-action-{device}-{harness:?}"
                )))
                .h(px(26.0))
                .px(px(9.0))
                .flex_none()
                .rounded_full()
                .border_1()
                .border_color(gpui::transparent_black())
                .flex()
                .items_center()
                .justify_center()
                .text_size(crate::typography::ui_rems(11.5))
                .font_weight(gpui::FontWeight::MEDIUM)
                .bg(if primary {
                    theme.text
                } else {
                    theme.element_hover
                })
                .text_color(if primary {
                    theme.on_solid
                } else {
                    theme.text_muted
                })
                .opacity(if available { 1.0 } else { 0.45 })
                .role(gpui::Role::Button)
                .aria_label(format!(
                    "{label} · {} · {}",
                    harness.display_name(),
                    row.device_name
                ))
                .tab_index(if enabled { 0 } else { -1 })
                .focus_visible(move |el| el.border_color(theme.accent))
                .when(enabled, |button| {
                    button
                        .cursor_pointer()
                        .hover(|el| el.opacity(0.85))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(method) = method {
                                this.run_harness_update_action(device.clone(), method, harness, cx);
                            } else {
                                this.open_harness_update_steps(device.clone(), cx);
                            }
                        }))
                        .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                cx.stop_propagation();
                                if let Some(method) = method {
                                    this.run_harness_update_action(
                                        key_device.clone(),
                                        method,
                                        harness,
                                        cx,
                                    );
                                } else {
                                    this.open_harness_update_steps(key_device.clone(), cx);
                                }
                            }
                        }))
                })
                .child(label)
                .into_any_element(),
        )
    }

    /// One surface, anchored by the Home mount. The bottom edge never moves;
    /// width, height, corner radius and row reveal share the shell resize clock.
    pub(super) fn render_harness_update_card(
        &mut self,
        window: &mut Window,
        main_width: f32,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        self.refresh_harness_update_watch(cx);
        let marks = visible_rows(&self.harness_update_devices, |id| {
            self.state.read(cx).device_name(id).unwrap_or(id).to_owned()
        });
        if marks.is_empty() {
            self.harness_update_expanded = false;
            self.harness_update_transition = None;
            self.harness_update_geometry = [None; 2];
            return None;
        }
        // Updates change in place; promoting active/completed rows makes
        // harnesses jump beneath the pointer when an action is clicked.
        let statuses = &marks;
        let row = &statuses[0];
        let status = &row.status;
        let multiple = statuses.len() > 1;
        if !multiple {
            self.set_harness_updates_expanded(false, cx);
        }
        let expanded = self.harness_update_expanded;
        let reveal = self.eval_tween(
            self.harness_update_transition,
            if expanded { 1.0 } else { 0.0 },
        );
        let theme = Theme::of(cx).for_settings_surface();
        let name = status.harness.display_name();
        let all_updated = statuses
            .iter()
            .all(|row| row.status.phase == Phase::Updated);
        let mut title = if multiple {
            if all_updated {
                format!("{} agents updated", statuses.len())
            } else {
                format!("{} agent updates", statuses.len())
            }
        } else {
            match status.phase {
                Phase::Available => status
                    .latest_version
                    .as_ref()
                    .map(|v| format!("{name} {v} available"))
                    .unwrap_or_else(|| format!("{name} update available")),
                Phase::WaitingForIdle => format!("{name} · waiting for idle"),
                Phase::Preparing => format!("Preparing {name}…"),
                Phase::Downloading => format!("Downloading {name}…"),
                Phase::Installing => format!("Updating {name}…"),
                Phase::Verifying => format!("Verifying {name}…"),
                Phase::Updated => format!("{name} updated"),
                Phase::Failed => format!("{name} update check failed"),
                _ => return None,
            }
        };
        let device_count = statuses
            .iter()
            .map(|row| &row.device_id)
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        if device_count == 1 {
            title = format!("{title} · {}", row.device_name);
        } else {
            title = format!("{title} · {device_count} devices");
        }
        if statuses.iter().any(|row| !row.connected) {
            title = format!("{title} · disconnected");
        }
        let activity = if statuses
            .iter()
            .any(|row| row.connected && active(&row.status))
        {
            Some(
                crate::loaders::ring_spinner("home-agent-update-activity", 14.0, theme.text_muted)
                    .into_any_element(),
            )
        } else if all_updated {
            Some(
                icon(icons::CHECK)
                    .size(px(14.0))
                    .text_color(theme.success)
                    .into_any_element(),
            )
        } else if !multiple && status.phase == Phase::Failed {
            Some(
                icon(icons::DANGER_TRIANGLE)
                    .size(px(14.0))
                    .text_color(theme.danger)
                    .into_any_element(),
            )
        } else {
            None
        };
        let has_activity = activity.is_some();
        let update_all = self.update_all_plan(cx).items() >= 2;
        let action_label = if update_all {
            Some("Update all")
        } else {
            (!multiple)
                .then(|| action(status))
                .flatten()
                .map(|(label, _)| label)
        };
        let compact_action = if update_all {
            Some(render_update_all_button(&theme, cx))
        } else if multiple {
            None
        } else {
            self.render_harness_update_action(row, true, cx)
        };
        let trailing = if multiple {
            8.0
        } else {
            right_inset(action_label.is_some())
        };
        let marks_width =
            MARK_SIZE + MARK_STEP * marks.len().min(MAX_MARKS).saturating_sub(1) as f32;
        let controls_width = if multiple { 24.0 + 8.0 } else { 0.0 }
            + action_label
                .filter(|_| update_all || !multiple)
                .map(|label| text_width(label, 11.5, window, &theme) + 20.0 + 8.0)
                .unwrap_or(0.0);
        let max_width = (main_width - 32.0).max(0.0);
        let compact_width = (12.0
            + marks_width
            + 8.0
            + text_width(&title, 12.0, window, &theme)
            + if has_activity { 22.0 } else { 0.0 }
            + controls_width
            + trailing
            + 2.0)
            .min(max_width);
        let list_width = LIST_WIDTH.min(max_width);
        let list_height = (CHIP_HEIGHT
            + ROW_HEIGHT * (statuses.len() as f32).min(MAX_VISIBLE_ROWS))
        .min((self.viewport_height - Theme::TITLEBAR_HEIGHT - 64.0).max(CHIP_HEIGHT));
        let targets = if expanded {
            [list_width, list_height]
        } else {
            [compact_width, CHIP_HEIGHT]
        };
        let mut size = targets;
        for (axis, target) in targets.into_iter().enumerate() {
            let old = self.harness_update_geometry[axis];
            self.harness_update_geometry[axis] = Some(match old {
                None => WidthTween::new(target, target),
                Some(tween) if (tween.to - target).abs() > 0.5 => {
                    WidthTween::new(self.eval_tween(old, tween.to), target)
                }
                Some(tween) => tween,
            });
            size[axis] = self.eval_tween(self.harness_update_geometry[axis], target);
        }
        let radius = motion::lerp(19.0, 16.0, reveal);
        let list_bottom_radius = (radius - 1.0).max(0.0);
        let summary = div()
            .id("home-harness-update-summary")
            .h(px(SUMMARY_HEIGHT))
            .w_full()
            .flex_none()
            .pl(px(12.0))
            .pr(px(trailing))
            .flex()
            .items_center()
            .gap(px(0.0))
            .when(multiple, |el| {
                el.cursor_pointer()
                    .role(gpui::Role::Button)
                    .aria_label(if expanded {
                        "Collapse agent updates"
                    } else {
                        "Show agent updates"
                    })
                    .tab_index(0)
                    .rounded(px(radius))
                    .focus_visible(move |el| el.bg(theme.accent.opacity(0.12)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.set_harness_updates_expanded(!this.harness_update_expanded, cx);
                    }))
                    .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            cx.stop_propagation();
                            this.set_harness_updates_expanded(!this.harness_update_expanded, cx);
                        }
                    }))
            })
            .child(
                div()
                    .flex_none()
                    .w(px(marks_width * (1.0 - reveal)))
                    .mr(px(8.0 * (1.0 - reveal)))
                    .overflow_hidden()
                    .opacity(1.0 - crate::composer_dock::stage(reveal, 0.0, 0.55))
                    .child(mark_stack(&marks, &theme)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(crate::typography::ui_rems(12.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(title),
            )
            .when_some(activity, |el, activity| {
                el.child(div().flex_none().ml(px(8.0)).child(activity))
            })
            .when_some(compact_action, |el, action| {
                el.child(div().flex_none().ml(px(8.0)).child(action))
            })
            .when(multiple, |el| {
                el.child(
                    div()
                        .relative()
                        .size(px(24.0))
                        .flex_none()
                        .ml(px(8.0))
                        .child(
                            icon(icons::ALT_ARROW_DOWN)
                                .absolute()
                                .left(px(5.0))
                                .top(px(5.0))
                                .size(px(14.0))
                                .text_color(theme.text_muted)
                                .with_transformation(gpui::Transformation::rotate(gpui::radians(
                                    std::f32::consts::PI * (1.0 - reveal),
                                ))),
                        ),
                )
            });
        let rows = (reveal > 0.0).then(|| {
            let rows: Vec<_> = statuses
                .iter()
                .enumerate()
                .map(|(index, row)| {
                    let status = &row.status;
                    // Reveal after the surface has made room. Use the same
                    // reversible progress on exit; never replay a mount animation.
                    let row_reveal = crate::composer_dock::stage(reveal, 0.42, 0.9);
                    let (mark, tint) = crate::pickers::harness_brand_icon(status.harness);
                    let label = if row.connected {
                        detail(status)
                    } else {
                        "Disconnected · reconnect to update".into()
                    };
                    let extra = status
                        .error
                        .as_ref()
                        .map(|e| e.message.clone())
                        .or_else(|| status.manual_command.clone())
                        .unwrap_or_else(|| label.clone());
                    let tooltip = Some(format!(
                        "{} · {}\n{extra}",
                        status.harness.display_name(),
                        row.device_name
                    ));
                    let row_action =
                        self.render_harness_update_action(row, expanded && row_reveal >= 0.95, cx);
                    div()
                        .id(SharedString::from(format!(
                            "harness-update-row-{}-{:?}",
                            row.device_id, status.harness
                        )))
                        .relative()
                        .top(px(3.0 * (1.0 - row_reveal)))
                        .opacity(row_reveal)
                        .h(px(ROW_HEIGHT))
                        .flex_none()
                        .px(px(12.0))
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .when(index > 0, |el| {
                            el.child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .left(px(42.0))
                                    .right(px(12.0))
                                    .h(px(1.0))
                                    .bg(settings::widgets::row_divider(&theme)),
                            )
                        })
                        .child(
                            icon(mark)
                                .size(px(20.0))
                                .flex_none()
                                .text_color(tint.unwrap_or(theme.text_muted)),
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
                                        .text_size(crate::typography::ui_rems(12.0))
                                        .line_height(crate::typography::ui_rems(16.0))
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .truncate()
                                        .child(status.harness.display_name()),
                                )
                                .when(device_count > 1, |el| {
                                    el.child(
                                        div()
                                            .text_size(crate::typography::ui_rems(11.0))
                                            .line_height(crate::typography::ui_rems(14.0))
                                            .text_color(theme.text_muted)
                                            .truncate()
                                            .child(row.device_name.clone()),
                                    )
                                })
                                .child(
                                    div()
                                        .text_size(crate::typography::ui_rems(11.0))
                                        .line_height(crate::typography::ui_rems(14.0))
                                        .text_color(if status.phase == Phase::Failed {
                                            theme.danger
                                        } else {
                                            theme.text_muted
                                        })
                                        .truncate()
                                        .child(label),
                                ),
                        )
                        .when(status.phase == Phase::Updated, |el| {
                            el.child(icon(icons::CHECK).size(px(14.0)).text_color(theme.success))
                        })
                        .children(row_action)
                        .when_some(tooltip, |el, text| {
                            el.tooltip(move |_, cx| {
                                cx.new(|_| SurfaceTabTooltip {
                                    text: text.clone().into(),
                                })
                                .into()
                            })
                        })
                        .into_any_element()
                })
                .collect();
            let rail = settings::widgets::rail(
                &mut self.harness_update_scroll,
                "home-harness-update-scrollbar",
                &theme,
                cx,
                |shell| &mut shell.harness_update_scroll,
            );
            let list = div()
                .id("home-harness-update-list")
                .size_full()
                .overflow_y_scroll()
                .track_scroll(&self.harness_update_scroll.scroll)
                .flex()
                .flex_col()
                .children(rows)
                // The fading list is inert while collapsing or before its
                // reveal. It must not intercept a click through clipped rows.
                .when(!expanded || reveal < 0.85, |el| {
                    el.child(div().absolute().inset_0().occlude())
                });
            let list = crate::edge_fade::edge_faded(LIST_FADE_BAND, true, true, list)
                .fade_overflow_y(&self.harness_update_scroll.scroll);
            div()
                .id("home-harness-update-list-host")
                .absolute()
                .top(px(SUMMARY_HEIGHT))
                // Keep the final layout centered as the shell widens, so
                // neither edge appears attached to a moving clipping boundary.
                // Absolute offsets start inside the card's border already.
                .left(px((size[0] - list_width) * 0.5))
                .w(px((list_width - 2.0).max(0.0)))
                .h(px((list_height - SUMMARY_HEIGHT - 1.0).max(0.0)))
                .rounded_bl(px(list_bottom_radius))
                .rounded_br(px(list_bottom_radius))
                .overflow_hidden()
                .bg(settings::widgets::block_fill(&theme))
                .opacity(crate::composer_dock::stage(reveal, 0.3, 0.72))
                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                    if this.harness_update_scroll.set_list_hovered(*hovered) {
                        cx.notify();
                    }
                }))
                .child(list)
                .children(rail)
        });
        let single_tooltip = (!multiple)
            .then(|| {
                status
                    .error
                    .as_ref()
                    .map(|e| e.message.clone())
                    .or_else(|| status.manual_command.clone())
            })
            .flatten();
        let card = div()
            .id("home-harness-update-card")
            .relative()
            .w(px(size[0]))
            .h(px(size[1]))
            .rounded(px(radius))
            .overflow_hidden()
            .border_1()
            .border_color(theme.border.opacity(0.7))
            .text_color(theme.text)
            .bg(popover::surface_bg(&theme))
            .when(!theme.is_frost(), |el| el.shadow_lg())
            .when(expanded, |el| {
                el.on_mouse_down_out(
                    cx.listener(|this, _, _, cx| this.set_harness_updates_expanded(false, cx)),
                )
            })
            .when_some(single_tooltip, |el, text| {
                el.tooltip(move |_, cx| {
                    cx.new(|_| SurfaceTabTooltip {
                        text: text.clone().into(),
                    })
                    .into()
                })
            })
            .child(summary)
            .when(reveal > 0.0, |el| {
                el.child(
                    div()
                        .absolute()
                        .top(px(SUMMARY_HEIGHT))
                        .left(px(0.0))
                        .right(px(0.0))
                        .h(px(1.0))
                        .bg(settings::widgets::row_divider(&theme))
                        .opacity(crate::composer_dock::stage(reveal, 0.2, 0.65)),
                )
            })
            .children(rows);
        Some(crate::frost::frosted(radius, 16.0, card).into_any_element())
    }
    pub(super) fn refresh_harness_update_watch(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        if !matches!(state.connection, ConnectionStatus::Ready) {
            self.harness_update_devices.clear();
            self.harness_update_expanded = false;
            self.harness_update_transition = None;
            self.harness_update_geometry = [None; 2];
            return;
        }
        let Some(engine) = state.engine().cloned() else {
            return;
        };
        // Include the connected engine before its registry row arrives. Selection
        // of a chat, project, or composer host must not affect this inventory.
        let devices: std::collections::BTreeMap<_, _> = state
            .devices
            .iter()
            .map(|device| device.id.clone())
            .chain(std::iter::once(engine.engine_info().device_id.clone()))
            .filter(|id| state.device_supports(id, zeron_proto::capabilities::HARNESS_UPDATES_V1))
            .map(|id| {
                let online = state.device_online(&id, Utc::now());
                (id, online)
            })
            .collect();
        let limit_devices: Vec<String> = devices
            .keys()
            .filter(|id| state.device_supports(id, zeron_proto::capabilities::LIMIT_RESUME_V1))
            .cloned()
            .collect();
        let app_state = self.state.clone();
        for device in reconcile_devices(&mut self.harness_update_devices, &devices) {
            let updates = self.harness_update_devices.get_mut(&device).unwrap();
            if limit_devices.contains(&device) {
                updates.limit_watch = Some(spawn_limit_watch(
                    engine.clone(),
                    app_state.clone(),
                    device.clone(),
                    cx,
                ));
            }
            let engine = engine.clone();
            updates.watch = Some(cx.spawn(async move |this, cx| {
                let mut retry = 1;
                loop {
                    let result = engine
                        .client()
                        .subscribe_checked(
                            methods::WATCH_HARNESS_UPDATES,
                            serde_json::json!({ "targetDeviceId": device }),
                        )
                        .await;
                    if let Ok(mut stream) = result {
                        while let Some(value) = stream.recv().await {
                            let Ok(rows) = serde_json::from_value(value) else {
                                break;
                            };
                            if this
                                .update(cx, |shell, cx| {
                                    let Some(updates) =
                                        shell.harness_update_devices.get_mut(&device)
                                    else {
                                        return;
                                    };
                                    updates.statuses = rows;
                                    updates.connected = true;
                                    cx.notify();
                                })
                                .is_err()
                            {
                                return;
                            }
                            retry = 1;
                        }
                    }
                    if this
                        .update(cx, |shell, cx| {
                            if let Some(updates) = shell.harness_update_devices.get_mut(&device) {
                                updates.connected = false;
                                cx.notify();
                            }
                        })
                        .is_err()
                    {
                        return;
                    }
                    cx.background_executor()
                        .timer(Duration::from_secs(retry))
                        .await;
                    retry = (retry * 2).min(15);
                }
            }));
        }
    }

    fn run_harness_update_action(
        &mut self,
        device: String,
        method: &'static str,
        harness: HarnessId,
        cx: &mut Context<Self>,
    ) {
        let state = self.state.read(cx);
        let Some(engine) = state.engine().cloned() else {
            return;
        };
        if !self
            .harness_update_devices
            .get(&device)
            .is_some_and(|updates| updates.online && updates.connected)
            || !state.device_online(&device, Utc::now())
            || !state.device_supports(&device, zeron_proto::capabilities::HARNESS_UPDATES_V1)
        {
            return;
        }
        let params = serde_json::json!({ "harness": harness, "targetDeviceId": device });
        // Each request owns its lifetime: acting on a second device must not
        // cancel the first device's RPC. Progress comes from independent watches.
        cx.spawn(async move |this, cx| {
            let result = engine.client().call(method, params).await;
            this.update(cx, |shell, cx| {
                if let Err(error) = result
                    && !matches!(&error, zeron_rpc::RpcError::Failed(message)
                        if method == methods::APPLY_HARNESS_UPDATE && message == "update cancelled")
                {
                    shell.show_notice(crate::toast::ToastKind::Error, format!("Agent update ({device}): {error}"), cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn status(phase: Phase) -> HarnessUpdateStatus {
        HarnessUpdateStatus {
            harness: HarnessId::Codex,
            installed_version: Some("1.0".into()),
            latest_version: Some("2.0".into()),
            channel: None,
            source: Default::default(),
            policy: Default::default(),
            phase,
            progress: None,
            checked_at: None,
            error: None,
            can_apply: true,
            manual_command: None,
        }
    }

    fn device(phase: Phase) -> DeviceUpdates {
        DeviceUpdates {
            online: true,
            connected: true,
            statuses: vec![status(phase)],
            watch: None,
            limit_watch: None,
        }
    }

    #[test]
    fn same_agent_on_two_hosts_keeps_both_identities_and_progress() {
        let mut devices = BTreeMap::from([
            ("desktop".into(), device(Phase::Available)),
            ("laptop".into(), device(Phase::Installing)),
        ]);
        let rows = visible_rows(&devices, |id| format!("My {id}"));
        assert_eq!(rows.len(), 2);
        assert_eq!(
            (&*rows[0].device_id, &*rows[0].device_name),
            ("desktop", "My desktop")
        );
        assert_eq!(
            (&*rows[1].device_id, &*rows[1].device_name),
            ("laptop", "My laptop")
        );
        assert_eq!(action(&rows[0].status).unwrap().0, "Update");
        assert!(action(&rows[1].status).is_none());
        assert!(active(&rows[1].status));

        // A phase transition must not move the clicked device's row.
        devices.get_mut("desktop").unwrap().statuses[0].phase = Phase::Downloading;
        devices.get_mut("laptop").unwrap().statuses[0].phase = Phase::Updated;
        let rows = visible_rows(&devices, str::to_owned);
        assert_eq!(
            rows.iter()
                .map(|r| r.device_id.as_str())
                .collect::<Vec<_>>(),
            ["desktop", "laptop"]
        );
        assert_eq!(action(&rows[0].status).unwrap().0, "Cancel");
        assert_eq!(rows[1].status.phase, Phase::Updated);
    }

    #[test]
    fn disconnect_retains_notice_but_reconnect_waits_for_fresh_status() {
        let mut devices = BTreeMap::from([("laptop".into(), device(Phase::Available))]);
        assert!(
            reconcile_devices(&mut devices, &BTreeMap::from([("laptop".into(), false)])).is_empty()
        );
        let rows = visible_rows(&devices, str::to_owned);
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].connected);
        assert_eq!(rows[0].status.phase, Phase::Available);

        assert_eq!(
            reconcile_devices(&mut devices, &BTreeMap::from([("laptop".into(), true)])),
            ["laptop"]
        );
        assert!(!visible_rows(&devices, str::to_owned)[0].connected);
        devices.get_mut("laptop").unwrap().connected = true;
        assert!(visible_rows(&devices, str::to_owned)[0].connected);
    }

    #[test]
    fn presence_changes_only_restart_the_affected_host() {
        let mut devices = BTreeMap::from([
            ("desktop".into(), device(Phase::Available)),
            ("laptop".into(), device(Phase::Available)),
        ]);
        for updates in devices.values_mut() {
            updates.watch = Some(Task::ready(()));
        }
        let mut desired = BTreeMap::from([("desktop".into(), true), ("laptop".into(), true)]);
        assert!(reconcile_devices(&mut devices, &desired).is_empty());
        desired.insert("laptop".into(), false);
        assert!(reconcile_devices(&mut devices, &desired).is_empty());
        assert!(devices["desktop"].watch.is_some());
        assert!(devices["desktop"].connected);
        assert!(devices["laptop"].watch.is_none());
        desired.insert("laptop".into(), true);
        assert_eq!(reconcile_devices(&mut devices, &desired), ["laptop"]);
    }

    #[test]
    fn inventory_removes_departed_hosts_and_hides_current_agents() {
        let mut devices = BTreeMap::from([
            ("removed".into(), device(Phase::Available)),
            ("current".into(), device(Phase::Current)),
        ]);
        let start = reconcile_devices(
            &mut devices,
            &BTreeMap::from([
                ("current".into(), true),
                ("new".into(), true),
                ("offline".into(), false),
            ]),
        );
        assert_eq!(start, ["current", "new"]);
        assert!(!devices.contains_key("removed"));
        assert!(visible_rows(&devices, str::to_owned).is_empty());
        devices.get_mut("new").unwrap().statuses = vec![status(Phase::Available)];
        devices.get_mut("new").unwrap().connected = true;
        let rows = visible_rows(&devices, str::to_owned);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].device_id, "new");
    }
}

fn spawn_limit_watch(
    engine: crate::state::EngineHandle,
    state: Entity<AppState>,
    device: String,
    cx: &mut Context<Shell>,
) -> Task<()> {
    cx.spawn(async move |_, cx| {
        let mut retry = 1;
        loop {
            let result = engine
                .client()
                .subscribe_checked(
                    methods::WATCH_LIMIT_RESUMES,
                    serde_json::json!({ "targetDeviceId": device }),
                )
                .await;
            if let Ok(mut stream) = result {
                while let Some(value) = stream.recv().await {
                    let Ok(resumes) = serde_json::from_value::<zeron_proto::LimitResumes>(value)
                    else {
                        break;
                    };
                    state.update(cx, |state, cx| {
                        if state.limit_resumes.get(&device) != Some(&resumes) {
                            state.limit_resumes.insert(device.clone(), resumes);
                            cx.notify();
                        }
                    });
                    retry = 1;
                }
            }
            cx.background_executor()
                .timer(Duration::from_secs(retry))
                .await;
            retry = (retry * 2).min(15);
        }
    })
}

pub(super) struct UpdateAllPlan {
    app: Option<String>,
    devices: Vec<(String, String, Vec<HarnessId>)>,
    running: usize,
}

impl UpdateAllPlan {
    pub(super) fn items(&self) -> usize {
        usize::from(self.app.is_some()) + self.devices.iter().map(|d| d.2.len()).sum::<usize>()
    }

    fn title(&self) -> String {
        let agents: usize = self.devices.iter().map(|d| d.2.len()).sum();
        let parts: Vec<String> = self
            .app
            .as_ref()
            .map(|_| "this app".to_owned())
            .into_iter()
            .chain(
                (agents > 0)
                    .then(|| format!("{agents} {}", if agents == 1 { "agent" } else { "agents" })),
            )
            .collect();
        format!("Update {}?", parts.join(" and "))
    }

    fn lines(&self) -> Vec<String> {
        self.app
            .iter()
            .map(|version| format!("This app: {version}, restarts last"))
            .chain(self.devices.iter().map(|(_, name, harnesses)| {
                let agents: Vec<&str> = harnesses.iter().map(|h| h.display_name()).collect();
                format!("{name}: {}", agents.join(", "))
            }))
            .collect()
    }
}

fn settled(status: &HarnessUpdateStatus) -> bool {
    matches!(
        status.phase,
        Phase::Current | Phase::Updated | Phase::Failed | Phase::ManualActionRequired
    )
}

impl Shell {
    pub(super) fn update_all_plan(&self, cx: &App) -> UpdateAllPlan {
        let state = self.state.read(cx);
        let local = state.local_device_id.as_deref();
        let app = crate::app_update::AppUpdate::global(cx).and_then(|update| {
            let update = update.read(cx);
            match update.flow() {
                crate::app_update::Flow::Ready { version, .. } => Some(version.clone()),
                crate::app_update::Flow::Idle
                    if update.blocker().is_none() && update.install().supports_desktop_update() =>
                {
                    update.available().map(str::to_owned)
                }
                _ => None,
            }
        });
        let devices: Vec<(String, String, Vec<HarnessId>)> = self
            .harness_update_devices
            .iter()
            .filter(|(_, device)| device.online && device.connected)
            .filter_map(|(id, device)| {
                let harnesses: Vec<HarnessId> = device
                    .statuses
                    .iter()
                    .filter(|status| status.phase == Phase::Available && status.can_apply)
                    .map(|status| status.harness)
                    .collect();
                (!harnesses.is_empty()).then(|| {
                    (
                        id.clone(),
                        state.device_name(id).unwrap_or(id).to_owned(),
                        harnesses,
                    )
                })
            })
            .collect();
        let now = Utc::now();
        let running = state
            .chats
            .iter()
            .filter(|chat| matches!(state.indicator_for(&chat.id, now), Indicator::Working))
            .filter(|chat| {
                (app.is_some() && local == Some(chat.device_id.as_str()))
                    || chat.config.as_ref().is_some_and(|config| {
                        devices.iter().any(|(id, _, harnesses)| {
                            *id == chat.device_id && harnesses.contains(&config.harness)
                        })
                    })
            })
            .count();
        UpdateAllPlan {
            app,
            devices,
            running,
        }
    }

    pub(super) fn open_update_all(&mut self, cx: &mut Context<Self>) {
        self.update_all_dialog = Some(true);
        cx.notify();
    }

    fn start_update_all(&mut self, continue_threads: bool, cx: &mut Context<Self>) {
        self.update_all_dialog = None;
        let plan = self.update_all_plan(cx);
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let local = self.state.read(cx).local_device_id.clone();
        let restarts = plan.app.is_some();
        if restarts {
            let local_harnesses = plan
                .devices
                .iter()
                .find(|(id, _, _)| Some(id) == local.as_ref())
                .map(|(_, _, harnesses)| harnesses.clone())
                .unwrap_or_default();
            self.update_all_restart = Some((local_harnesses, false));
            if let Some(update) = crate::app_update::AppUpdate::global(cx)
                && matches!(update.read(cx).flow(), crate::app_update::Flow::Idle)
            {
                update.update(cx, |update, cx| update.start_download(cx));
            }
        }
        cx.spawn(async move |this, cx| {
            let client = engine.client();
            let mut failures = Vec::new();
            if restarts
                && let Err(error) = client
                    .call(
                        methods::PREPARE_RESTART,
                        serde_json::json!({ "continueThreads": continue_threads }),
                    )
                    .await
            {
                failures.push(error.to_string());
            }
            for (device, name, harnesses) in plan.devices {
                let continues = continue_threads && !(restarts && Some(&device) == local.as_ref());
                let params = serde_json::json!({
                    "harnesses": harnesses,
                    "continueThreads": continues,
                    "targetDeviceId": device,
                });
                if let Err(error) = client.call(methods::UPDATE_HARNESSES, params).await {
                    failures.push(format!("{name}: {error}"));
                }
            }
            this.update(cx, |shell, cx| {
                if let Some(pending) = shell.update_all_restart.as_mut() {
                    pending.1 = true;
                }
                if !failures.is_empty() {
                    shell.show_notice(
                        crate::toast::ToastKind::Error,
                        format!("Could not update everything: {}", failures.join("; ")),
                        cx,
                    );
                }
                shell.advance_update_all(cx);
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// The app restarts last: once the local agents have settled and the
    /// download is staged.
    pub(super) fn advance_update_all(&mut self, cx: &mut Context<Self>) {
        let Some((harnesses, requested)) = self.update_all_restart.as_ref() else {
            return;
        };
        if !requested {
            return;
        }
        let local = self.state.read(cx).local_device_id.clone();
        let agents_done = local
            .and_then(|id| self.harness_update_devices.get(&id))
            .is_none_or(|device| {
                harnesses.iter().all(|harness| {
                    device
                        .statuses
                        .iter()
                        .find(|status| status.harness == *harness)
                        .is_none_or(settled)
                })
            });
        let Some(update) = crate::app_update::AppUpdate::global(cx) else {
            self.update_all_restart = None;
            return;
        };
        match update.read(cx).flow().clone() {
            crate::app_update::Flow::Ready { staged, .. } if agents_done => {
                self.update_all_restart = None;
                self.apply_staged_update(staged, cx);
            }
            crate::app_update::Flow::Failed { message, .. } => {
                self.update_all_restart = None;
                self.show_notice(
                    crate::toast::ToastKind::Error,
                    format!("The app update failed: {message}"),
                    cx,
                );
            }
            _ => {}
        }
    }

    pub(super) fn render_update_all_dialog(
        &mut self,
        viewport: gpui::Size<Pixels>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let continue_threads = self.update_all_dialog?;
        let plan = self.update_all_plan(cx);
        if plan.items() == 0 {
            self.update_all_dialog = None;
            return None;
        }
        let theme = Theme::of(cx).for_popup();
        let running = (plan.running > 0).then(|| {
            let threads = if plan.running == 1 {
                "thread"
            } else {
                "threads"
            };
            div()
                .mt(px(14.0))
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(
                    div()
                        .text_size(crate::typography::ui_rems(13.0))
                        .text_color(theme.text)
                        .child(format!(
                            "{} running {threads} will be interrupted.",
                            plan.running
                        )),
                )
                .child(
                    div()
                        .id("update-all-continue")
                        .debug_selector(|| "update-all-continue".into())
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .cursor_pointer()
                        .role(gpui::Role::CheckBox)
                        .aria_label("Continue threads after restarts")
                        .aria_toggled(if continue_threads {
                            gpui::Toggled::True
                        } else {
                            gpui::Toggled::False
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.update_all_dialog = Some(!continue_threads);
                            cx.notify();
                        }))
                        .child(
                            div()
                                .size(px(16.0))
                                .flex_none()
                                .rounded(px(4.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .when(continue_threads, |el| el.bg(theme.accent))
                                .when(!continue_threads, |el| {
                                    el.border_1().border_color(theme.border_strong)
                                })
                                .when(continue_threads, |el| {
                                    el.child(
                                        icon(icons::CHECK)
                                            .size(px(12.0))
                                            .text_color(theme.on_solid),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .text_size(crate::typography::ui_rems(13.0))
                                .text_color(theme.text_muted)
                                .child("Continue threads after restarts"),
                        ),
                )
        });
        let buttons = div()
            .mt(px(18.0))
            .flex()
            .flex_row()
            .justify_end()
            .gap(px(8.0))
            .child(
                popover::btn_ghost(&theme, "Cancel", "update-all-cancel")
                    .id("update-all-cancel")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.update_all_dialog = None;
                        cx.notify();
                    })),
            )
            .child(
                popover::btn_primary(&theme, "Update")
                    .id("update-all-confirm")
                    .debug_selector(|| "update-all-confirm".into())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.start_update_all(continue_threads, cx)
                    })),
            );
        let card = popover::dialog_card(&theme)
            .child(popover::dialog_title(&theme, &plan.title()))
            .child(
                div().mt(px(6.0)).flex().flex_col().children(
                    plan.lines()
                        .into_iter()
                        .map(|line| popover::dialog_body(&theme, line)),
                ),
            )
            .children(running)
            .child(buttons)
            .into_any_element();
        Some(popover::modal("update-all-dialog", viewport, card))
    }
}

fn render_update_all_button(theme: &Theme, cx: &mut Context<Shell>) -> AnyElement {
    div()
        .id("harness-update-all")
        .debug_selector(|| "harness-update-all".into())
        .h(px(26.0))
        .px(px(9.0))
        .flex_none()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .text_size(crate::typography::ui_rems(11.5))
        .font_weight(gpui::FontWeight::MEDIUM)
        .bg(theme.text)
        .text_color(theme.on_solid)
        .role(gpui::Role::Button)
        .aria_label("Update all")
        .tab_index(0)
        .cursor_pointer()
        .hover(|el| el.opacity(0.85))
        .on_click(cx.listener(|this, _, _, cx| {
            cx.stop_propagation();
            this.open_update_all(cx);
        }))
        .child("Update all")
        .into_any_element()
}
