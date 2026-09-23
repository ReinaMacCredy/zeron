//! Settings → Shortcuts (feature-inventory §1.4): a table of the rebindable
//! bindings — click a combo to record (Esc cancels), live conflict detection,
//! per-row Reset and Restore defaults. Changes emit [`ShortcutsEvent`]; the
//! shell persists them and re-applies the app keymap.

use gpui::{
    Context, Entity, EventEmitter, FocusHandle, Keystroke, SharedString, Window, div, prelude::*,
    px,
};

use crate::appshots::{AppshotCapabilities, AppshotDestination};
use crate::popover::{self, ScrollRailHost};

#[path = "appshots.rs"]
mod appshots_page;
use crate::settings::widgets;
use crate::settings::{
    ComposerSendBehavior, FollowUpBehavior, KeymapConfig, ShortcutId, SwitcherWindow,
    combo_from_keystroke, display_combo,
};
use crate::state::AppState;
use crate::theme::Theme;

/// A cap of 1 would leave only the open session and Ctrl+Tab would go
/// nowhere, so the stepper goes from "All" straight to 2.
const SWITCHER_LIMIT_MIN: u32 = 2;
const SWITCHER_LIMIT_MAX: u32 = 50;

/// Outcome of one keystroke while recording. Pure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordOutcome {
    /// Esc — abandon recording, keep the old combo.
    Cancelled,
    /// A bare modifier (or unusable key) — stay recording.
    Ignored,
    /// A full combo landed.
    Set(String),
}

pub fn record_key(key: &str, ctrl: bool, alt: bool, shift: bool, cmd: bool) -> RecordOutcome {
    if key.eq_ignore_ascii_case("escape") {
        return RecordOutcome::Cancelled;
    }
    match combo_from_keystroke(ctrl, alt, shift, cmd, key) {
        Some(combo) => RecordOutcome::Set(combo),
        None => RecordOutcome::Ignored,
    }
}

#[derive(Debug, Clone)]
pub enum ShortcutsEvent {
    /// The keymap changed — persist + re-apply.
    KeymapChanged(KeymapConfig),
    /// The Escape fallback changed — persist it locally.
    EscapeStopsActiveAgentChanged(bool),
    /// The composer send behavior changed — persist + re-apply.
    ComposerSendBehaviorChanged(ComposerSendBehavior),
    /// Busy-turn submits queue or steer by default — persist it locally.
    FollowUpBehaviorChanged(FollowUpBehavior),
    /// The switcher's recency window changed — persist it locally.
    SwitcherWindowChanged(SwitcherWindow),
    /// The switcher's card cap changed — persist it locally.
    SwitcherLimitChanged(u32),
    AppshotsChanged {
        enabled: bool,
        sound_enabled: bool,
        destination: AppshotDestination,
    },
}

pub struct ShortcutsPage {
    appshots_page: bool,
    appshots_focus_pending: bool,
    scroll: crate::settings::widgets::PageScroll,
    /// Working copy (kept in sync with the shell via change events).
    keymap: KeymapConfig,
    escape_stops_active_agent: bool,
    composer_send_behavior: ComposerSendBehavior,
    follow_up_behavior: FollowUpBehavior,
    switcher_window: SwitcherWindow,
    switcher_limit: u32,
    recording: Option<ShortcutId>,
    recording_blur: Option<gpui::Subscription>,
    recording_interceptor: Option<gpui::Subscription>,
    /// A rejected record attempt ("{Combo} is already assigned to {label}.") —
    /// conflicts never persist; they're refused at record time, as in zeron.
    conflict_notice: Option<SharedString>,
    focus: FocusHandle,
    appshots_enabled: bool,
    appshot_sound_enabled: bool,
    appshot_destination: AppshotDestination,
    appshot_capabilities: AppshotCapabilities,
    permissions: appshots_page::PermissionsUi,
    // The page never talks RPC; state is kept for parity with sibling pages
    // (and future per-device keymaps).
    _state: Entity<AppState>,
}

impl EventEmitter<ShortcutsEvent> for ShortcutsPage {}

impl ShortcutsPage {
    pub fn new(
        state: Entity<AppState>,
        keymap: KeymapConfig,
        escape_stops_active_agent: bool,
        composer_send_behavior: ComposerSendBehavior,
        follow_up_behavior: FollowUpBehavior,
        switcher_window: SwitcherWindow,
        switcher_limit: u32,
        appshots_enabled: bool,
        appshot_sound_enabled: bool,
        appshot_destination: AppshotDestination,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.on_release(|_, _| crate::appshots::set_recording(false))
            .detach();
        Self {
            appshots_page: false,
            appshots_focus_pending: false,
            scroll: crate::settings::widgets::PageScroll::default(),
            keymap,
            escape_stops_active_agent,
            composer_send_behavior,
            follow_up_behavior,
            switcher_window,
            switcher_limit,
            recording: None,
            recording_blur: None,
            recording_interceptor: None,
            conflict_notice: None,
            focus: cx.focus_handle(),
            appshots_enabled,
            appshot_sound_enabled,
            appshot_destination,
            appshot_capabilities: crate::appshots::capabilities(),
            permissions: appshots_page::PermissionsUi::default(),
            _state: state,
        }
    }

    pub fn show_appshots(&mut self, appshots: bool) {
        if self.appshots_page != appshots {
            self.stop_recording();
            self.conflict_notice = None;
            self.appshots_page = appshots;
            self.appshots_focus_pending = appshots;
            // One scroll state serves both pages — rewind it so each opens
            // at the top instead of where the other was left.
            self.scroll.reset();
        }
    }

