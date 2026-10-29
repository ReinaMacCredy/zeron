//! Appshots settings share the shortcut recorder and persistence events with
//! keyboard settings, while keeping capture setup on its own route. The page
//! mirrors Synara's AppSnap panel: an intro card, a Capture card, and a guided
//! macOS permissions card whose Grant buttons expand per-pane steps and watch
//! for the grant to land.
use std::time::{Duration, Instant};

use super::*;
use crate::appshots::{AppshotPlatform, CapabilityState, ChordModifier};
use crate::settings::permission_guide::{
    self, PermissionGuide, PermissionGuideHost, PermissionPane, choice_button, outline_button, row,
    section_label,
};
use crate::settings::widgets;

const APPSHOT_PANES: [(PermissionPane, &str); 3] = [
    (
        PermissionPane::InputMonitoring,
        "Lets Zeron notice the hotkey chord while another app owns the keyboard. Nothing you type is recorded.",
    ),
    (
        PermissionPane::ScreenRecording,
        "Lets Zeron capture an image of the frontmost window. Only the single window you snap is captured, only at the moment you press the chord.",
    ),
    (
        PermissionPane::Accessibility,
        "Optional. Lets Zeron read the captured window's visible and off-screen text so the agent gets more than pixels. Screenshots work without it.",
    ),
];

/// Whether the active hotkey is a modifier chord, whose event tap needs Input
/// Monitoring.
fn uses_chord(combo: &str) -> bool {
    ChordModifier::from_combo(combo).is_some()
}

/// The hotkey menu's entries in order: the three chords, then None.
const HOTKEY_CHOICES: [Option<ChordModifier>; 4] = [
    Some(ChordModifier::Command),
    Some(ChordModifier::Option),
    Some(ChordModifier::Shift),
    None,
];

fn hotkey_choice_combo(choice: Option<ChordModifier>) -> &'static str {
    choice.map_or(crate::appshots::NO_HOTKEY, ChordModifier::combo)
}

fn hotkey_choice_label(choice: Option<ChordModifier>) -> String {
    choice.map_or_else(|| "None".to_string(), ChordModifier::label)
}

/// Codex's hotkey row copy: "Press both ⌘ keys simultaneously".
fn hotkey_description(combo: &str) -> String {
    match ChordModifier::from_combo(combo) {
        Some(chord) => format!("Press both {} keys simultaneously", chord.glyph()),
        None => "No hotkey. Choose a chord to capture from other apps.".to_string(),
    }
}
/// Page-local state: the hotkey dropdown, plus the permissions card's shared
/// guide and the recheck that keeps it honest.
#[derive(Default)]
pub(super) struct PermissionsUi {
    hotkey_menu: popover::Popup<()>,
    hotkey_menu_dismissed_at: Option<Instant>,
    pub(super) guide: PermissionGuide,
    recheck: Option<gpui::Task<()>>,
    /// Transient line under the card ("Permission granted.", "Permissions
    /// unchanged."), replaced by the next action.
    notice: Option<SharedString>,
}

impl PermissionGuideHost for ShortcutsPage {
    fn permission_guide(&mut self) -> &mut PermissionGuide {
        &mut self.permissions.guide
    }

    fn guide_opening(&mut self) {
        self.permissions.notice = None;
    }
}

impl ShortcutsPage {
    fn enable_status(&self) -> SharedString {
        let capabilities = self.appshot_capabilities;
        if capabilities.platform == AppshotPlatform::Unsupported {
            return "Available on macOS and Linux only".into();
        }
        if !self.appshots_enabled {
            return "Off".into();
        }
        let combo = self.keymap.get(ShortcutId::CaptureAppshot);
        if capabilities.platform == AppshotPlatform::MacOs && combo == crate::appshots::NO_HOTKEY {
            return "No hotkey selected".into();
        }
        if uses_chord(combo) && capabilities.input_monitoring == CapabilityState::PermissionRequired
        {
            return "Allow Input Monitoring in macOS System Settings, then try again.".into();
        }
        if capabilities.window_capture == CapabilityState::PermissionRequired {
            return "Allow Screen Recording in macOS System Settings, then try again.".into();
        }
        if capabilities.global_shortcut == CapabilityState::Checking {
            return "Starting the capture listener…".into();
        }
        if !capabilities.global_shortcut.is_ready() {
            return "This shortcut is unavailable. Choose a different key combination.".into();
        }
        format!("Listening — press {} to snap", display_combo(combo)).into()
    }