    fn start_recording(&mut self, id: ShortcutId, window: &mut Window, cx: &mut Context<Self>) {
        self.recording = Some(id);
        crate::appshots::set_recording(true);
        let page = cx.entity().downgrade();
        // Bound actions run before Div key listeners. Intercept first so a
        // conflicting chord is recorded/refused instead of running its action.
        self.recording_interceptor = Some(cx.intercept_keystrokes(move |event, window, cx| {
            let _ = page.update(cx, |page, cx| {
                if page.focus.is_focused(window) {
                    page.record_keystroke(&event.keystroke, cx);
                }
            });
        }));
        self.recording_blur = Some(cx.on_blur(&self.focus, window, |this, _, cx| {
            this.stop_recording();
            cx.notify();
        }));
        self.conflict_notice = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn stop_recording(&mut self) {
        self.recording = None;
        self.recording_blur = None;
        self.recording_interceptor = None;
        crate::appshots::set_recording(false);
    }

    fn adopt_recorded_combo(
        &mut self,
        recording: ShortcutId,
        combo: String,
        cx: &mut Context<Self>,
    ) {
        // A combo already bound elsewhere is REFUSED, naming the owner
        // (zeron settings.shortcuts.tsx: "… is already assigned to …").
        if let Some(owner) = conflict_owner(&self.keymap, recording, &combo) {
            self.conflict_notice = Some(
                format!(
                    "{} is already assigned to {}.",
                    display_combo(&combo),
                    owner.label()
                )
                .into(),
            );
            self.stop_recording();
            cx.notify();
        } else {
            self.keymap.set(recording, combo);
            self.stop_recording();
            self.conflict_notice = None;
            self.commit(cx);
        }
    }

    fn commit(&mut self, cx: &mut Context<Self>) {
        cx.emit(ShortcutsEvent::KeymapChanged(self.keymap.clone()));
        cx.notify();
    }

    fn set_escape_stops_active_agent(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.escape_stops_active_agent != enabled {
            self.escape_stops_active_agent = enabled;
            cx.emit(ShortcutsEvent::EscapeStopsActiveAgentChanged(enabled));
            cx.notify();
        }
    }

    fn set_composer_send_behavior(
        &mut self,
        behavior: ComposerSendBehavior,
        cx: &mut Context<Self>,
    ) {
        if self.composer_send_behavior != behavior {
            self.composer_send_behavior = behavior;
            self.conflict_notice = None;
            cx.emit(ShortcutsEvent::ComposerSendBehaviorChanged(behavior));
            cx.notify();
        }
    }

    fn set_follow_up_behavior(&mut self, behavior: FollowUpBehavior, cx: &mut Context<Self>) {
        if self.follow_up_behavior != behavior {
            self.follow_up_behavior = behavior;
            cx.emit(ShortcutsEvent::FollowUpBehaviorChanged(behavior));
            cx.notify();
        }
    }

    fn set_switcher_window(&mut self, window: SwitcherWindow, cx: &mut Context<Self>) {
        if self.switcher_window != window {
            self.switcher_window = window;
            cx.emit(ShortcutsEvent::SwitcherWindowChanged(window));
            cx.notify();
        }
    }

    fn set_switcher_limit(&mut self, limit: u32, cx: &mut Context<Self>) {
        if self.switcher_limit != limit {
            self.switcher_limit = limit;
            cx.emit(ShortcutsEvent::SwitcherLimitChanged(limit));
            cx.notify();
        }
    }

    fn commit_appshots(&self, cx: &mut Context<Self>) {
        cx.emit(ShortcutsEvent::AppshotsChanged {
            enabled: self.appshots_enabled,
            sound_enabled: self.appshot_sound_enabled,
            destination: self.appshot_destination,
        });
    }

    fn record_keystroke(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) {
        let Some(recording) = self.recording else {
            return;
        };
        let mods = &keystroke.modifiers;
        match record_key(
            &keystroke.key,
            mods.control,
            mods.alt,
            mods.shift,
            mods.platform,
        ) {
            RecordOutcome::Cancelled => {
                self.stop_recording();
                cx.notify();
            }
            RecordOutcome::Ignored => {}
            RecordOutcome::Set(combo) => {
                // Linux records the Appshot key combo here; macOS picks a
                // modifier chord from a menu instead and never records.
                if recording == ShortcutId::CaptureAppshot
                    && crate::appshots::validate_shortcut(&combo).is_err()
                {
                    self.conflict_notice = Some(
                        "Use Control or Alt with a letter, number, function key or navigation key."
                            .into(),
                    );
                    self.stop_recording();
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                if send_combo_is_reserved(self.composer_send_behavior, &combo) {
                    self.conflict_notice = Some(
                        format!("{} is reserved for the composer.", display_combo(&combo)).into(),
                    );
                    self.stop_recording();
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                self.adopt_recorded_combo(recording, combo, cx);
            }
        }
        cx.stop_propagation();
    }

    /// One shortcut row: label + description left, Reset when customized, and
    /// the click-to-record combo chip (recording inverts it to
    /// white-on-black). `ix` is the id's position in [`ShortcutId::ALL`]
    /// (unique element ids across the group cards); `gx` is the row's place
    /// in its own card (separator rule).
    fn render_row(
        &self,
        id: ShortcutId,
        ix: usize,
        gx: usize,
        recording: Option<ShortcutId>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        // zeron settings.shortcuts.tsx row: min-h-[72px] px-5 gap-5.
        div()
            .min_h(px(72.0))
            .px(px(20.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(20.0))
            .when(gx > 0, |el| el.border_t_1().border_color(theme.border))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(crate::typography::ui_rems(13.0))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child(SharedString::from(id.label())),
                    )
                    .child(
                        div()
                            .mt(px(2.0))
                            .text_size(crate::typography::ui_rems(12.0))
                            .text_color(theme.text_muted)
                            .child(SharedString::from(description(id))),
                    ),
            )
            .child(
                if id == ShortcutId::CaptureAppshot && cfg!(target_os = "macos") {
                    self.render_hotkey_dropdown("shortcuts", theme, cx)
                } else {
                    self.render_binding_control(id, ix, recording, theme, cx)
                        .into_any_element()
                },
            )
    }

    fn render_binding_control(
        &self,
        id: ShortcutId,
        ix: usize,
        recording: Option<ShortcutId>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let accent = theme.accent;
        let combo = self.keymap.get(id).to_string();
        let is_recording = recording == Some(id);
        let non_default = combo != id.default_combo();
        let chip_text: SharedString = if is_recording {
            "Press keys…".into()
        } else {
            display_combo(&combo).into()
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(20.0))
            .when(non_default && !is_recording, |el| {
                el.child(
                    div()
                        .id(("shortcut-reset", ix))
                        .role(gpui::Role::Button)
                        .aria_label(format!("Reset {} shortcut", id.label()))
                        .min_h(px(24.0))
                        .tab_index(0)
                        .focus_visible(move |style| style.border_2().border_color(accent))
                        .text_size(crate::typography::ui_rems(11.0))
                        .text_color(theme.text_muted.opacity(0.7))
                        .cursor_pointer()
                        .hover(|s| s.text_color(theme.text))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.keymap.reset(id);
                            this.stop_recording();
                            this.commit(cx);
                        }))
                        .child(SharedString::from("Reset")),
                )
            })
            .child(
                div()
                    .id(("shortcut-combo", ix))
                    .role(gpui::Role::Button)
                    .aria_label(format!(
                        "Change {} shortcut: {}",
                        id.label(),
                        display_combo(&combo)
                    ))
                    .tab_index(0)
                    .focus_visible(move |style| style.border_2().border_color(accent))
                    .min_w(px(96.0))
                    .px(px(12.0))
                    .py(px(6.0))
                    .rounded(px(8.0))
                    .border_1()
                    .flex()
                    .justify_center()
                    .font_family(theme.font_mono.clone())
                    .text_size(crate::typography::ui_rems(12.0))
                    .cursor_pointer()
                    .map(|el| {
                        if is_recording {
                            el.border_color(theme.text.opacity(0.3))
                                .bg(theme.text)
                                .text_color(theme.on_solid)
                        } else {
                            el.border_color(theme.border)
                                .bg(theme.bg)
                                .text_color(theme.text)
                                .hover(|s| {
                                    // `hover:border-foreground/20` — the
                                    // neutral foreground, not pure white.
                                    s.border_color(theme.text.opacity(0.2))
                                        .bg(crate::theme::ink(0.03))
                                })
                        }
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.start_recording(id, window, cx);
                    }))
                    .child(chip_text),
            )
    }

    fn on_scroll_hovered(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.scroll.set_list_hovered(*hovered) {
            cx.notify();
        }
    }

    // Kept for the appshots half of this page (appshots.rs), whose host still
    // carries the drag-move listener itself.
    fn on_bar_drag_move(
        &mut self,
        event: &gpui::DragMoveEvent<popover::MenuScrollbarDrag>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.rail_drag_to(event.event.position.y) {
            cx.notify();
        }
    }

    /// The shared rail under the id of whichever page is showing. Kept as a
    /// method because the appshots half of this page renders through it.
    fn render_scrollbar(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let id = if self.appshots_page {
            "appshots-settings-page-scrollbar"
        } else {
            "shortcuts-page-scrollbar"
        };
        popover::rail(self, id, theme, cx)
    }
}

impl popover::ScrollRailHost for ShortcutsPage {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        self.scroll.rail_bar()
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.scroll.rail_scroll()
    }
}

/// The shortcut (other than `id`) already bound to `combo`, if any. Pure.
pub fn conflict_owner(keymap: &KeymapConfig, id: ShortcutId, combo: &str) -> Option<ShortcutId> {
    ShortcutId::ALL
        .into_iter()
        .find(|&other| other.available() && other != id && keymap.get(other) == combo)
}

pub fn send_combo_is_reserved(_behavior: ComposerSendBehavior, combo: &str) -> bool {
    combo == "mod-enter"
}

pub fn modifier_send_label(is_macos: bool) -> &'static str {
    if is_macos { "⌘ Enter" } else { "Ctrl Enter" }
}

/// The per-message override rides the modifier chord, which is the send key
/// itself once "Send messages with" moves to it — say so instead of promising
/// a chord that cannot invert anything.
pub fn follow_up_description(send_behavior: ComposerSendBehavior, modifier_label: &str) -> String {
    match send_behavior {
        ComposerSendBehavior::Enter => format!(
            "Queue follow-ups while the agent runs, or steer the current run. Press {modifier_label} to do the opposite for one message."
        ),
        ComposerSendBehavior::ModEnter => format!(
            "Queue follow-ups while the agent runs, or steer the current run. {modifier_label} is the send key, so every message follows this setting."
        ),
    }
}

/// The page's sections, in display order. [`group`] is a total match, so every
/// [`ShortcutId::ALL`] entry lands in exactly one — a shortcut added later
/// extends the match and appears on the page by construction
/// (`every_shortcut_lands_in_a_rendered_group` holds the other half: its group
/// name must be listed here).
const GROUP_ORDER: [&str; 7] = [
    "Files",
    "Browser",
    "Panels",
    "Sessions",
    "Projects",
    "Jump to session",
    "Appshots",
];