    fn open_guide(&mut self, pane: PermissionPane, cx: &mut Context<Self>) {
        self.permissions.notice = None;
        self.permissions.guide.open(pane, cx);
    }

    fn set_appshots_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.appshots_enabled = enabled;
        self.commit_appshots(cx);
        if enabled {
            // Synara requests the permissions from the toggle, then opens the
            // first missing pane's guide when a grant did not land.
            let chord = uses_chord(self.keymap.get(ShortcutId::CaptureAppshot));
            let missing = |capabilities: AppshotCapabilities| {
                if chord && capabilities.input_monitoring == CapabilityState::PermissionRequired {
                    Some(PermissionPane::InputMonitoring)
                } else if capabilities.window_capture == CapabilityState::PermissionRequired {
                    Some(PermissionPane::ScreenRecording)
                } else {
                    None
                }
            };
            if let Some(pane) = missing(self.appshot_capabilities) {
                self.permissions.guide.request_once(pane);
                crate::appshots::refresh_registration();
                self.appshot_capabilities = crate::appshots::capabilities();
                if let Some(pane) = missing(self.appshot_capabilities) {
                    self.open_guide(pane, cx);
                }
            }
        }
        cx.notify();
    }

    fn recheck_permissions(&mut self, cx: &mut Context<Self>) {
        if self.permissions.recheck.is_some() {
            return;
        }
        self.permissions.notice = None;
        // The preflight is synchronous; the short hold keeps the "Rechecking…"
        // state readable instead of flickering for one frame.
        self.permissions.recheck = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(350))
                .await;
            this.update(cx, |this, cx| {
                crate::appshots::refresh_registration();
                this.appshot_capabilities = crate::appshots::capabilities();
                let chord = uses_chord(this.keymap.get(ShortcutId::CaptureAppshot));
                let missing = [
                    PermissionPane::InputMonitoring,
                    PermissionPane::ScreenRecording,
                ]
                .into_iter()
                .filter(|pane| chord || *pane != PermissionPane::InputMonitoring)
                .any(|pane| pane.state(this.appshot_capabilities) != CapabilityState::Ready);
                this.permissions.notice = missing.then(|| {
                    "Permissions unchanged. Use Grant next to a permission to walk through setup."
                        .into()
                });
                this.permissions.recheck = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    pub(super) fn restore_appshot_defaults(&mut self, cx: &mut Context<Self>) {
        self.keymap.reset(ShortcutId::CaptureAppshot);
        self.stop_recording();
        self.conflict_notice = None;
        self.commit(cx);
        self.appshots_enabled = false;
        self.appshot_sound_enabled = true;
        self.appshot_destination = AppshotDestination::default();
        self.commit_appshots(cx);
        cx.notify();
    }

    fn set_hotkey(&mut self, choice: Option<ChordModifier>, cx: &mut Context<Self>) {
        self.close_hotkey_menu(cx);
        self.keymap.set(
            ShortcutId::CaptureAppshot,
            hotkey_choice_combo(choice).into(),
        );
        self.conflict_notice = None;
        self.commit(cx);
    }

    #[cfg(test)]
    pub(super) fn permissions_hotkey_menu_open(&self) -> bool {
        self.permissions.hotkey_menu.is_open()
    }

    fn close_hotkey_menu(&mut self, cx: &mut Context<Self>) {
        if self.permissions.hotkey_menu.begin_close() {
            popover::reap_popup(cx, |page| &mut page.permissions.hotkey_menu);
        }
    }

    fn dismiss_hotkey_menu(&mut self, cx: &mut Context<Self>) {
        self.permissions.hotkey_menu_dismissed_at = Some(Instant::now());
        self.close_hotkey_menu(cx);
    }

    fn toggle_hotkey_menu(&mut self, cx: &mut Context<Self>) {
        let just_dismissed = self
            .permissions
            .hotkey_menu_dismissed_at
            .take()
            .is_some_and(|at| at.elapsed() < Duration::from_millis(400));
        if self.permissions.hotkey_menu.is_open() {
            self.close_hotkey_menu(cx);
        } else if !just_dismissed {
            self.permissions.hotkey_menu.open(());
        }
        cx.notify();
    }

    /// Codex's hotkey select: the current chord with a chevron, opening a menu
    /// of the three double-modifier chords and None, the current one checked.
    pub(super) fn render_hotkey_dropdown(
        &self,
        slug: &'static str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let accent = theme.accent;
        let current = ChordModifier::from_combo(self.keymap.get(ShortcutId::CaptureAppshot));
        let rows: Vec<gpui::AnyElement> = HOTKEY_CHOICES
            .into_iter()
            .enumerate()
            .map(|(ix, choice)| {
                let selected = choice == current;
                popover::menu_row_nav(
                    theme,
                    selected,
                    false,
                    format!("{slug}-appshot-hotkey-option-{ix}"),
                )
                .id(SharedString::from(format!(
                    "{slug}-appshot-hotkey-option-{ix}"
                )))
                .debug_selector(move || format!("{slug}-appshot-hotkey-option-{ix}"))
                .role(gpui::Role::MenuItem)
                .aria_label(hotkey_choice_label(choice))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.set_hotkey(choice, cx);
                }))
                .child(
                    div()
                        .flex_1()
                        .child(SharedString::from(hotkey_choice_label(choice))),
                )
                .child(div().w(px(18.0)).flex_none().when(selected, |slot| {
                    slot.child(
                        crate::icons::icon(crate::icons::CHECK)
                            .size(px(14.0))
                            .text_color(theme.text),
                    )
                }))
                .into_any_element()
            })
            .collect();
        let menu = popover::popover_card(theme)
            .w(px(150.0))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.dismiss_hotkey_menu(cx)))
            .flex()
            .flex_col()
            .gap(px(2.0))
            .children(rows)
            .into_any_element();
        let open = self.permissions.hotkey_menu.is_open();
        let closing = self.permissions.hotkey_menu.closing_since();
        div()
            .id(SharedString::from(format!("{slug}-appshot-hotkey")))
            .debug_selector(move || format!("{slug}-appshot-hotkey"))
            .role(gpui::Role::Button)
            .aria_label(format!("Appshot hotkey: {}", hotkey_choice_label(current)))
            .tab_index(0)
            .focus_visible(move |style| style.border_2().border_color(accent))
            .relative()
            .flex_none()
            .h(px(32.0))
            .min_w(px(112.0))
            .px(px(12.0))
            .rounded(px(9.0))
            .border_1()
            .border_color(if open {
                theme.border_strong
            } else {
                theme.border
            })
            .bg(crate::theme::ink(0.05))
            .flex()
            .items_center()
            .gap(px(10.0))
            .cursor_pointer()
            .when(!open, |el| el.hover(|s| s.bg(crate::theme::ink(0.08))))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_hotkey_menu(cx)))
            .child(
                div()
                    .flex_1()
                    .text_size(crate::typography::ui_rems(13.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(SharedString::from(hotkey_choice_label(current))),
            )
            .child(
                crate::icons::icon(crate::icons::ALT_ARROW_DOWN)
                    .size(px(14.0))
                    .flex_none()
                    .text_color(theme.text_muted),
            )
            .when_some(self.permissions.hotkey_menu.get(), |trigger, _| {
                trigger.child(popover::anchored_menu_below_end(
                    SharedString::from(format!("{slug}-appshot-hotkey-menu")),
                    menu,
                    closing,
                ))
            })
            .into_any_element()
    }

    pub(super) fn render_appshots(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        #[cfg(test)]
        self.permissions.guide.rendered.clear();
        let theme = Theme::of(cx).for_settings_surface();
        let accent = theme.accent;
        let capabilities = self.appshot_capabilities;
        let supported = capabilities.platform != AppshotPlatform::Unsupported;
        let app = "Zeron";
        let combo = self.keymap.get(ShortcutId::CaptureAppshot).to_string();
        let mac = capabilities.platform == AppshotPlatform::MacOs;

        if let Some(pane) = self.permissions.guide.close_if_granted(capabilities, cx) {
            self.permissions.notice = Some(
                format!(
                    "Permission granted. {} is ready for Appshots.",
                    pane.label()
                )
                .into(),
            );
        }

        let toggle = |id: &'static str, label: &'static str, enabled: bool| {
            widgets::toggle_switch(&theme, enabled, id)
                .id(id)
                .role(gpui::Role::Switch)
                .aria_label(label)
                .aria_toggled(if enabled {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                })
                .tab_index(0)
                .focus_visible(move |style| style.border_2().border_color(accent))
        };
        let reset_glyph = |id: &'static str, label: String| {
            widgets::ghost_action(&theme)
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(format!("Reset {label} to default"))
                .tab_index(0)
                .focus_visible(move |style| style.border_2().border_color(accent))
                .flex_none()
                .size(px(20.0))
                .p(px(0.0))
                .rounded(px(6.0))
                .justify_center()
                .child(
                    crate::icons::icon(crate::icons::RESTART)
                        .size(px(12.0))
                        .text_color(theme.text_muted),
                )
        };

        let intro = widgets::section_card(&theme)
            .child(
                div()
                    .px(px(16.0))
                    .py(px(14.0))
                    .flex()
                    .items_start()
                    .gap(px(12.0))
                    .child(
                        div()
                            .flex_none()
                            .mt(px(2.0))
                            .size(px(32.0))
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(theme.border)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                crate::icons::icon(crate::icons::SCREEN_CAPTURE)
                                    .size(px(16.0))
                                    .text_color(theme.text_muted),
                            ),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .child(widgets::row_title(
                                &theme,
                                "Take an Appshot to show your agent another app's window",
                            ).whitespace_normal())
                            .child(
                                div()
                                    .text_size(crate::typography::ui_rems(12.5))
                                    .line_height(px(19.0))
                                    .text_color(theme.text_muted)
                                    .child(SharedString::from(format!(
                                        "Press your hotkey while any app is frontmost. {app} captures that window as an image, brings itself forward, and attaches the snap to a session composer — the capture stays on this device until you send the message."
                                    ))),
                            )
                            .when(!supported, |el| {
                                el.child(
                                    div()
                                        .pt(px(2.0))
                                        .text_size(crate::typography::ui_rems(12.5))
                                        .text_color(theme.text_muted)
                                        .child("Appshots are available only in the desktop app."),
                                )
                            }),
                    ),
            );

        let enabled = supported && self.appshots_enabled;
        let capture = toggle("appshots-enabled", "Enable Appshots", enabled)
            .when(!supported, |el| el.opacity(0.4))
            .when(supported, |el| {
                el.on_click(cx.listener(|this, _, _, cx| {
                    let next = !this.appshots_enabled;
                    this.set_appshots_enabled(next, cx);
                }))
            });
        let enable_reset = self.appshots_enabled.then(|| {
            reset_glyph("appshots-enabled-reset", "Appshots".into())
                .on_click(cx.listener(|this, _, _, cx| this.set_appshots_enabled(false, cx)))
                .into_any_element()
        });

        let non_default = combo != ShortcutId::CaptureAppshot.default_combo();
        let hotkey_reset = non_default.then(|| {
            reset_glyph("appshot-hotkey-reset", "hotkey".into())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.keymap.reset(ShortcutId::CaptureAppshot);
                    this.conflict_notice = None;
                    this.commit(cx);
                }))
                .into_any_element()
        });
        // macOS picks a modifier chord from a menu; Linux records a key combo
        // with the shared shortcut recorder.
        let hotkey_control = if mac {
            self.render_hotkey_dropdown("appshots", &theme, cx)
        } else {
            self.render_binding_control(ShortcutId::CaptureAppshot, 0, self.recording, &theme, cx)
                .into_any_element()
        };
        let hotkey_description = if mac {
            hotkey_description(&combo)
        } else {
            "Press the shortcut while another application has focus.".to_string()
        };

        let destinations = AppshotDestination::ALL
            .into_iter()
            .enumerate()
            .map(|(ix, destination)| {
                let selected = self.appshot_destination == destination;
                choice_button(
                    &theme,
                    ("appshot-destination", ix),
                    destination.label(),
                    selected,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.appshot_destination = destination;
                    this.commit_appshots(cx);
                    cx.notify();
                }))
            })
            .collect::<Vec<_>>();
        let destination_description = match self.appshot_destination {
            AppshotDestination::Automatic => {
                "Snaps join the session you interacted with in the last minute, and consecutive snaps stay together. Otherwise Zeron opens a fresh session with the capture attached."
            }
            AppshotDestination::LastSession => {
                "Use the open session, or return to the last session used for an Appshot."
            }
            AppshotDestination::NewSession => {
                "Stage captures in a new-session composer, keeping existing drafts intact."
            }
        };
        let destination_reset =
            (self.appshot_destination != AppshotDestination::default()).then(|| {
                reset_glyph("appshot-destination-reset", "destination".into())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.appshot_destination = AppshotDestination::default();
                        this.commit_appshots(cx);
                        cx.notify();
                    }))
                    .into_any_element()
            });

        let sound = toggle(
            "appshot-sound-enabled",
            "Play a sound when an Appshot is captured",
            self.appshot_sound_enabled,
        )
        .on_click(cx.listener(|this, _, _, cx| {
            this.appshot_sound_enabled = !this.appshot_sound_enabled;
            this.commit_appshots(cx);
            cx.notify();
        }));
        let sound_control = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(
                outline_button(&theme)
                    .id("appshot-sound-preview")
                    .role(gpui::Role::Button)
                    .aria_label("Preview the capture sound")
                    .tab_index(0)
                    .focus_visible(move |style| style.border_2().border_color(accent))
                    .on_click(|_, _, _| {
                        crate::sound::prepare_appshot();
                        crate::sound::play_appshot();
                    })
                    .child("Preview"),
            )
            .child(sound);
        let sound_reset = (!self.appshot_sound_enabled).then(|| {
            reset_glyph("appshot-sound-reset", "capture sound".into())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.appshot_sound_enabled = true;
                    this.commit_appshots(cx);
                    cx.notify();
                }))
                .into_any_element()
        });

        let capture_card = widgets::section_card(&theme)
            .mt(px(6.0))
            .child(
                row(
                    &theme,
                    true,
                    "Enable Appshots",
                    &format!("Run the capture listener in the background while {app} is open."),
                    Some(self.enable_status()),
                    enable_reset,
                    capture.into_any_element(),
                    None,
                )
                .search_anchor(SettingId::CaptureAppshots),
            )
            .child(
                row(
                    &theme,
                    false,
                    "Hotkey",
                    &hotkey_description,
                    None,
                    hotkey_reset,
                    hotkey_control,
                    None,
                )
                .search_anchor(ShortcutId::CaptureAppshot),
            )
            .child(
                row(
                    &theme,
                    false,
                    "Destination",
                    destination_description,
                    None,
                    destination_reset,
                    div()
                        .flex()
                        .flex_wrap()
                        .justify_end()
                        .gap(px(6.0))
                        .children(destinations)
                        .into_any_element(),
                    None,
                )
                .search_anchor(SettingId::AppshotDestination),
            )
            .child(
                row(
                    &theme,
                    false,
                    "Capture sound",
                    "Play a short shutter cue when a window is captured.",
                    None,
                    sound_reset,
                    sound_control.into_any_element(),
                    None,
                )
                .search_anchor(SettingId::AppshotSound),
            );

        let mut permissions_card = widgets::section_card(&theme).mt(px(6.0));
        let panes: &[(PermissionPane, &str)] = if capabilities.platform == AppshotPlatform::MacOs {
            &APPSHOT_PANES
        } else {
            &APPSHOT_PANES[1..]
        };
        for (element, (pane, _)) in permission_guide::permission_rows(
            self,
            "appshot",
            panes,
            capabilities,
            &theme,
            window,
            cx,
        )
        .into_iter()
        .zip(panes)
        {
            permissions_card = permissions_card.child(match pane {
                PermissionPane::ScreenRecording => element
                    .search_anchor(SettingId::WindowCapture)
                    .into_any_element(),
                PermissionPane::Accessibility => element
                    .search_anchor(SettingId::ApplicationText)
                    .into_any_element(),
                PermissionPane::InputMonitoring => element
                    .search_anchor(SettingId::InputMonitoring)
                    .into_any_element(),
            });
        }
        let rechecking = self.permissions.recheck.is_some();
        let mut recheck = outline_button(&theme)
            .id("appshots-refresh-permissions")
            .role(gpui::Role::Button)
            .aria_label("Recheck permissions")
            .tab_index(0)
            .focus_visible(move |style| style.border_2().border_color(accent))
            .when(rechecking, |el| el.opacity(0.6))
            .on_click(cx.listener(|this, _, _, cx| this.recheck_permissions(cx)));
        if rechecking {
            recheck = recheck
                .child(crate::loaders::mini_mono_spinner(
                    "appshots-recheck-spinner",
                    2.5,
                    theme.text_muted,
                    cx.entity_id(),
                    cx,
                ))
                .child("Rechecking…");
        } else {
            recheck = recheck.child("Recheck permissions");
        }
        permissions_card = permissions_card.child(
            row(
                &theme,
                false,
                "Permission status",
                "Grant each permission with the steps above. If you take longer than 10 minutes, press Grant again.",
                self.permissions.notice.clone(),
                None,
                recheck.into_any_element(),
                None,
            )
            .search_anchor(SettingId::AppshotPermissions),
        );

        let customized = non_default
            || self.appshots_enabled
            || !self.appshot_sound_enabled
            || self.appshot_destination != AppshotDestination::default();
        let restore_disabled = !customized;
        let restore = widgets::ghost_action(&theme)
            .id("appshots-restore-defaults")
            .role(gpui::Role::Button)
            .aria_label("Restore Appshot defaults")
            .aria_disabled(restore_disabled)
            .when(!restore_disabled, |el| {
                el.tab_index(0)
                    .focus_visible(move |style| style.border_2().border_color(accent))
            })
            .flex_none()
            .h(px(28.0))
            .border_1()
            .border_color(theme.border)
            .when(restore_disabled, |el| el.opacity(0.35))
            .when(!restore_disabled, |el| {
                el.on_click(cx.listener(|this, _, _, cx| this.restore_appshot_defaults(cx)))
            })
            .child(
                crate::icons::icon(crate::icons::RESTART)
                    .size(px(14.0))
                    .text_color(theme.text_muted),
            )
            .child(SharedString::from("Restore defaults"));

        let helper: SharedString = if self.recording.is_some() {
            "Press Escape to cancel.".into()
        } else {
            self.conflict_notice.clone().unwrap_or_default()
        };
        let scrollbar = self.render_scrollbar(&theme, cx);
        div()
            .id("appshots-settings-page-host")
            .relative()
            .size_full()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .on_drag_move(cx.listener(Self::on_bar_drag_move))
            .child(
                crate::edge_fade::edge_faded(16.0, true, true, div()
                    .id("appshots-settings-page")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll.scroll)
                    .track_focus(&self.focus)
                    .tab_group()
                    .tab_index(0)
                    .tab_stop(false)
                    .on_key_down(|event, window, cx| {
                        let key = &event.keystroke;
                        if key.key == "tab"
                            && !key.modifiers.control
                            && !key.modifiers.alt
                            && !key.modifiers.platform
                        {
                            if key.modifiers.shift {
                                window.focus_prev(cx);
                            } else {
                                window.focus_next(cx);
                            }
                            cx.stop_propagation();
                        }
                    })
                    .child(
                        widgets::page_column()
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_start()
                                    .flex_wrap()
                                    .justify_between()
                                    .gap(px(24.0))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(200.0))
                                            .flex()
                                            .flex_col()
                                            .child(widgets::page_header(&theme, "Appshots", None))
                                            .child(
                                                widgets::page_subtitle(
                                                    &theme,
                                                    "Capture another app's frontmost window directly into a session.",
                                                )
                                                .max_w(px(512.0))
                                                .line_height(px(20.0)),
                                            ),
                                    )
                                    .child(restore),
                            )
                            .child(intro)
                            .child(section_label(&theme, "Capture"))
                            .child(capture_card)
                            .when(supported, |el| {
                                let label = if mac { "macOS permissions" } else { "Permissions" };
                                el.child(section_label(&theme, label))
                                    .child(permissions_card)
                            })
                            .child(
                                div()
                                    .min_h(px(20.0))
                                    .mt(px(8.0))
                                    .text_size(crate::typography::ui_rems(12.0))
                                    .text_color(theme.text_muted)
                                    .child(helper),
                            ),
                    )).fade_overflow_y(&self.scroll.scroll),
            )
            .child(widgets::page_reveal(&self.scroll.scroll))
            .children(scrollbar)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkey_row_describes_the_chosen_chord() {
        assert_eq!(
            hotkey_description(ChordModifier::Command.combo()),
            "Press both ⌘ keys simultaneously"
        );
        assert_eq!(
            hotkey_description(ChordModifier::Shift.combo()),
            "Press both ⇧ keys simultaneously"
        );
        assert!(hotkey_description(crate::appshots::NO_HOTKEY).starts_with("No hotkey"));
        assert_eq!(hotkey_choice_label(None), "None");
        assert_eq!(
            hotkey_choice_combo(Some(ChordModifier::Option)),
            "option-option"
        );
    }
}