/// The section a shortcut's row renders under.
fn group(id: ShortcutId) -> &'static str {
    match id {
        ShortcutId::CaptureAppshot => "Appshots",
        ShortcutId::SaveFile => "Files",
        ShortcutId::BrowserReload => "Browser",
        ShortcutId::ToggleSidebar | ShortcutId::ToggleChanges | ShortcutId::ToggleTerminal => {
            "Panels"
        }
        ShortcutId::NewProject => "Projects",
        ShortcutId::OpenModelPicker
        | ShortcutId::NewSession
        | ShortcutId::NextSession
        | ShortcutId::PrevSession
        | ShortcutId::ArchiveSession => "Sessions",
        ShortcutId::JumpSession(_) => "Jump to session",
    }
}

/// One-line purpose copy per shortcut (zeron lib/shortcuts.ts
/// `SHORTCUT_DEFINITIONS` descriptions, verbatim).
fn description(id: ShortcutId) -> &'static str {
    match id {
        ShortcutId::CaptureAppshot => {
            "Capture the focused application from anywhere on your desktop."
        }
        ShortcutId::SaveFile => "Save the active workspace file.",
        ShortcutId::BrowserReload => "Reload the focused browser tab.",
        ShortcutId::ToggleSidebar => "Show or hide sessions and settings navigation.",
        ShortcutId::ToggleChanges => "Show or hide the right sidebar for the current session.",
        ShortcutId::ToggleTerminal => "Show or hide the terminal for the current session.",
        ShortcutId::NewSession => "Open a blank session canvas to start a new session.",
        ShortcutId::NewProject => "Open the new project dialog.",
        ShortcutId::OpenModelPicker => "Open the model picker for the current session.",
        ShortcutId::NextSession => {
            "Open the session switcher on the previous session; press again to move on, release to switch."
        }
        ShortcutId::PrevSession => {
            "Open the session switcher on the least recent session; press again to move back, release to switch."
        }
        ShortcutId::ArchiveSession => "Move the current session to the archived shelf.",
        // One line per slot would repeat itself nine times; the ordinal is
        // already in the row's label.
        ShortcutId::JumpSession(_) => "Open the session at this place in the sidebar list.",
    }
}

impl Render for ShortcutsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.appshot_capabilities = crate::appshots::capabilities();
        if self.appshots_page {
            if std::mem::take(&mut self.appshots_focus_pending) {
                window.focus(&self.focus, cx);
            }
            return self.render_appshots(window, cx);
        }
        let theme = Theme::of(cx).clone();
        let recording = self.recording;
        let escape_stops_active_agent = self.escape_stops_active_agent;
        let send_behavior = self.composer_send_behavior;
        let follow_up_behavior = self.follow_up_behavior;
        let switcher_window = self.switcher_window;
        let switcher_limit = self.switcher_limit;
        let customized = self.keymap != KeymapConfig::default()
            || escape_stops_active_agent
            || send_behavior != ComposerSendBehavior::default()
            || follow_up_behavior != FollowUpBehavior::default()
            || switcher_window != SwitcherWindow::default()
            || switcher_limit != 0;
        let modifier_label = modifier_send_label(cfg!(target_os = "macos"));

        let escape_behavior_row = widgets::section_card(&theme).child(
            widgets::card_row(&theme, true)
                .min_h(px(84.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(widgets::row_title(&theme, "Stop active agent with Escape"))
                        .child(
                            div()
                                .mt(px(4.0))
                                .max_w(px(430.0))
                                .text_size(crate::typography::ui_rems(11.5))
                                .line_height(px(17.0))
                                .text_color(theme.text_muted.opacity(0.65))
                                .child(SharedString::from(
                                    "When no dialog, menu, picker, or terminal handles Escape, stop the agent in the active session.",
                                )),
                        ),
                )
                .child(
                    widgets::toggle_switch(&theme, escape_stops_active_agent)
                        .id("escape-stops-active-agent-toggle")
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_escape_stops_active_agent(!escape_stops_active_agent, cx);
                        })),
                ),
        );

        let send_behavior_control = div()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .when(send_behavior != ComposerSendBehavior::Enter, |el| {
                el.child(
                    div()
                        .id("composer-send-reset")
                        .size(px(26.0))
                        .rounded(px(7.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(theme.text_muted)
                        .cursor_pointer()
                        .hover(|s| s.bg(crate::theme::ink(0.04)).text_color(theme.text))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_composer_send_behavior(ComposerSendBehavior::Enter, cx)
                        }))
                        .child(
                            crate::icons::icon(crate::icons::RESTART)
                                .size(px(13.0))
                                .text_color(theme.text_muted),
                        ),
                )
            })
            .child(
                div()
                    .id("composer-send-behavior")
                    .flex()
                    .flex_row()
                    .rounded(px(9.0))
                    .p(px(2.0))
                    .bg(crate::theme::ink(0.04))
                    .children(
                        [
                            (ComposerSendBehavior::Enter, "Enter"),
                            (ComposerSendBehavior::ModEnter, modifier_label),
                        ]
                        .into_iter()
                        .enumerate()
                        .map(|(ix, (behavior, label))| {
                            let selected = send_behavior == behavior;
                            div()
                                .id(("composer-send-option", ix))
                                .min_w(px(72.0))
                                .px(px(12.0))
                                .py(px(6.0))
                                .rounded(px(7.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .font_family(theme.font_mono.clone())
                                .text_size(px(12.0))
                                .text_color(if selected {
                                    theme.text
                                } else {
                                    theme.text_muted
                                })
                                .when(selected, |el| {
                                    el.bg(theme.bg)
                                        .border_1()
                                        .border_color(theme.border.opacity(0.8))
                                })
                                .when(!selected, |el| {
                                    el.cursor_pointer()
                                        .hover(|s| s.text_color(theme.text))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.set_composer_send_behavior(behavior, cx)
                                        }))
                                })
                                .child(SharedString::from(label))
                        }),
                    ),
            );

        let send_behavior_row = widgets::section_card(&theme)
            .child(
                widgets::card_row(&theme, true)
                    .min_h(px(84.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(widgets::row_title(&theme, "Send messages with"))
                            .child(
                                div()
                                    .mt(px(4.0))
                                    .max_w(px(430.0))
                                    .text_size(px(11.5))
                                    .line_height(px(17.0))
                                    .text_color(theme.text_muted.opacity(0.65))
                                    .child(SharedString::from(
                                        "Choose whether Enter sends immediately or starts a new paragraph. Cmd/Ctrl+Enter always submits; with an empty composer it sends the most recently queued message. Shift+Enter always inserts a line break.",
                                    )),
                            ),
                    )
                    .child(send_behavior_control),
            )
            .child(
                widgets::card_row(&theme, false)
                    .min_h(px(84.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(widgets::row_title(&theme, "Follow-up behavior"))
                            .child(
                                div()
                                    .mt(px(4.0))
                                    .max_w(px(430.0))
                                    .text_size(px(11.5))
                                    .line_height(px(17.0))
                                    .text_color(theme.text_muted.opacity(0.65))
                                    .child(SharedString::from(follow_up_description(
                                        send_behavior,
                                        modifier_label,
                                    ))),
                            ),
                    )
                    .child(
                        div()
                            .id("follow-up-behavior")
                            .flex_none()
                            .flex()
                            .flex_row()
                            .rounded(px(9.0))
                            .p(px(2.0))
                            .bg(crate::theme::ink(0.04))
                            .children(
                                [
                                    (FollowUpBehavior::Queue, "Queue"),
                                    (FollowUpBehavior::Steer, "Steer"),
                                ]
                                .into_iter()
                                .enumerate()
                                .map(|(ix, (behavior, label))| {
                                    let selected = follow_up_behavior == behavior;
                                    div()
                                        .id(("follow-up-option", ix))
                                        .min_w(px(72.0))
                                        .px(px(12.0))
                                        .py(px(6.0))
                                        .rounded(px(7.0))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .text_size(px(12.0))
                                        .text_color(if selected {
                                            theme.text
                                        } else {
                                            theme.text_muted
                                        })
                                        .when(selected, |el| {
                                            el.bg(theme.bg)
                                                .border_1()
                                                .border_color(theme.border.opacity(0.8))
                                        })
                                        .when(!selected, |el| {
                                            el.cursor_pointer()
                                                .hover(|s| s.text_color(theme.text))
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.set_follow_up_behavior(behavior, cx)
                                                }))
                                        })
                                        .child(SharedString::from(label))
                                }),
                            ),
                    ),
            );
        let switcher_row = widgets::section_card(&theme).child(
            widgets::card_row(&theme, true)
                .min_h(px(84.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(widgets::row_title(&theme, "Session switcher shows"))
                        .child(
                            div()
                                .mt(px(4.0))
                                .max_w(px(430.0))
                                .text_size(px(11.5))
                                .line_height(px(17.0))
                                .text_color(theme.text_muted.opacity(0.65))
                                .child(SharedString::from(
                                    "Ctrl+Tab offers every session in the sidebar, or only those active within this window. The open session and running tabs are always included.",
                                )),
                        ),
                )
                .child(
                    div()
                        .id("switcher-window")
                        .flex_none()
                        .flex()
                        .flex_row()
                        .rounded(px(9.0))
                        .p(px(2.0))
                        .bg(crate::theme::ink(0.04))
                        .children(SwitcherWindow::ALL.into_iter().enumerate().map(
                            |(ix, window)| {
                                let selected = switcher_window == window;
                                div()
                                    .id(("switcher-window-option", ix))
                                    .min_w(px(44.0))
                                    .px(px(10.0))
                                    .py(px(6.0))
                                    .rounded(px(7.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(12.0))
                                    .text_color(if selected {
                                        theme.text
                                    } else {
                                        theme.text_muted
                                    })
                                    .when(selected, |el| {
                                        el.bg(theme.bg)
                                            .border_1()
                                            .border_color(theme.border.opacity(0.8))
                                    })
                                    .when(!selected, |el| {
                                        el.cursor_pointer()
                                            .hover(|s| s.text_color(theme.text))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.set_switcher_window(window, cx)
                                            }))
                                    })
                                    .child(SharedString::from(window.label()))
                            },
                        )),
                ),
        );
        let limit_step = |theme: &Theme,
                          id: &'static str,
                          glyph: &'static str,
                          next: Option<u32>| {
            let enabled = next.is_some();
            div()
                .id(id)
                .size(px(26.0))
                .rounded(px(7.0))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(14.0))
                .text_color(if enabled {
                    theme.text_muted
                } else {
                    theme.text_muted.opacity(0.35)
                })
                .when_some(next, |el, next| {
                    let text = theme.text;
                    el.cursor_pointer()
                        .hover(move |s| s.bg(theme.bg).text_color(text))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.set_switcher_limit(next, cx)),
                        )
                })
                .child(SharedString::from(glyph))
        };
        let switcher_limit_row = widgets::section_card(&theme).child(
            widgets::card_row(&theme, true)
                .min_h(px(84.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(widgets::row_title(&theme, "Session switcher shows at most"))
                        .child(
                            div()
                                .mt(px(4.0))
                                .max_w(px(430.0))
                                .text_size(px(11.5))
                                .line_height(px(17.0))
                                .text_color(theme.text_muted.opacity(0.65))
                                .child(SharedString::from(
                                    "Cap the number of cards Ctrl+Tab offers, most recently used first. The open session and running tabs are always included.",
                                )),
                        ),
                )
                .child(
                    div()
                        .id("switcher-limit")
                        .flex_none()
                        .flex()
                        .flex_row()
                        .items_center()
                        .rounded(px(9.0))
                        .p(px(2.0))
                        .bg(crate::theme::ink(0.04))
                        .child(limit_step(
                            &theme,
                            "switcher-limit-down",
                            "−",
                            match switcher_limit {
                                0 => None,
                                n if n <= SWITCHER_LIMIT_MIN => Some(0),
                                n => Some(n - 1),
                            },
                        ))
                        .child(
                            div()
                                .min_w(px(44.0))
                                .px(px(6.0))
                                .flex()
                                .justify_center()
                                .text_size(px(12.0))
                                .text_color(theme.text)
                                .child(SharedString::from(if switcher_limit == 0 {
                                    "All".to_string()
                                } else {
                                    switcher_limit.to_string()
                                })),
                        )
                        .child(limit_step(
                            &theme,
                            "switcher-limit-up",
                            "+",
                            match switcher_limit {
                                0 => Some(SWITCHER_LIMIT_MIN),
                                n if n >= SWITCHER_LIMIT_MAX => None,
                                n => Some(n + 1),
                            },
                        )),
                ),
        );
        // One card per group, each under its small section label — the flat
        // 16-row table read as one undifferentiated wall. `ix` (the id's
        // position in ALL) keys the interactive elements, so ids stay unique
        // across cards. The label nests tight to its card (8px); the card's
        // own `section_card` top margin is zeroed here or it stacks on the
        // wrapper gap and reads as a separate, floating block — group
        // separation comes from the 28px between wrappers instead.
        let mut groups: Vec<gpui::AnyElement> = Vec::new();
        for name in GROUP_ORDER {
            if name == "Appshots" {
                continue;
            }
            let mut card = widgets::section_card(&theme).mt(px(0.0));
            let ids = ShortcutId::ALL.into_iter().filter(|&id| group(id) == name);
            for (gx, id) in ids.enumerate() {
                let ix = ShortcutId::ALL.iter().position(|&a| a == id).unwrap_or(0);
                card = card.child(self.render_row(id, ix, gx, recording, &theme, cx));
            }
            groups.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(widgets::field_label(&theme, name))
                    .child(card)
                    .into_any_element(),
            );
        }

        // Helper line stays in the muted tone even for a rejected conflict —
        // the message names the specific clash (zeron settings.shortcuts.tsx).
        let helper: SharedString = if recording.is_some() {
            "Press Escape to cancel.".into()
        } else if let Some(notice) = self.conflict_notice.clone() {
            notice
        } else {
            "Shortcuts must be unique.".into()
        };

        let scrollbar = self.render_scrollbar(&theme, cx);
        div()
            .id("shortcuts-page-host")
            .relative()
            .size_full()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .child(
                div()
                    .id("shortcuts-page")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll.scroll)
                    .track_focus(&self.focus)
                    .child(
                        widgets::page_column()
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_start()
                                    .justify_between()
                                    .gap(px(24.0))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .child(widgets::page_header(
                                                &theme,
                                                "Keyboard shortcuts",
                                                None,
                                            ))
                                            .child(
                                                widgets::page_subtitle(
                                                    &theme,
                                                    "Click a binding, then press the key combination you \
                                                     want to use. Changes apply immediately and stay on \
                                                     this device.",
                                                )
                                                .max_w(px(512.0))
                                                .line_height(px(20.0)),
                                            ),
                                    )
                                    .child({
                                        // `disabled:opacity-35` when nothing is
                                        // customized or while recording.
                                        let disabled = !customized || recording.is_some();
                                        widgets::ghost_action(&theme)
                                            .id("shortcuts-restore-defaults")
                                            .flex_none()
                                            .when(disabled, |el| el.opacity(0.35))
                                            .when(!disabled, |el| {
                                                el.hover(|s| {
                                                    s.bg(crate::theme::ink(0.04))
                                                        .text_color(theme.text)
                                                })
                                                .on_click(
                                                    cx.listener(|this, _, _, cx| {
                                                        this.keymap = KeymapConfig::default();
                                                        this.stop_recording();
                                                        this.conflict_notice = None;
                                                        this.commit(cx);
                                                        this.set_escape_stops_active_agent(
                                                            false, cx,
                                                        );
                                                        this.set_composer_send_behavior(
                                                            ComposerSendBehavior::Enter,
                                                            cx,
                                                        );
                                                        this.set_follow_up_behavior(
                                                            FollowUpBehavior::Queue,
                                                            cx,
                                                        );
                                                        this.set_switcher_window(
                                                            SwitcherWindow::All,
                                                            cx,
                                                        );
                                                        this.set_switcher_limit(0, cx);
                                                    }),
                                                )
                                            })
                                            .child(
                                                crate::icons::icon(crate::icons::RESTART)
                                                    .size(px(14.0))
                                                    .text_color(theme.text_muted),
                                            )
                                            .child(SharedString::from("Restore defaults"))
                                    }),
                            )
                            .child(send_behavior_row.mt(px(32.0)))
                            .child(switcher_row.mt(px(12.0)))
                            .child(switcher_limit_row.mt(px(12.0)))
                            .child(
                                div()
                                    .mt(px(28.0))
                                    .flex()
                                    .flex_col()
                                    .gap(px(28.0))
                                    .children(groups),
                            )
                            .child(
                                div()
                                    .mt(px(12.0))
                                    .px(px(4.0))
                                    .min_h(px(20.0))
                                    .flex()
                                    .justify_center()
                                    .text_size(crate::typography::ui_rems(12.0))
                                    .text_color(theme.text_muted)
                                    .child(helper),
                            )
                            .child(escape_behavior_row),
                    ),
            )
            .children(scrollbar)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn appshots_setup_can_be_enabled_and_configured_by_keyboard(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
        });
        let window = cx.add_window(|_, cx| {
            let state = cx.new(|_| AppState::new());
            let mut page = ShortcutsPage::new(
                state,
                KeymapConfig::default(),
                false,
                ComposerSendBehavior::default(),
                FollowUpBehavior::default(),
                SwitcherWindow::default(),
                0,
                false,
                false,
                AppshotDestination::Automatic,
                cx,
            );
            page.show_appshots(true);
            // The permission was "asked" already: enabling must not raise the
            // native prompt from a test.
            page.permissions
                .guide
                .set_prompted(crate::settings::permission_guide::PermissionPane::ScreenRecording);
            page
        });
        window
            .update(cx, |page, w, cx| w.focus(&page.focus, cx))
            .unwrap();
        cx.update_window(window.into(), |_, w, cx| {
            w.draw(cx).clear();
        })
        .unwrap();
        let press = |cx: &mut gpui::TestAppContext, key: &str| {
            cx.update_window(window.into(), |_, w, cx| {
                w.draw(cx).clear();
                let keystroke = gpui::Keystroke::parse(key).unwrap();
                w.dispatch_event(
                    gpui::PlatformInput::KeyDown(gpui::KeyDownEvent {
                        keystroke: keystroke.clone(),
                        is_held: false,
                        prefer_character_input: false,
                    }),
                    cx,
                );
                w.dispatch_event(
                    gpui::PlatformInput::KeyUp(gpui::KeyUpEvent { keystroke }),
                    cx,
                );
            })
            .unwrap();
        };
        // Sound starts off (non-default), so Restore defaults is the first stop.
        press(cx, "tab");
        press(cx, "tab");
        press(cx, "space");
        window
            .update(cx, |page, _, _| assert!(page.appshots_enabled))
            .unwrap();
        // The reset glyph that enabling reveals sits before the toggle in tree
        // order; ahead are the recorder and Automatic, then Last session.
        for key in ["tab", "tab", "tab", "space"] {
            press(cx, key);
        }
        window
            .update(cx, |page, _, _| {
                assert_eq!(page.appshot_destination, AppshotDestination::LastSession)
            })
            .unwrap();
        for key in ["tab", "enter"] {
            press(cx, key);
        }
        window
            .update(cx, |page, _, _| {
                assert_eq!(page.appshot_destination, AppshotDestination::NewSession)
            })
            .unwrap();
        for key in ["shift-tab", "space"] {
            press(cx, key);
        }
        window
            .update(cx, |page, _, _| {
                assert_eq!(page.appshot_destination, AppshotDestination::LastSession)
            })
            .unwrap();
        // Past New session, the sound row's reset glyph (sound is off) and
        // Preview to the sound toggle.
        for key in ["tab", "tab", "tab", "tab", "enter"] {
            press(cx, key);
        }
        window
            .update(cx, |page, _, _| assert!(page.appshot_sound_enabled))
            .unwrap();
        // Restore defaults is reachable again at the top of the page.
        window
            .update(cx, |page, _, cx| page.restore_appshot_defaults(cx))
            .unwrap();
        window
            .update(cx, |page, _, _| {
                assert!(!page.appshots_enabled);
                assert!(page.appshot_sound_enabled);
                assert_eq!(page.appshot_destination, AppshotDestination::Automatic);
            })
            .unwrap();
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn hotkey_menu_picks_a_chord_and_persists_it(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
        });
        let (page, cx) = cx.add_window_view(|_, cx| {
            let state = cx.new(|_| AppState::new());
            let mut page = ShortcutsPage::new(
                state,
                KeymapConfig::default(),
                false,
                ComposerSendBehavior::default(),
                FollowUpBehavior::default(),
                SwitcherWindow::default(),
                0,
                false,
                true,
                AppshotDestination::Automatic,
                cx,
            );
            page.show_appshots(true);
            page
        });
        cx.run_until_parked();
        let trigger = cx
            .debug_bounds("appshots-appshot-hotkey")
            .expect("hotkey trigger painted");
        cx.simulate_click(trigger.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        // Row order follows HOTKEY_CHOICES: ⌘, ⌥, ⇧, None.
        let option_row = cx
            .debug_bounds("appshots-appshot-hotkey-option-1")
            .expect("menu open with the ⌥ + ⌥ row");
        cx.simulate_click(option_row.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        page.update(cx, |page, _| {
            assert_eq!(
                page.keymap.capture_appshot,
                crate::appshots::ChordModifier::Option.combo()
            );
            // The menu is closing (its exit tween still paints for ~100ms).
            assert!(!page.permissions_hotkey_menu_open());
        });
    }

    #[gpui::test]
    fn recorder_refuses_bound_actions_before_they_can_run(cx: &mut gpui::TestAppContext) {
        use std::{cell::Cell, rc::Rc};
        let fired = Rc::new(Cell::new(false));
        let observed = fired.clone();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            cx.bind_keys([gpui::KeyBinding::new(
                &crate::settings::platform_combo("mod-n"),
                crate::shell::NewSession,
                None,
            )]);
            cx.on_action(move |_: &crate::shell::NewSession, _| observed.set(true));
        });
        let window = cx.add_window(|_, cx| {
            let state = cx.new(|_| AppState::new());
            ShortcutsPage::new(
                state,
                KeymapConfig::default(),
                false,
                ComposerSendBehavior::default(),
                FollowUpBehavior::default(),
                SwitcherWindow::default(),
                0,
                false,
                true,
                AppshotDestination::Automatic,
                cx,
            )
        });
        window
            .update(cx, |page, window, cx| {
                page.start_recording(ShortcutId::CaptureAppshot, window, cx)
            })
            .unwrap();
        cx.simulate_keystrokes(window.into(), &crate::settings::platform_combo("mod-n"));
        window
            .update(cx, |page, _, _| {
                assert!(!fired.get(), "the existing action ran while recording");
                assert!(
                    page.conflict_notice
                        .as_deref()
                        .unwrap()
                        .contains("New session")
                );
                assert_eq!(
                    page.keymap.capture_appshot,
                    ShortcutId::CaptureAppshot.default_combo()
                );
                assert!(page.recording.is_none());
                assert!(page.recording_interceptor.is_none());
            })
            .unwrap();
        cx.simulate_keystrokes(window.into(), &crate::settings::platform_combo("mod-n"));
        assert!(
            fired.get(),
            "finishing recording must restore normal actions"
        );
    }

    #[test]
    fn recording_outcomes() {
        assert_eq!(
            record_key("escape", false, false, false, false),
            RecordOutcome::Cancelled
        );
        assert_eq!(
            record_key("Escape", true, false, false, false),
            RecordOutcome::Cancelled
        );
        assert_eq!(
            record_key("s", false, false, false, true),
            RecordOutcome::Set("mod-s".into())
        );
        assert_eq!(
            record_key("k", false, true, true, true),
            RecordOutcome::Set("mod-alt-shift-k".into())
        );
        // macOS-only: elsewhere ctrl IS the primary and records as "mod".
        #[cfg(target_os = "macos")]
        assert_eq!(
            record_key("tab", true, false, true, false),
            RecordOutcome::Set("ctrl-shift-tab".into())
        );
        // Bare modifiers stay recording.
        assert_eq!(
            record_key("shift", false, false, true, false),
            RecordOutcome::Ignored
        );
        assert_eq!(
            record_key("ctrl", true, false, false, false),
            RecordOutcome::Ignored
        );
    }

    #[test]
    fn every_shortcut_lands_in_a_rendered_group() {
        // The page renders GROUP_ORDER's cards and nothing else — a group()
        // arm returning a name missing from GROUP_ORDER would silently drop
        // its rows from Settings.
        for id in ShortcutId::ALL {
            assert!(
                GROUP_ORDER.contains(&group(id)),
                "{:?} is grouped under {:?}, which GROUP_ORDER does not render",
                id,
                group(id)
            );
        }
        // And every named group has at least one row — no empty cards.
        for name in GROUP_ORDER {
            assert!(
                ShortcutId::ALL.into_iter().any(|id| group(id) == name),
                "group {:?} would render an empty card",
                name
            );
        }
    }

    #[test]
    fn conflicting_records_are_refused() {
        // zeron parity: a combo bound elsewhere is refused at record time (the
        // helper names the owner) — conflicts never persist into the keymap.
        let keymap = KeymapConfig::default();
        let RecordOutcome::Set(combo) = record_key("r", false, false, false, true) else {
            panic!("expected Set");
        };
        assert_eq!(
            conflict_owner(&keymap, ShortcutId::ToggleSidebar, &combo),
            Some(ShortcutId::ToggleChanges)
        );
        // Re-recording a shortcut's own combo is not a conflict.
        assert_eq!(
            conflict_owner(&keymap, ShortcutId::ToggleChanges, &combo),
            None
        );
        // A free combo conflicts with nothing.
        assert_eq!(
            conflict_owner(&keymap, ShortcutId::ToggleSidebar, "mod-shift-x"),
            None
        );
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn appshot_binding_participates_in_existing_conflict_checks() {
        let mut keymap = KeymapConfig::default();
        assert_eq!(
            conflict_owner(&keymap, ShortcutId::CaptureAppshot, "mod-n"),
            Some(ShortcutId::NewSession)
        );
        keymap.set(ShortcutId::CaptureAppshot, "mod-alt-k".into());
        assert_eq!(
            conflict_owner(&keymap, ShortcutId::NewSession, "mod-alt-k"),
            Some(ShortcutId::CaptureAppshot)
        );
        assert_eq!(
            conflict_owner(&keymap, ShortcutId::CaptureAppshot, "mod-alt-k"),
            None
        );
    }

    #[test]
    fn modifier_send_labels_are_platform_specific() {
        assert_eq!(modifier_send_label(true), "⌘ Enter");
        assert_eq!(modifier_send_label(false), "Ctrl Enter");
    }

    #[test]
    fn modifier_send_is_always_reserved_for_the_composer() {
        assert!(send_combo_is_reserved(
            ComposerSendBehavior::Enter,
            "mod-enter"
        ));
        assert!(send_combo_is_reserved(
            ComposerSendBehavior::ModEnter,
            "mod-enter"
        ));
        assert!(!send_combo_is_reserved(
            ComposerSendBehavior::ModEnter,
            "mod-shift-enter"
        ));
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn appshots_lists_its_grants_through_the_shared_guide(cx: &mut gpui::TestAppContext) {
        use crate::settings::permission_guide::PermissionPane;
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
        });
        let (page, cx) = cx.add_window_view(|_, cx| {
            let state = cx.new(|_| AppState::new());
            let mut page = ShortcutsPage::new(
                state,
                KeymapConfig::default(),
                false,
                ComposerSendBehavior::default(),
                FollowUpBehavior::default(),
                SwitcherWindow::default(),
                0,
                false,
                true,
                AppshotDestination::Automatic,
                cx,
            );
            page.show_appshots(true);
            page
        });
        cx.run_until_parked();
        page.update(cx, |page, _| {
            assert_eq!(
                page.permissions.guide.rendered,
                vec![
                    PermissionPane::InputMonitoring,
                    PermissionPane::ScreenRecording,
                    PermissionPane::Accessibility,
                ]
            );
        });
        assert!(
            cx.debug_bounds("appshot-permission-row-input-monitoring")
                .is_some()
        );
    }
}
