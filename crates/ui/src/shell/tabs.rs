//! Session navigation: the titlebar carries a horizontal tab strip of the
//! sessions open on THIS device (`UiSettings.open_tabs`, the vec IS the
//! order), and the activity sidebar is the full session list. A tab is a
//! device-local viewport onto a synced session: closing one removes it here
//! and nowhere else — the session keeps running on its host and stays in the
//! sidebar, where a click reopens it. Any selection (sidebar, palette,
//! notification banner) becomes a tab through one invariant in
//! [`Shell::sync_open_tabs`]. New sessions are minted by the titlebar's
//! control-cluster `+` and the sidebar header `+`.
//!
//! Each tab: agent brand icon + title + a trailing slot that shows the status
//! dot at rest and swaps to a close button on hover; right-click opens the
//! close/close-others/close-right/close-left menu. Styling and drag-reorder
//! mirror the right pane's surface strip (`render_right_tab_strip`) — same
//! fixed-width tabs, drop-index math, 150ms sibling slide, and drag ghost.
//! Overflow scrolls horizontally with edge fades.

use super::*;
use crate::terminal::panel::{drop_index, reorder_tabs, slide_offset};
use zeron_proto::ChatIndicator;

/// Fixed tab width (the right pane's surface chips use 112; session titles
/// get a bit more).
pub(super) const SESSION_TAB_WIDTH: f32 = 140.0;
/// Flex gap between tabs — part of the drop-index slot width.
const TAB_GAP: f32 = 4.0;
/// Width of the overflow edge fades. Wide enough that per-glyph fade steps
/// (title text fades glyph-by-glyph on glass) stay gentle.
const FADE_WIDTH: f32 = 36.0;

/// Drag-reorder state; `epoch` keys the 150ms slide animation restarts.
pub(super) struct TabDragState {
    from: usize,
    over: usize,
    epoch: usize,
    prev_over: usize,
}

/// The dragged-tab payload (gpui drag-and-drop). `chat_id` is what the split
/// view's drop zones read; the strip itself reorders by `from`.
pub(crate) struct TabDragPayload {
    from: usize,
    pub(crate) chat_id: String,
    title: SharedString,
    brand: Option<(&'static str, Option<gpui::Hsla>)>,
}

/// The floating tab rendered at the cursor while dragging (tab or sidebar row).
pub(super) struct TabGhost {
    pub(super) title: SharedString,
    pub(super) brand: Option<(&'static str, Option<gpui::Hsla>)>,
}

impl Render for TabGhost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        div()
            .w(px(SESSION_TAB_WIDTH))
            .h(px(28.0))
            .px(px(Theme::SPACE_SM))
            .flex()
            .items_center()
            .gap(px(6.0))
            .rounded(px(Theme::CONTROL_RADIUS))
            .bg(theme.surface_raised)
            .border_1()
            .border_color(theme.border_strong)
            .text_size(px(12.0))
            .text_color(theme.text)
            .opacity(0.85)
            .when_some(self.brand, |el, (path, tint)| {
                el.child(
                    icon(path)
                        .size(px(14.0))
                        .flex_none()
                        .text_color(tint.unwrap_or(theme.text_muted)),
                )
            })
            .child(div().truncate().child(self.title.clone()))
    }
}

/// The neighbor to select after closing `closed`: the next tab, else the
/// previous, else `None` (last tab → new-session canvas). Pure.
pub(super) fn next_after_close(order: &[String], closed: &str) -> Option<String> {
    let ix = order.iter().position(|id| id == closed)?;
    if order.len() <= 1 {
        return None;
    }
    Some(if ix + 1 < order.len() {
        order[ix + 1].clone()
    } else {
        order[ix - 1].clone()
    })
}

/// Sliding transform while a sibling is dragged over: animate 150ms between
/// committed offsets. `slots` is how many tab slots the element spans.
fn slide_tab<E: gpui::Element + gpui::Styled + 'static>(
    tab_el: E,
    ix: usize,
    id: String,
    slots: usize,
    drag: Option<(usize, usize, usize, usize)>,
) -> AnyElement {
    match drag {
        Some((from, over, epoch, prev_over)) if ix != from => {
            let slot = SESSION_TAB_WIDTH + TAB_GAP;
            let target = slide_offset(ix, from, over) * slot;
            let start = slide_offset(ix, from, prev_over) * slot;
            div()
                .relative()
                .child(tab_el.with_animation(
                    SharedString::from(format!("session-tab-slide-{id}-{epoch}")),
                    TAB_SLIDE.animation(),
                    move |el, t| el.left(px(motion::lerp(start, target, t))),
                ))
                .into_any_element()
        }
        // The dragged tab is represented by the cursor ghost; its flow slot
        // renders as an INVISIBLE spacer. A dimmed tab here overlapped
        // whatever sibling slid into the vacated slot (slide_offset moves one
        // tab exactly there — user-reported double-exposure).
        Some((from, ..)) if ix == from => div()
            .w(px(
                SESSION_TAB_WIDTH * slots as f32 + TAB_GAP * (slots - 1) as f32
            ))
            .h(px(28.0))
            .flex_none()
            .into_any_element(),
        _ => tab_el.into_any_element(),
    }
}

/// A tab's trailing slot: the status at rest, the close button under the
/// pointer. Fixed at 20px so the title never reflows on hover.
///
/// NB: no `.occlude()` on the close button — the TAB already occludes (for
/// the titlebar drag region), and an occluding child would block the tab's
/// own hover hit-test: a flicker loop (user-reported). `stop_propagation`
/// on click is enough.
fn tab_trailing(
    id: &str,
    status: ChatIndicator,
    hovered: bool,
    theme: &Theme,
    cx: &mut Context<Shell>,
) -> AnyElement {
    if hovered {
        let close_id = id.to_string();
        return div()
            .id(SharedString::from(format!("session-tab-close-{id}")))
            .size(px(20.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .hover(|s| s.bg(crate::theme::wash(0.14)))
            // The tab owns a drag payload. Claim the close press
            // before it reaches that parent or GPUI starts a tab
            // drag instead of delivering the close click.
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.close_session_tab(close_id.clone(), cx);
            }))
            .child(
                icon(icons::CLOSE)
                    .size(px(12.0))
                    .text_color(theme.text_muted),
            )
            .into_any_element();
    }
    // Working animates (the sidebar's miniaturized glyph spinner) instead
    // of a static dot; every other non-idle status stays a dot.
    let dot = spaces::status_dot_color(status, theme);
    div()
        .size(px(20.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .when(status == ChatIndicator::Working, |el| {
            el.child(loaders::mini_glyph_spinner(
                format!("tab-working-{id}"),
                2.0,
                theme.glyph,
                cx.entity_id(),
                cx,
            ))
        })
        .when(
            !matches!(status, ChatIndicator::Idle | ChatIndicator::Working),
            |el| el.child(div().size(px(6.0)).rounded_full().bg(dot)),
        )
        .into_any_element()
}

/// The split's double chip: both sessions in one selected pill, left half
/// for the left pane. A half clicks, hovers a close, drags (into the drop
/// zones or along the strip) and middle-closes like a tab; the pane header
/// carries the split's own close.
fn split_tab_chip(
    ix: usize,
    halves: [(
        String,
        SharedString,
        Vec<zeron_proto::HarnessId>,
        ChatIndicator,
    ); 2],
    selected: Option<&str>,
    hovered: Option<&str>,
    theme: &Theme,
    cx: &mut Context<Shell>,
) -> gpui::Stateful<gpui::Div> {
    // The chip is "selected" while either member is on screen; another
    // session selected shows full width and the pair waits here.
    let active = halves.iter().any(|(id, ..)| selected == Some(id.as_str()));
    let mut chip = div()
        .id("session-tab-split")
        .w(px(SESSION_TAB_WIDTH * 2.0 + TAB_GAP))
        .h(px(28.0))
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .rounded(px(8.0))
        .text_size(px(12.0))
        .when(active, |el| {
            el.bg(crate::theme::glass_selected_bg())
                .shadow(crate::theme::glass_selected_shadows())
        })
        .when(!active, |el| el.hover(|s| s.bg(theme.glass_hover())))
        .block_mouse_except_scroll()
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default());
    for (n, (id, title, harness, status)) in halves.into_iter().enumerate() {
        if n == 1 {
            chip = chip.child(
                div()
                    .w(px(1.0))
                    .h(px(16.0))
                    .flex_none()
                    .bg(theme.border_strong),
            );
        }
        let is_primary = selected == Some(id.as_str());
        let brand = harness.last().copied().map(crate::pickers::harness_brand_icon);
        let trailing = tab_trailing(&id, status, hovered == Some(id.as_str()), theme, cx);
        let select_id = id.clone();
        let middle_id = id.clone();
        let menu_id = id.clone();
        let hover_id = id.clone();
        chip = chip.child(
            div()
                .id(SharedString::from(format!("session-tab-{id}")))
                .debug_selector(|| format!("session-tab-{id}"))
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .pl(px(8.0))
                .pr(px(4.0))
                .rounded(px(8.0))
                .text_color(if is_primary {
                    theme.text
                } else if active {
                    theme.text.opacity(0.7)
                } else {
                    theme.text_muted.opacity(0.6)
                })
                .cursor_pointer()
                // Per-half hover in Shell state, like a tab's: the trailing
                // slot swaps status for close on the half under the pointer.
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if *hovered {
                        this.tab_hover = Some(hover_id.clone());
                    } else if this.tab_hover.as_deref() == Some(hover_id.as_str()) {
                        this.tab_hover = None;
                    }
                    cx.notify();
                }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.navigate_to(Some(select_id.clone()), true, cx);
                }))
                .on_mouse_down(
                    MouseButton::Middle,
                    cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.close_session_tab(middle_id.clone(), cx);
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.tab_menu.open((menu_id.clone(), event.position));
                        cx.notify();
                    }),
                )
                .on_drag(
                    TabDragPayload {
                        from: ix,
                        chat_id: id.clone(),
                        title: title.clone(),
                        brand,
                    },
                    |payload, _point, _, cx| {
                        let title = payload.title.clone();
                        let brand = payload.brand;
                        cx.stop_propagation();
                        cx.new(|_| TabGhost { title, brand })
                    },
                )
                .map(|el| {
                    match crate::computer::strip::chat_glyph(
                        "session-tab",
                        &id,
                        14.0,
                        0.9,
                        theme,
                        cx,
                    ) {
                        Some(glyph) => el.child(glyph),
                        None => el.when(!harness.is_empty(), |el| {
                            let selector = format!("session-tab-harness-{id}");
                            el.child(
                                crate::pickers::harness_mark_stack(
                                    &harness,
                                    14.0,
                                    theme.text_muted,
                                    0.9,
                                )
                                .debug_selector(move || selector),
                            )
                        }),
                    }
                })
                .child(div().flex_1().min_w_0().truncate().child(title))
                .child(trailing),
        );
    }
    chip
}

pub(super) fn right_pane_expand_icon(expanded: bool) -> &'static str {
    if expanded {
        icons::COLLAPSE_ARROWS
    } else {
        icons::EXPAND_ARROWS
    }
}

struct PanelTitlebarWidths {
    surface_reveal: f32,
    files_controls: f32,
}

/// The session header's "+" and fork buttons (28px each, 2px gap) plus the
/// row gap they cost the project-actions control.
const SESSION_CONTROLS_WIDTH: f32 = 28.0 * 2.0 + 2.0 + 8.0;

/// The two fixed right-edge anchors: the explorer toggle and the pane toggle
/// (28px each) with the same 4px gap the surface strip keeps between its
/// controls, so the two never render as one fused block.
const PANEL_TOGGLE_GAP: f32 = 4.0;
const PANEL_TOGGLE_SLOTS: f32 = 28.0 * 2.0 + PANEL_TOGGLE_GAP;

fn panel_titlebar_widths(
    surfaces_visible: f32,
    files_visible: f32,
    available: f32,
    right_pad: f32,
) -> PanelTitlebarWidths {
    // Caption controls occupy the far-right panel first. Subtract their
    // clearance once across the combined header, then split it at Files.
    // The explorer and pane toggles keep their slots even when closed.
    let files_controls = (files_visible - right_pad).max(PANEL_TOGGLE_SLOTS);
    let surfaces = surfaces_visible + files_visible - right_pad - files_controls;
    PanelTitlebarWidths {
        surface_reveal: surfaces.min(available - files_controls).max(0.0),
        files_controls,
    }
}

impl Shell {
    /// The same binding navigates the focused pane. Keep the persisted action
    /// IDs so existing user keymaps and the native browser bridge still work.
    pub(super) fn cycle_navigation(
        &mut self,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.switcher.is_some() {
            self.switcher_step(forward, cx);
            return;
        }
        if self.navigation_overlay_open(cx) {
            return;
        }
        if matches!(self.route, Route::Chat)
            && self.right_pane_open(cx)
            && self.navigation_focus.in_right(window, cx)
        {
            let rows = self.right_surface_rows(cx);
            if rows.len() <= 1 {
                return;
            }
            let active = self.resolved_right_active(cx);
            let at = rows.iter().position(|(surface, ..)| *surface == active);
            let next = match (at, forward) {
                (Some(at), true) => (at + 1) % rows.len(),
                (Some(at), false) => (at + rows.len() - 1) % rows.len(),
                (None, true) => 0,
                (None, false) => rows.len() - 1,
            };
            self.activate_right_surface(rows[next].0, window, cx);
        } else {
            self.switcher_step(forward, cx);
        }
    }

    fn navigation_overlay_open(&self, cx: &App) -> bool {
        self.overlay_owns_keyboard(cx)
            || self.sync_flow.has_visible_overlay()
            || self.delete_confirm.is_some()
            || self.delete_space_confirm.is_some()
            || self.chat_rename.is_some()
            || self.rename_space_dialog.is_some()
            || self.discard_working_tree.is_some()
            || self.chat_menu.get().is_some()
            || self.space_menu.get().is_some()
            || self.user_menu.get().is_some()
            || self.spaces_menu.get().is_some()
            || self.right_plus.get().is_some()
            || !self.pending_file_closes.is_empty()
            || self.pending_exit.is_some()
    }

    pub(super) fn activate_right_surface(
        &mut self,
        surface: RightSurface,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigation_focus.right_was_focused = true;
        self.composer
            .update(cx, |composer, _| composer.focus_pending = false);
        // Establish a stable target before detaching the old editor. Read-only
        // surfaces keep it; editable surfaces claim their own focus below/on mount.
        window.focus(&self.navigation_focus.right, cx);
        self.set_right_active(surface, cx);
        self.focus_right_file_editor(surface, window, cx);
    }

    pub(super) fn restore_right_focus_after_close(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.right_pane_open(cx) {
            self.activate_right_surface(self.resolved_right_active(cx), window, cx);
        } else {
            self.navigation_focus.right_was_focused = false;
            window.focus(&self.composer.focus_handle(cx), cx);
        }
    }

    /// Navigation requests focus once the destination composer renders.
    pub(super) fn focus_composer(&mut self, cx: &mut Context<Self>) {
        self.composer.update(cx, |composer, cx| {
            composer.focus_pending = true;
            cx.notify();
        });
    }

    /// Boot landing: the most recently active visible chat once the first
    /// chats frame has synced (manual selection wins; no chats → the
    /// new-session canvas shows).
    pub(super) fn boot_select_chat(&mut self, cx: &mut Context<Self>) {
        let first = {
            let state = self.state.read(cx);
            if !state.chats_synced || state.selected_chat.is_some() || state.auto_selected {
                return;
            }
            // Capture knob: `ZERON_OPEN_CHAT=<id>` lands on that chat instead of the most recent.
            std::env::var("ZERON_OPEN_CHAT")
                .ok()
                .filter(|id| state.chats.iter().any(|c| c.id == *id))
                .or_else(|| {
                    state
                        .overview_chats(Utc::now())
                        .first()
                        .map(|(_, c)| c.id.clone())
                })
        };
        if let Some(first) = first {
            self.focus_composer(cx);
            self.state
                .update(cx, |s, cx| s.select_chat(Some(first), cx));
        }
    }

    /// Open a session from the sidebar: select it, the main area follows.
    /// The session being left is snapshotted first for the switcher.
    #[track_caller]
    pub(crate) fn open_chat(&mut self, chat_id: String, cx: &mut Context<Self>) {
        if self.state.read(cx).selected_chat.as_deref() != Some(chat_id.as_str()) {
            self.snapshot_current_chat(cx);
        }
        self.select_existing_chat(chat_id, cx);
    }

    #[track_caller]
    pub(super) fn select_existing_chat(&mut self, chat_id: String, cx: &mut Context<Self>) {
        tracing::info!(
            target: "chat_selection",
            to = %chat_id,
            caller = %std::panic::Location::caller(),
            "shell opens chat"
        );
        // A draft id must never become `selected_chat`: it has no doc to
        // watch and would read as a promoted session.
        if self.state.read(cx).is_draft(&chat_id) {
            return self.open_draft(chat_id, false, cx);
        }
        self.command_palette = None;
        self.route = Route::Chat;
        self.focus_composer(cx);
        self.state
            .update(cx, |s, cx| s.select_chat(Some(chat_id), cx));
        cx.notify();
    }

    /// One door for every tab-shaped identity: a chat opens as a session, a
    /// draft opens on the canvas, `None` lands on the canvas (a fresh draft
    /// when the active one is gone). `snapshot`: capture the frame being
    /// left for the switcher (false under an overlay, which is in the frame).
    #[track_caller]
    pub(super) fn navigate_to(
        &mut self,
        id: Option<String>,
        snapshot: bool,
        cx: &mut Context<Self>,
    ) {
        match id {
            Some(id) if self.state.read(cx).is_draft(&id) => self.open_draft(id, snapshot, cx),
            Some(id) if snapshot => self.open_chat(id, cx),
            Some(id) => self.select_existing_chat(id, cx),
            None => {
                if snapshot {
                    self.snapshot_current_chat(cx);
                }
                self.state.update(cx, |s, cx| {
                    s.select_chat(None, cx);
                    s.ensure_draft(cx);
                });
                self.freeze_draft_row(cx);
                cx.notify();
            }
        }
    }

    /// Open a draft from its sidebar row, tab or switcher card.
    pub(crate) fn open_draft(&mut self, draft_id: String, snapshot: bool, cx: &mut Context<Self>) {
        if self.state.read(cx).focused_id().as_deref() == Some(draft_id.as_str()) {
            return;
        }
        if snapshot {
            self.snapshot_current_chat(cx);
        }
        self.command_palette = None;
        self.route = Route::Chat;
        self.focus_composer(cx);
        self.state
            .update(cx, |s, cx| s.open_draft(draft_id.clone(), cx));
        self.touch_session_mru(&draft_id);
        self.freeze_draft_row(cx);
        cx.notify();
    }

    /// Capture the active draft's row text as it becomes the route (d21).
    /// No content yet → no row until the next visit.
    pub(super) fn freeze_draft_row(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.state.read(cx).active_draft.clone() else {
            return;
        };
        match crate::draft_store::entry_prompt(&id, cx) {
            Some(prompt) => {
                self.frozen_draft_rows.insert(id, prompt);
            }
            None => {
                self.frozen_draft_rows.remove(&id);
            }
        }
    }

    /// The row's ✕ or closing an empty draft's tab: the session and its
    /// stored text go; an open draft is replaced by a fresh canvas.
    pub(super) fn discard_draft(&mut self, draft_id: String, cx: &mut Context<Self>) {
        let was_focused = self.state.read(cx).focused_id().as_deref() == Some(draft_id.as_str());
        if let Some(tabs) = self.settings.open_tabs.as_mut() {
            tabs.retain(|id| id != &draft_id);
        }
        self.frozen_draft_rows.remove(&draft_id);
        self.session_mru.retain(|id| id != &draft_id);
        self.state
            .update(cx, |s, cx| s.discard_draft(&draft_id, cx));
        self.composer
            .update(cx, |composer, cx| composer.purge_chat(&draft_id, cx));
        if was_focused {
            self.state.update(cx, |s, cx| {
                s.ensure_draft(cx);
            });
            self.focus_composer(cx);
        }
        self.schedule_save(cx);
        cx.notify();
    }

    /// `+` in the titlebar: open the new-session canvas. A set sidebar filter
    /// re-homes the canvas onto that project; under "All" the canvas reopens
    /// on the project/device it was left at (see `AppState::canvas_target`).
    ///
    /// A new chat always starts with the terminal hidden: when the drawer is
    /// open it just hides (detach, not close — the source chat's tabs and
    /// PTYs survive for the return trip).
    ///
    /// `project` (the per-project `+` on a sidebar group header) homes the
    /// canvas on that project and wins over the sidebar filter.
    pub(super) fn open_new_session(&mut self, project: Option<String>, cx: &mut Context<Self>) {
        self.snapshot_current_chat(cx);
        self.command_palette = None;
        self.route = Route::Chat;
        self.focus_composer(cx);
        // Pre-hide before the selection flips so the state change can't
        // auto-create a canvas tab (the panel's observer runs on the same
        // update). The source chat's flag stays set — returning restores it.
        let was_open = self.terminal_open(cx)
            || self
                .terminal
                .as_ref()
                .is_some_and(|panel| panel.read(cx).is_open());
        if was_open {
            self.terminal_tween = None;
            if let Some(panel) = self.terminal.clone() {
                panel.update(cx, |panel, cx| panel.set_open(false, cx));
            }
        }
        let target = {
            let state = self.state.read(cx);
            project
                .or_else(|| self.settings.space_filter.clone())
                .filter(|id| state.space_row(id).is_some())
        };
        let defaults = crate::settings::composer::ComposerDefaults::load(&self.data_dir);
        self.state.update(cx, |s, cx| {
            // The draft comes first so the target picks below record onto
            // IT, never onto an invested draft being left behind (d20).
            // Leaving a chat puts the canvas's own target back; the filter
            // (an explicit standing choice) then applies on top of it.
            let restores = s.canvas_target.is_some();
            s.select_chat(None, cx);
            s.new_draft(cx);
            if target.is_some() {
                s.select_space(target, cx);
            } else if !restores && defaults.no_project {
                // No canvas target was set aside (e.g. after a runtime swap):
                // fall back to the saved new-session opt-out.
                s.select_space(None, cx);
                if let Some(device) = defaults.device {
                    s.select_device(device, cx);
                }
            }
        });
        self.freeze_draft_row(cx);
        // The canvas never restores a drawer: a previously opened canvas
        // terminal must not pop open on a fresh new chat.
        let key = self.panel_key(cx);
        if self.panels.get(&key).terminal_open {
            self.panels.update(&key, |panels| {
                panels.terminal_open = false;
            });
        }
        self.terminal_tween = None;
        if let Some(panel) = self.terminal.clone()
            && panel.read(cx).is_open()
        {
            panel.update(cx, |panel, cx| panel.set_open(false, cx));
        }
        cx.notify();
    }

    /// The open tabs in VISUAL order: `open_tabs` minus ids the doc no longer
    /// backs with a live (non-archived) chat. A tab stays until the user
    /// closes it; idle ones dim instead of hiding. The stored vec is pruned
    /// separately by [`Shell::sync_open_tabs`]; this read-only view just
    /// skips rows that haven't been reconciled yet.
    pub(super) fn open_tab_ids(&self, cx: &App) -> Vec<String> {
        let Some(open) = self.settings.open_tabs.as_ref() else {
            return Vec::new();
        };
        let state = self.state.read(cx);
        let now = Utc::now();
        let chats: std::collections::HashMap<_, _> = state
            .chats
            .iter()
            .map(|chat| (chat.id.as_str(), chat))
            .collect();
        let drafts: std::collections::HashSet<_> = state
            .draft_sessions
            .iter()
            .map(|draft| draft.id.as_str())
            .collect();
        open.iter()
            .filter(|id| {
                // A draft is always a place the user is working in (d18); a
                // session started in the background shows from its send on.
                drafts.contains(id.as_str())
                    || state.send_pending(id, now)
                    || chats.get(id.as_str()).is_some_and(|c| !c.archived)
            })
            .cloned()
            .collect()
    }

    /// Chats whose tab should open on its own: a person-opened chat that
    /// starts a new run, or any chat that starts waiting on input. Edges
    /// only, so a closed tab stays closed until the next run or question.
    fn auto_open_tab_ids(&mut self, cx: &App) -> Vec<String> {
        let state = self.state.read(cx);
        if !state.sessions_synced() || !state.spaces_synced {
            return Vec::new();
        }
        let now = Utc::now();
        let mut opened = Vec::new();
        let mut next = std::collections::HashMap::new();
        for (status, chat) in state.overview_chats(now) {
            let prev = match self.tab_status_prev.get(&chat.id) {
                Some(prev) => Some(*prev),
                None if chat.created_at > self.tab_watch_since => Some(ChatIndicator::Idle),
                None => None,
            };
            // A run whose lease went stale (host asleep or offline) reads
            // Idle; keeping the old baseline stops its return from counting
            // as a new run.
            let stale_run = status == ChatIndicator::Idle
                && state.session_for(&chat.id).is_some_and(|s| {
                    matches!(
                        s.status,
                        zeron_proto::SessionStatus::Working
                            | zeron_proto::SessionStatus::AwaitingInput
                    )
                });
            next.insert(
                chat.id.clone(),
                match (stale_run, prev) {
                    (true, Some(prev)) => prev,
                    _ => status,
                },
            );
            let Some(prev) = prev else { continue };
            let person_chat = chat.sidechat_source_chat_id.is_none() && chat.is_person_opened();
            let new_run = status == ChatIndicator::Working
                && !matches!(prev, ChatIndicator::Working | ChatIndicator::AwaitingInput);
            let asks = status == ChatIndicator::AwaitingInput
                && prev != ChatIndicator::AwaitingInput
                && chat.delegated_by().is_none();
            if (new_run && person_chat) || asks {
                opened.push(chat.id.clone());
            }
        }
        self.tab_status_prev = next;
        opened
    }

    /// Tabs the "Close idle tabs" menu row would close: idle sessions that
    /// are neither focused, a split member, nor a draft.
    pub(super) fn idle_tab_ids(&self, cx: &App) -> Vec<String> {
        let state = self.state.read(cx);
        let now = Utc::now();
        let focused = state.focused_id();
        self.open_tab_ids(cx)
            .into_iter()
            .filter(|id| {
                focused.as_deref() != Some(id.as_str())
                    && !self.split_pair_contains(id)
                    && !state.is_draft(id)
                    && state
                        .chats
                        .iter()
                        .find(|c| &c.id == id)
                        .is_some_and(|c| state.display_status_for(c, now) == ChatIndicator::Idle)
            })
            .collect()
    }

    pub(super) fn close_idle_tabs(&mut self, cx: &mut Context<Self>) {
        let idle = self.idle_tab_ids(cx);
        if let Some(tabs) = self.settings.open_tabs.as_mut() {
            tabs.retain(|id| !idle.contains(id));
        }
        self.schedule_save(cx);
        cx.notify();
    }

    /// Reconcile the device-local tab list against the synced doc. Runs on
    /// every state change (cheap set math), but only judges once the first
    /// chats frame has landed — the pre-sync empty list would prune everything.
    ///
    /// - No `open_tabs` yet (pre-tabs settings file): seed from the sidebar's
    ///   visible sessions, so the strip doesn't start out blank.
    /// - Prune archived/deleted chats (the selected chat is exempt: a
    ///   just-sent new session's row hasn't synced yet).
    /// - Any selected chat becomes a tab (sidebar clicks, boot landing,
    ///   palette, fresh sends — one invariant covers every selection path).
    /// - [`Shell::auto_open_tab_ids`] appends chats that just started a run
    ///   or a question.
    pub(super) fn sync_open_tabs(&mut self, cx: &mut Context<Self>) {
        if !self.state.read(cx).chats_synced {
            return;
        }
        let (selected, live): (Option<String>, std::collections::HashSet<String>) = {
            let state = self.state.read(cx);
            (
                state.focused_id(),
                state
                    .visible_chats()
                    .map(|c| c.id.clone())
                    .chain(state.draft_sessions.iter().map(|d| d.id.clone()))
                    .collect(),
            )
        };
        let mut changed = false;
        if self.settings.open_tabs.is_none() {
            // First run with tabs (or a pre-tabs settings file): seed from
            // the sidebar; the `open_tabs_stable` pass below then keeps only
            // the active ones, so the strip doesn't start out as the single
            // boot-selected session nor as the whole sidebar. The
            // sidebar list is space-gated, so wait for the spaces frame too —
            // chats alone seeded a single tab (the boot selection).
            if !self.state.read(cx).spaces_synced {
                return;
            }
            self.settings.open_tabs = Some(self.sidebar_visible_order(cx));
            changed = true;
        }
        if !self.settings.open_tabs_stable {
            let state = self.state.read(cx);
            if state.sessions_synced() && state.spaces_synced {
                let now = Utc::now();
                let pair = self.split_pair.as_ref();
                let keep: Vec<String> = self
                    .settings
                    .open_tabs
                    .iter()
                    .flatten()
                    .filter(|id| {
                        selected.as_deref() == Some(id.as_str())
                            || pair.is_some_and(|p| p.contains(id))
                            || state.is_draft(id)
                            || state.send_pending(id, now)
                            || state.chats.iter().find(|c| &c.id == *id).is_some_and(|c| {
                                state.display_status_for(c, now) != ChatIndicator::Idle
                            })
                    })
                    .cloned()
                    .collect();
                self.settings.open_tabs = Some(keep);
                self.settings.open_tabs_stable = true;
                changed = true;
            }
        }
        // A background-started session is neither a draft nor a synced chat
        // until its doc frame lands; its pending send keeps the tab.
        let pending: Vec<String> = {
            let state = self.state.read(cx);
            let now = Utc::now();
            self.settings
                .open_tabs
                .iter()
                .flatten()
                .filter(|id| state.send_pending(id, now))
                .cloned()
                .collect()
        };
        let pair: Vec<String> = self
            .split_pair
            .iter()
            .flat_map(|p| [p.left.clone(), p.right.clone()])
            .collect();
        let opened = self.auto_open_tab_ids(cx);
        let tabs = self.settings.open_tabs.get_or_insert_with(Vec::new);
        let before = tabs.len();
        tabs.retain(|id| {
            live.contains(id)
                || selected.as_deref() == Some(id.as_str())
                || pair.contains(id)
                || pending.contains(id)
        });
        changed |= tabs.len() != before;
        // The split pair is always on the strip (its double chip).
        for pinned in selected.iter().chain(pair.iter()).chain(opened.iter()) {
            if !tabs.contains(pinned) {
                tabs.push(pinned.clone());
                changed = true;
            }
        }
        if changed {
            self.schedule_save(cx);
        }
    }

    /// Close a tab — a purely local act: the session is untouched (it keeps
    /// running on its host and stays in the sidebar). Selection moves to a
    /// neighbor; the last tab lands on the new-session canvas.
    pub(super) fn close_session_tab(&mut self, chat_id: String, cx: &mut Context<Self>) {
        let order = self.open_tab_ids(cx);
        let (focused, is_draft) = {
            let state = self.state.read(cx);
            (state.focused_id(), state.is_draft(&chat_id))
        };
        // A draft's tab is the only place an EMPTY draft exists; an invested
        // one keeps its sidebar row like a session keeps its sidebar entry.
        if is_draft && !crate::draft_store::has_content(&chat_id, cx) {
            let was_focused = focused.as_deref() == Some(chat_id.as_str());
            let next = next_after_close(&order, &chat_id);
            self.discard_draft(chat_id, cx);
            if was_focused && let Some(next) = next {
                self.navigate_to(Some(next), false, cx);
            }
            return;
        }
        if let Some(tabs) = self.settings.open_tabs.as_mut() {
            tabs.retain(|id| id != &chat_id);
        }
        // Closing a pair member dissolves the pair; its partner survives at
        // full width when the closed one was selected.
        let partner = self
            .split_pair
            .as_ref()
            .and_then(|pair| pair.partner(&chat_id))
            .map(|(chat, _)| chat.to_string());
        if self.split_pair_contains(&chat_id) {
            self.dissolve_split_pair(cx);
        }
        if focused.as_deref() == Some(chat_id.as_str()) {
            if is_draft {
                // Leaving the draft on the canvas would re-open its tab.
                self.state.update(cx, |s, _| s.active_draft = None);
            }
            let next = partner.or_else(|| next_after_close(&order, &chat_id));
            self.navigate_to(next, true, cx);
        }
        self.schedule_save(cx);
        cx.notify();
    }

    /// The second rung of `⌘W`: close the focused tab. The chat route always
    /// consumes it (the canvas is a draft), so `⌘W` never closes the window
    /// from there.
    pub fn close_focused_tab(&mut self, cx: &mut Context<Self>) -> bool {
        if !matches!(self.route, Route::Chat) {
            return false;
        }
        let Some(id) = self.state.read(cx).focused_id() else {
            return false;
        };
        self.close_session_tab(id, cx);
        true
    }

    /// Bulk closes (tab context menu). `keep_side`: retain tabs left/right of
    /// the anchor per the flags. The anchor always survives; if the selected
    /// tab was closed, selection lands on the anchor.
    pub(super) fn close_tabs_around(
        &mut self,
        anchor: &str,
        keep_left: bool,
        keep_right: bool,
        cx: &mut Context<Self>,
    ) {
        let order = self.open_tab_ids(cx);
        let Some(anchor_ix) = order.iter().position(|id| id == anchor) else {
            return;
        };
        let kept: Vec<String> = order
            .iter()
            .enumerate()
            .filter(|(ix, _)| match ix.cmp(&anchor_ix) {
                std::cmp::Ordering::Less => keep_left,
                std::cmp::Ordering::Equal => true,
                std::cmp::Ordering::Greater => keep_right,
            })
            .map(|(_, id)| id.clone())
            .collect();
        let closed: std::collections::HashSet<_> =
            order.iter().filter(|id| !kept.contains(id)).collect();
        if let Some(tabs) = self.settings.open_tabs.as_mut() {
            tabs.retain(|id| !closed.contains(id));
        }
        // Closed empty drafts die with their tabs.
        let closed_empty: Vec<String> = order
            .iter()
            .filter(|id| !kept.contains(id))
            .filter(|id| {
                self.state.read(cx).is_draft(id) && !crate::draft_store::has_content(id, cx)
            })
            .cloned()
            .collect();
        for id in closed_empty {
            self.state.update(cx, |s, cx| s.discard_draft(&id, cx));
            self.frozen_draft_rows.remove(&id);
        }
        if self
            .split_pair
            .as_ref()
            .is_some_and(|pair| !kept.contains(&pair.left) || !kept.contains(&pair.right))
        {
            self.dissolve_split_pair(cx);
        }
        let selected = self.state.read(cx).focused_id();
        if let Some(selected) = selected
            && !kept.contains(&selected)
        {
            self.navigate_to(Some(anchor.to_string()), true, cx);
        }
        self.schedule_save(cx);
        cx.notify();
    }

    /// Track the drop slot while a tab is dragged over the strip (150ms sibling
    /// slides restart per committed `over` change).
    fn update_tab_drag_over(&mut self, from: usize, over: usize, cx: &mut Context<Self>) {
        match &mut self.tab_drag {
            Some(drag) if drag.from == from => {
                if drag.over != over {
                    drag.prev_over = drag.over;
                    drag.over = over;
                    drag.epoch += 1;
                    cx.notify();
                }
            }
            _ => {
                self.tab_drag = Some(TabDragState {
                    from,
                    over,
                    epoch: 0,
                    prev_over: from,
                });
                cx.notify();
            }
        }
    }

    /// Commit a drag: the reordered visual list becomes the stored list, with
    /// not-yet-synced ids kept after it (device-local).
    fn commit_tab_reorder(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        let mut order = self.open_tab_ids(cx);
        if from < order.len() {
            reorder_tabs(&mut order, from, to);
            let hidden: Vec<String> = self
                .settings
                .open_tabs
                .iter()
                .flatten()
                .filter(|id| !order.contains(id))
                .cloned()
                .collect();
            order.extend(hidden);
            self.settings.open_tabs = Some(order);
            self.schedule_save(cx);
        }
        self.tab_drag = None;
        cx.notify();
    }

    /// The scrollable session tabs (edge fades on overflow), sized to whatever
    /// the titlebar row leaves it.
    fn render_session_tabs(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let now = Utc::now();
        // A drag that ended off-strip (no drop event) must not strand the
        // sibling slide offsets.
        if self.tab_drag.is_some() && !cx.has_active_drag() {
            self.tab_drag = None;
        }
        let mut order: Vec<String> = self.open_tab_ids(cx);
        // A split's two sessions share ONE double chip (Atlas): the secondary
        // moves next to the selected one, on the side it is shown on, and
        // the pair spends exactly two slots so the drop-index math holds.
        let group: Option<(String, String)> = self.split_pair.as_ref().and_then(|pair| {
            let left_ix = order.iter().position(|id| *id == pair.left)?;
            order.iter().position(|id| *id == pair.right)?;
            order.retain(|id| *id != pair.right);
            let left_ix = order
                .iter()
                .position(|id| *id == pair.left)
                .unwrap_or(left_ix);
            order.insert(left_ix + 1, pair.right.clone());
            Some((pair.left.clone(), pair.right.clone()))
        });
        let tabs: Vec<(
            String,
            SharedString,
            Vec<zeron_proto::HarnessId>,
            ChatIndicator,
            bool,
        )> = {
            let state = self.state.read(cx);
            let chats: std::collections::HashMap<_, _> = state
                .chats
                .iter()
                .map(|chat| (chat.id.as_str(), chat))
                .collect();
            let sessions: std::collections::HashMap<_, _> = state
                .sessions
                .iter()
                .map(|session| (session.chat_id.as_str(), session))
                .collect();
            let drafts: std::collections::HashSet<_> = state
                .draft_sessions
                .iter()
                .map(|draft| draft.id.as_str())
                .collect();
            order
                .iter()
                .filter_map(|id| {
                    if drafts.contains(id.as_str()) {
                        return Some((
                            id.clone(),
                            SharedString::from("New session"),
                            Vec::new(),
                            ChatIndicator::Idle,
                            true,
                        ));
                    }
                    let chat = chats.get(id.as_str())?;
                    Some((
                        chat.id.clone(),
                        SharedString::from(transcript::single_line(
                            &chat.title.clone().unwrap_or_else(|| "New session".into()),
                        )),
                        crate::pickers::chat_harness_marks(chat),
                        if state.send_pending(&chat.id, now) {
                            ChatIndicator::Working
                        } else {
                            crate::state::display_status(
                                chat,
                                sessions.get(chat.id.as_str()).copied(),
                                now,
                            )
                        },
                        false,
                    ))
                })
                .collect()
        };
        let selected = self.state.read(cx).focused_id();
        let split = self.split_chat_id().map(str::to_string);
        // Keep the selected tab visible: on selection change, scroll it into
        // view (minimal movement — a new session's tab materializes at the far
        // right of an overflowing strip and would otherwise be stranded
        // off-screen).
        match selected.as_deref() {
            Some(sel) if self.tabs_scrolled_to.as_deref() != Some(sel) => {
                if let Some(ix) = order.iter().position(|id| id == sel) {
                    self.tabs_scroll.scroll_to_item(ix);
                }
                self.tabs_scrolled_to = Some(sel.to_string());
            }
            Some(_) => {}
            None => self.tabs_scrolled_to = None,
        }
        let hovered = self.tab_hover.clone();
        let count = tabs.len();
        let drag = self
            .tab_drag
            .as_ref()
            .map(|d| (d.from, d.over, d.epoch, d.prev_over));

        let tabs_by_id: std::collections::HashMap<
            String,
            (SharedString, Vec<zeron_proto::HarnessId>, ChatIndicator),
        > = tabs
            .iter()
            .map(|(id, title, harness, status, _)| (id.clone(), (title.clone(), harness.clone(), *status)))
            .collect();
        let tab_elements: Vec<AnyElement> =
            tabs.into_iter()
                .enumerate()
                .filter_map(|(ix, (id, title, harness, status, is_draft))| {
                    if let Some((left, right)) = &group {
                        if id == *right {
                            // Drawn as the second half of the chip below.
                            return None;
                        }
                        if id == *left {
                            let chip = split_tab_chip(
                                ix,
                                [left, right].map(|id| {
                                    let (title, harness, status) = tabs_by_id[id].clone();
                                    (id.clone(), title, harness, status)
                                }),
                                selected.as_deref(),
                                hovered.as_deref(),
                                &theme,
                                cx,
                            );
                            return Some(slide_tab(chip, ix, id.clone(), 2, drag));
                        }
                    }
                    // The split's secondary wears the selected look too: it
                    // is on screen beside the selected session.
                    let is_selected = selected.as_deref() == Some(id.as_str())
                        || split.as_deref() == Some(id.as_str());
                    let is_hovered = hovered.as_deref() == Some(id.as_str());
                    // Hover state lives in Shell (the trailing slot swaps dot ↔
                    // close), so the wash snaps off it too — gpui allows only one
                    // `on_hover` per element, and the state listener wins.
                    let dim = !is_draft && status == ChatIndicator::Idle;
                    let (text_color, bg) = if is_selected {
                        (theme.text, crate::theme::glass_selected_bg())
                    } else if is_hovered {
                        (theme.text_muted.opacity(0.8), theme.glass_hover())
                    } else if dim {
                        (theme.text_muted.opacity(0.35), crate::theme::wash(0.0))
                    } else {
                        (theme.text_muted.opacity(0.6), crate::theme::wash(0.0))
                    };
                    let glyph_alpha = match (is_selected, dim && !is_hovered) {
                        (true, _) => 0.9,
                        (false, true) => 0.35,
                        (false, false) => 0.6,
                    };
                    let brand = harness.last().copied().map(crate::pickers::harness_brand_icon);
                    let select_id = id.clone();
                    let middle_id = id.clone();
                    let menu_id = id.clone();
                    let hover_id = id.clone();
                    let trailing = tab_trailing(&id, status, is_hovered, &theme, cx);
                    let tab_el = div()
                        .id(SharedString::from(format!("session-tab-{id}")))
                        .debug_selector(|| format!("session-tab-{id}"))
                        .w(px(SESSION_TAB_WIDTH))
                        .h(px(28.0))
                        .flex_none()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .pl(px(8.0))
                        .pr(px(4.0))
                        .rounded(px(8.0))
                        .text_size(px(12.0))
                        .text_color(text_color)
                        .bg(bg)
                        .when(is_selected, |el| {
                            el.shadow(crate::theme::glass_selected_shadows())
                        })
                        .cursor_pointer()
                        // Tabs sit inside the titlebar drag strip — carve them out.
                        // NOT `.occlude()`: a BlockMouse hitbox ends the hit test,
                        // so the scroll container behind the tabs never saw wheel
                        // events and an overflowing strip could not be scrolled
                        // (tabs tile the whole region). ExceptScroll keeps the
                        // drag-region carve-out and lets the strip scroll.
                        .block_mouse_except_scroll()
                        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                        // Track hover in Shell state: the trailing slot flips
                        // between dot and close button (hover_blend only fades
                        // colors; child swaps need real state).
                        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                            if *hovered {
                                this.tab_hover = Some(hover_id.clone());
                            } else if this.tab_hover.as_deref() == Some(hover_id.as_str()) {
                                this.tab_hover = None;
                            }
                            cx.notify();
                        }))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.navigate_to(Some(select_id.clone()), true, cx);
                        }))
                        // Middle-click closes (terminal-tab parity).
                        .on_mouse_down(
                            MouseButton::Middle,
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.close_session_tab(middle_id.clone(), cx);
                            }),
                        )
                        // Right-click: close / close others / close to a side.
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                this.tab_menu.open((menu_id.clone(), event.position));
                                cx.notify();
                            }),
                        )
                        .on_drag(
                            TabDragPayload {
                                from: ix,
                                chat_id: id.clone(),
                                title: title.clone(),
                                brand,
                            },
                            |payload, _point, _, cx| {
                                let title = payload.title.clone();
                                let brand = payload.brand;
                                cx.stop_propagation();
                                cx.new(|_| TabGhost { title, brand })
                            },
                        )
                        .map(|el| {
                            match crate::computer::strip::chat_glyph(
                                "session-tab",
                                &id,
                                14.0,
                                glyph_alpha,
                                &theme,
                                cx,
                            ) {
                                Some(glyph) => el.child(glyph),
                                None => el.when(!harness.is_empty(), |el| {
                                    let selector = format!("session-tab-harness-{id}");
                                    el.child(
                                        crate::pickers::harness_mark_stack(
                                            &harness,
                                            14.0,
                                            theme.text_muted,
                                            glyph_alpha,
                                        )
                                        .debug_selector(move || selector),
                                    )
                                }),
                            }
                        })
                        // A draft wears the amber pen where a session wears
                        // its harness brand (d18); a session with unsent
                        // composer text wears both, like its sidebar row.
                        .when(is_draft || crate::draft_store::has_content(&id, cx), |el| {
                            el.child(
                                icon(icons::PEN)
                                    .size(px(13.0))
                                    .flex_none()
                                    .text_color(theme.warning.opacity(glyph_alpha)),
                            )
                        })
                        .child(div().flex_1().min_w_0().truncate().child(title))
                        .child(trailing);

                    Some(slide_tab(tab_el, ix, id, 1, drag))
                })
                .collect();

        // Overflow: the tab region scrolls horizontally; edge fades appear on
        // whichever side has hidden tabs (offset from the LAST frame — a
        // one-frame lag is invisible). The fades are an EdgeFade scope on
        // every theme: a painted surface-tone overlay shows as a dark block
        // over a chat background image.
        let scrolled = -f32::from(self.tabs_scroll.offset().x);
        let max_scroll = f32::from(self.tabs_scroll.max_offset().x);
        let fade_left = scrolled > 1.0;
        let fade_right = scrolled < max_scroll - 1.0;
        let scroll_for_drag = self.tabs_scroll.clone();
        let tab_region = div()
            .relative()
            .min_w_0()
            .child(
                div()
                    .id("session-tabs-scroll")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(TAB_GAP))
                    .min_w_0()
                    .overflow_x_scroll()
                    .track_scroll(&self.tabs_scroll)
                    // Windows caption hit-testing includes the scroll-only
                    // hitboxes behind each tab. Stop at the scroller so the
                    // titlebar cannot claim tab clicks, while wheel events
                    // still reach this scroller.
                    .when(cfg!(target_os = "windows"), |strip| strip.occlude())
                    .on_drag_move::<TabDragPayload>(cx.listener(
                        move |this, event: &gpui::DragMoveEvent<TabDragPayload>, _, cx| {
                            let from = event.drag(cx).from;
                            // Drop math runs in CONTENT coordinates: viewport-
                            // relative x plus the scrolled-off width.
                            let rel_x = f32::from(event.event.position.x)
                                - f32::from(event.bounds.left())
                                - f32::from(scroll_for_drag.offset().x);
                            let over = drop_index(rel_x, SESSION_TAB_WIDTH + TAB_GAP, count);
                            this.update_tab_drag_over(from, over, cx);
                        },
                    ))
                    .on_drop::<TabDragPayload>(cx.listener(
                        move |this, payload: &TabDragPayload, _, cx| {
                            let to = this
                                .tab_drag
                                .as_ref()
                                .map(|d| d.over)
                                .unwrap_or(payload.from);
                            this.commit_tab_reorder(payload.from, to, cx);
                        },
                    ))
                    .children(tab_elements),
            );
        crate::edge_fade::edge_faded(FADE_WIDTH, false, false, tab_region)
            .fade_left(fade_left)
            .fade_right(fade_right)
            .into_any_element()
    }

    /// The unified titlebar in chat mode:
    /// `[new-session +] [session tabs] … [project actions] [toggle-changes]`.
    /// Carries the titlebar duties (drag region, animated left inset, the
    /// right-pane controls on git projects).
    pub(super) fn render_session_title_bar(
        &mut self,
        viewport_height: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).clone();
        // The canvas shows no tabs-adjacent chrome beyond the strip itself;
        // the bar keeps its height, drag region, and buttons.
        let on_canvas = self.state.read(cx).selected_chat.is_none();

        // The new-session `+` renders in the WINDOW-CONTROL CLUSTER whenever a
        // session is selected (`render_titlebar_cluster`) — this row budgets
        // one button slot so the tabs never sit under it.
        let sidebar_now = self.sidebar_now();
        let plus_inset = TITLEBAR_ACTION_SLOT_WIDTH * self.titlebar_plus_alpha(cx);

        // Content starts at the inset card's left edge while the sidebar is
        // open, and slides toward the control cluster as it collapses.
        let content_left =
            (sidebar_now + Theme::SPACE_LG).max(self.title_bar_content_start() + plus_inset);

        // Trailing titlebar section. With the changes pane open this is the
        // PANE'S HEADER — a strip exactly as wide as the pane carrying its
        // controls (scope dropdown, ref selector, fold-all from the Changes
        // entity; expand + close shell-side). It lives up here because the
        // titlebar overlay owns this band's hit-testing: controls mounted in
        // the pane itself would sit under the drag region and never see a
        // click. Closed, it is just the stable open/close toggle.
        let right_pane_open = self.right_pane_open(cx);
        let takeover = right_pane_open && self.right_pane_expanded;
        // In takeover the tabs hide and the strip owns the whole band, so
        // the row's left inset pulls back to the sidebar seam — the tabs
        // inset would push the scope dropdown off the pane's own left gutter
        // (user report: misaligned dead space). With the sidebar COLLAPSED
        // the seam is the window edge, where the traffic lights + nav
        // cluster overlay lives — the strip must still clear it, but only
        // just: `title_bar_content_start` carries the identity-group margin the
        // strip doesn't want (it brings its own 8px pad), and doubling up
        // read as a hole after the `+` (user report).
        let row_left = if takeover {
            // The surface tabs must LEFT-ALIGN with the pane's own rows (the
            // diff options and stats strip carry an 8px box gutter off the
            // seam — user report: rows started at different insets). The
            // strip's width is capped to `avail`, which subtracts the row's
            // 8px child gap — pulling row_left 8 LEFT of the seam cancels
            // that, so the uncapped strip starts exactly at the seam and its
            // own 8px pad lands the first chip on the pane gutter. The
            // window-control cluster still wins while the sidebar is
            // collapsed (the chips clear it instead of underlapping).
            let cluster_end =
                self.title_bar_content_start() - TITLEBAR_IDENTITY_GAP + plus_inset - 14.0;
            (sidebar_now - 8.0).max(cluster_end)
        } else {
            content_left
        };
        let row_gap = 8.0;
        let files_width = self.files_visible_width(cx);
        let right_pad = self.titlebar_right_pad(TITLEBAR_ACTION_EDGE_INSET);
        // The row's gaps are outside the fixed-width panel controls.
        let gap_budget = if takeover { row_gap } else { row_gap * 3.0 };
        let right_visible = self.right_visible_width(cx);
        let widths = panel_titlebar_widths(
            right_visible,
            files_width,
            self.viewport_width - row_left - right_pad - gap_budget,
            right_pad,
        );
        let surface_reveal = widths.surface_reveal;
        // The trailing strip always carries the explorer slot with its two
        // toggles; the surface tabs reveal to their left only while the
        // surface host is open.
        let trailing_width = {
            let surface = if right_pane_open { surface_reveal } else { 0.0 };
            surface + widths.files_controls
        };
        let available_titlebar_width =
            (self.viewport_width - row_left - right_pad - trailing_width - row_gap * 3.0).max(0.0);

        let trailing: Option<gpui::AnyElement> = {
            let mut controls = div()
                .id("right-titlebar-controls")
                .flex_none()
                .h_full()
                .flex()
                .flex_row()
                .items_center()
                // The transcript extends under this band (it fades beneath
                // the tab strip); a wheel over the tabs must scroll the
                // strip, never the surface behind it.
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation());
            if right_pane_open {
                // The right pane's SURFACE TABS (t3 RightPanelTabs) — the diff
                // options that used to live here moved into the pane's own
                // second row; expand stays in this band (user request).
                let tabs = self.render_right_tab_strip(cx);
                // The summary + toggles are the fixed right-edge anchor, like
                // the left sidebar control. Only the tabs + expand section
                // reveals to its left; including the toggle in this animated
                // width compressed both icons into the same clipped box at
                // open.
                controls = controls.child(
                    div()
                        .w(px(surface_reveal))
                        .h_full()
                        .flex_none()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(4.0))
                        .overflow_hidden()
                        // 8 + the trigger's own 8px pad = the pane's 16px
                        // text gutter. The 4px right padding is the stable
                        // gap before the fixed toggle.
                        .pl(px(8.0))
                        .pr(px(4.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                .overflow_hidden()
                                .child(tabs),
                        )
                        .child(header_icon_button(
                            "expand-changes",
                            right_pane_expand_icon(self.right_pane_expanded),
                            if self.right_pane_expanded {
                                "Collapse panel"
                            } else {
                                "Expand panel"
                            },
                            &theme,
                            cx.listener(|this, _, _, cx| this.toggle_right_pane_expand(cx)),
                        )),
                );
            }
            let files_panel_label = if self.files_panel_open(cx) {
                "Hide files panel"
            } else {
                "Show files panel"
            };
            // The explorer slot sits over the explorer column and carries the
            // two fixed right-edge anchors — the explorer toggle and,
            // outermost, the pane toggle — which stay mounted at one position
            // while the surface tabs reveal to their left. The explorer's own
            // search and visibility controls live in its secondary header.
            Some(
                controls
                    .child(
                        div()
                            .w(px(widths.files_controls))
                            .h_full()
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap(px(PANEL_TOGGLE_GAP))
                            // No hairline here: the explorer column below is
                            // padded down by the titlebar height and its left
                            // border already runs through this band, so a
                            // second one on the slot stacked on the same
                            // pixels and read lighter than the seam beneath
                            // it (user report).
                            .when(!on_canvas, |slot| {
                                slot.child(
                                    header_icon_button(
                                        "toggle-files-panel",
                                        icons::FILE_TREE,
                                        files_panel_label,
                                        &theme,
                                        cx.listener(|this, _, window, cx| {
                                            this.toggle_files_panel(window, cx)
                                        }),
                                    )
                                    .role(gpui::Role::Button)
                                    .aria_label(files_panel_label)
                                    .when(self.files_panel_open(cx), |button| {
                                        button.bg(crate::theme::wash(0.09))
                                    }),
                                )
                            })
                            .child(header_icon_button_with(
                                "toggle-changes",
                                icons::sidebar_glyph(
                                    motion::state_t(
                                        "toggle-changes",
                                        right_pane_open,
                                        motion::GLYPH_STATE,
                                        self.reduced_motion,
                                    ),
                                    true,
                                    16.0,
                                    theme.text_muted,
                                ),
                                ShortcutId::ToggleChanges.label(),
                                cx.listener(|this, _, _, cx| this.toggle_right_pane(cx)),
                            )),
                    )
                    .into_any_element(),
            )
        };

        // In panel takeover the header strip spans the whole band — the tabs
        // would sit UNDER it (both flex_none, the row overflows and paint
        // order stacks them), so they hide for the duration.
        let session_tabs = (!takeover).then(|| self.render_session_tabs(cx));
        // The session's own side-chat controls: "+" mints a fresh side chat
        // under this session, fork copies its history into one. Same pair
        // the side-chat header carries, so a family reads the same from
        // either end.
        let session_controls = (!takeover && !on_canvas).then(|| {
            let busy = self.side_chat_creating;
            div()
                .flex_none()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(2.0))
                .child(
                    header_icon_button(
                        "session-new-side-chat",
                        icons::PLUS,
                        "New side chat",
                        &theme,
                        cx.listener(|this, _, _, cx| this.create_child_chat(None, cx)),
                    )
                    .role(gpui::Role::Button)
                    .aria_label("New side chat")
                    .when(busy, |el| el.opacity(0.4)),
                )
                .child(
                    header_icon_button(
                        "session-fork",
                        icons::FORK,
                        "Fork this session",
                        &theme,
                        cx.listener(|this, _, _, cx| this.create_side_chat(cx)),
                    )
                    .role(gpui::Role::Button)
                    .aria_label("Fork this session")
                    .when(busy, |el| el.opacity(0.4)),
                )
        });
        let available_titlebar_width = if session_controls.is_some() {
            (available_titlebar_width - SESSION_CONTROLS_WIDTH).max(0.0)
        } else {
            available_titlebar_width
        };
        let actions = (!takeover && !on_canvas)
            .then(|| {
                self.render_project_actions_control(available_titlebar_width, viewport_height, cx)
            })
            .flatten();

        let inner = div()
            .size_full()
            .flex()
            .items_center()
            .pt(px(Theme::TITLEBAR_TOP_PAD))
            .gap(px(row_gap))
            .pl(px(row_left))
            .pr(px(right_pad))
            .children(session_tabs)
            .child(div().flex_1())
            .children(session_controls)
            .children(actions)
            .children(trailing);

        // The unified window titlebar: full-width on the glass shell, ABOVE
        // the inset card. No bottom border — the card's own hairline is the
        // separation; the glass gutter shows between.
        let bar = div().h(px(Theme::TITLEBAR_HEIGHT)).flex_none().child(inner);
        self.titlebar_drag_region("chat-titlebar", bar, cx)
            .into_any_element()
    }

    fn close_tab_menu(&mut self, cx: &mut Context<Self>) {
        if self.tab_menu.begin_close() {
            popover::reap_popup(cx, |shell: &mut Self| &mut shell.tab_menu);
            cx.notify();
        }
    }

    /// Tab context menu (right-click): close, plus the bulk closes that apply
    /// at this tab's position — rows that would close nothing are omitted.
    pub(super) fn render_tab_menu(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (chat_id, position) = self.tab_menu.get().cloned()?;
        let closing = self.tab_menu.closing_since();
        let theme = Theme::of(cx).clone();
        let order = self.open_tab_ids(cx);
        let ix = match order.iter().position(|id| id == &chat_id) {
            Some(ix) => ix,
            // The tab itself vanished mid-exit ("Close tab") — keep the card
            // mounted so the fade still plays.
            None if closing.is_some() => 0,
            None => return None,
        };
        let has_others = order.len() > 1;
        let has_right = ix + 1 < order.len();
        let has_left = ix > 0;
        let has_idle = !self.idle_tab_ids(cx).is_empty();

        let row = |label: &'static str,
                   id: &'static str,
                   keep_left: bool,
                   keep_right: bool,
                   cx: &mut Context<Self>| {
            let target = chat_id.clone();
            popover::menu_row(&theme, false, format!("{id}-{target}"))
                .id(id)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.close_tab_menu(cx);
                    if keep_left && keep_right {
                        this.close_session_tab(target.clone(), cx);
                    } else {
                        this.close_tabs_around(&target, keep_left, keep_right, cx);
                    }
                }))
                .child(SharedString::from(label))
        };

        let mut menu = popover::popover_card(&theme)
            .w(px(170.0))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.close_tab_menu(cx);
            }))
            .flex()
            .flex_col()
            // keep_left && keep_right = "just this one" (see `row`).
            .child(row("Close tab", "tab-menu-close", true, true, cx));
        if has_others {
            menu = menu.child(row("Close others", "tab-menu-others", false, false, cx));
        }
        if has_right {
            menu = menu.child(row("Close to the right", "tab-menu-right", true, false, cx));
        }
        if has_left {
            menu = menu.child(row("Close to the left", "tab-menu-left", false, true, cx));
        }
        if has_idle {
            menu = menu.child(
                popover::menu_row(&theme, false, format!("tab-menu-idle-{chat_id}"))
                    .id("tab-menu-idle")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.close_tab_menu(cx);
                        this.close_idle_tabs(cx);
                    }))
                    .child(SharedString::from("Close idle tabs")),
            );
        }
        Some(popover::menu_at(
            "tab-context-menu",
            position,
            menu.into_any_element(),
            closing,
        ))
    }
}

#[cfg(test)]
mod panel_titlebar_tests {
    use super::*;

    #[test]
    fn tabs_align_with_the_panel_for_each_caption_layout_and_files_width() {
        let viewport = 1400.0;
        for right_pad in [6.0, 40.0, 92.0, 114.0] {
            for files in [0.0, 10.0, 28.0, 100.0, 220.0, 286.0, 440.0] {
                let widths = panel_titlebar_widths(520.0, files, 1100.0, right_pad);
                let controls_left =
                    viewport - right_pad - widths.files_controls - widths.surface_reveal;
                assert_eq!(
                    controls_left,
                    viewport - files - 520.0,
                    "caption clearance {right_pad}, Files width {files}"
                );
                assert!(widths.files_controls >= PANEL_TOGGLE_SLOTS);
                if files >= right_pad + PANEL_TOGGLE_SLOTS {
                    assert_eq!(
                        viewport - right_pad - widths.files_controls,
                        viewport - files
                    );
                }
            }
        }
    }

    #[test]
    fn expanded_tabs_clear_the_left_controls_without_reserving_captions_twice() {
        for right_pad in [6.0, 92.0, 114.0] {
            let viewport = 1400.0;
            let files = 286.0;
            // With the sidebar open, the header starts exactly at its seam.
            // With it collapsed, leave room for the window/nav controls.
            for (sidebar, row_left) in [(256.0, 248.0), (0.0, 180.0)] {
                let widths = panel_titlebar_widths(
                    viewport - sidebar - files,
                    files,
                    viewport - row_left - right_pad - 8.0,
                    right_pad,
                );
                let controls_left =
                    viewport - right_pad - widths.files_controls - widths.surface_reveal;
                assert_eq!(controls_left, sidebar.max(row_left + 8.0));
            }
        }
    }

    #[test]
    fn narrow_panels_and_tight_headers_keep_nonnegative_reveal_widths() {
        // A narrow surface and Files share a 520px header.
        let widths = panel_titlebar_widths(234.0, 286.0, 600.0, 92.0);
        assert_eq!(
            1000.0 - 92.0 - widths.files_controls - widths.surface_reveal,
            480.0
        );
        for available in [-20.0, 0.0, 28.0, 56.0, 100.0] {
            let widths = panel_titlebar_widths(0.0, 0.0, available, 114.0);
            assert_eq!(widths.surface_reveal, 0.0);
            assert_eq!(widths.files_controls, PANEL_TOGGLE_SLOTS);
        }
    }
}

#[cfg(test)]
mod strip_regressions {
    use super::*;
    use gpui::{AppContext, TestAppContext, VisualTestContext};

    // Render the production titlebar with its real Shell callbacks, without
    // starting an engine or rendering the rest of the desktop application.
    struct StripHost {
        shell: Entity<Shell>,
        _data_dir: tempfile::TempDir,
    }

    impl Render for StripHost {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let viewport_height = window.viewport_size().height;
            self.shell.update(cx, |shell, cx| {
                div()
                    .w(px(900.))
                    .h(px(60.))
                    .child(shell.render_session_title_bar(viewport_height, cx))
            })
        }
    }

    fn chat(id: &str) -> zeron_proto::Chat {
        serde_json::from_value(serde_json::json!({
            "id": id, "deviceId": "local", "spaceId": "a", "archived": false,
            "createdAt": Utc::now(),
        }))
        .unwrap()
    }

    /// A live Working run for `id` — the status that keeps a tab on the
    /// strip regardless of selection (selecting a chat marks it seen, so a
    /// finished-but-unseen fixture would go Idle on first select).
    fn working(id: &str) -> zeron_proto::Session {
        zeron_proto::Session {
            last_completed_turn: None,
            held_by_subagents: false,
            compacting: false,
            background_tasks: Vec::new(),
            chat_id: id.into(),
            device_id: "local".into(),
            status: zeron_proto::SessionStatus::Working,
            started_at: None,
            updated_at: Utc::now(),
        }
    }

    fn setup(cx: &mut TestAppContext) -> (Entity<Shell>, &mut VisualTestContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            settings::init(settings::UiSettings::default(), dir.path(), cx);
            crate::draft_store::init(dir.path(), cx);
            crate::history::init(
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                cx,
            );
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            crate::app_menus::init(cx);
        });
        let (host, cx) = cx.add_window_view(|_, cx| {
            let shell = cx.new(|cx| {
                let state = cx.new(|_| AppState::new());
                Shell::new(
                    state,
                    EngineBootConfig {
                        data_dir: dir.path().into(),
                        ipc_port: 0,
                        edge_url: "http://127.0.0.1:1".into(),
                        edge_token: None,
                        org_id: None,
                        workos_client_id: None,
                        default_harness: zeron_proto::HarnessId::Mock,
                    },
                    cx,
                )
            });
            StripHost {
                shell,
                _data_dir: dir,
            }
        });
        let shell = host.read_with(cx, |host, _| host.shell.clone());
        (shell, cx)
    }

    #[gpui::test]
    fn cmd_w_closes_the_focused_tab_and_keeps_the_window(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, cx| {
                state.chats = vec![chat("a"), chat("b")];
                state.chats_synced = true;
                state.spaces_synced = true;
                state.apply_sessions(vec![working("a"), working("b")]);
                state.select_chat(Some("a".into()), cx);
            });
            shell.on_state_changed(&shell.state.clone(), cx);
            shell.open_chat("b".into(), cx);
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(shell.open_tab_ids(cx), ["a", "b"]);

            assert!(shell.close_focused_tab(cx));
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(shell.open_tab_ids(cx), ["a"]);
            assert_eq!(shell.state.read(cx).selected_chat.as_deref(), Some("a"));

            assert!(shell.close_focused_tab(cx));
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(shell.state.read(cx).selected_chat, None);
            assert!(shell.close_focused_tab(cx));

            shell.route = Route::Settings(SettingsSection::default());
            assert!(!shell.close_focused_tab(cx));
        });
    }

    #[gpui::test]
    fn ctrl_tab_cycles_the_strip_when_the_setting_is_on(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, cx| {
                state.chats = vec![chat("a"), chat("b"), chat("c")];
                state.chats_synced = true;
                state.spaces_synced = true;
                state.apply_sessions(vec![working("a"), working("b"), working("c")]);
                state.select_chat(Some("a".into()), cx);
            });
            shell.on_state_changed(&shell.state.clone(), cx);
            for id in ["b", "c", "a"] {
                shell.open_chat(id.into(), cx);
                shell.on_state_changed(&shell.state.clone(), cx);
            }
            assert_eq!(shell.open_tab_ids(cx), ["a", "b", "c"]);
            shell.settings.switcher_cycles_tabs = true;

            let focused = |shell: &Shell, cx: &App| shell.state.read(cx).focused_id();
            shell.switcher_step(true, cx);
            assert!(shell.switcher.is_none());
            assert_eq!(focused(shell, cx).as_deref(), Some("b"));
            shell.switcher_step(false, cx);
            shell.switcher_step(false, cx);
            assert_eq!(focused(shell, cx).as_deref(), Some("c"));
            shell.switcher_step(true, cx);
            assert_eq!(focused(shell, cx).as_deref(), Some("a"));
            assert!(shell.switcher.is_none());
        });
    }

    #[gpui::test]
    fn selection_opens_a_tab_and_close_moves_to_the_neighbor(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, cx| {
                state.chats = vec![chat("a"), chat("b")];
                state.chats_synced = true;
                state.spaces_synced = true;
                state.apply_sessions(vec![working("a"), working("b")]);
                state.select_chat(Some("a".into()), cx);
            });
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(shell.open_tab_ids(cx), ["a"]);
            // Every selection path (sidebar, palette, banner) funnels through
            // open_chat; the invariant appends the tab.
            shell.open_chat("b".into(), cx);
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(shell.open_tab_ids(cx), ["a", "b"]);
        });
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(cx.debug_bounds("session-tab-a").is_some());
        assert!(cx.debug_bounds("session-tab-b").is_some());

        shell.update(cx, |shell, cx| {
            shell.close_session_tab("b".into(), cx);
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(shell.state.read(cx).selected_chat.as_deref(), Some("a"));
            assert_eq!(shell.open_tab_ids(cx), ["a"]);
            // Closing the tab never touched the session itself.
            assert!(!shell.state.read(cx).chats[1].archived);

            // A session archived elsewhere leaves the strip on the next sync.
            shell.open_chat("b".into(), cx);
            shell.on_state_changed(&shell.state.clone(), cx);
            shell.open_chat("a".into(), cx);
            shell
                .state
                .update(cx, |state, _| state.chats[1].archived = true);
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(shell.open_tab_ids(cx), ["a"]);
            assert_eq!(
                shell.settings.open_tabs.as_deref(),
                Some(&["a".to_string()][..])
            );

            // The last tab closes onto the new-session canvas — itself a
            // draft with a tab (d18/d20), so the strip holds that one tab.
            shell.close_session_tab("a".into(), cx);
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(shell.state.read(cx).selected_chat, None);
            let draft = shell
                .state
                .read(cx)
                .active_draft
                .clone()
                .expect("canvas draft");
            assert_eq!(shell.open_tab_ids(cx), [draft]);
        });
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(cx.debug_bounds("session-tab-a").is_none());
    }

    #[gpui::test]
    fn a_driving_chat_wears_the_cursor_glyph_and_gets_its_harness_mark_back_after(
        cx: &mut TestAppContext,
    ) {
        use crate::computer::preview::{self, tests::thread_state};
        use zeron_proto::computer::{ComputerEvent, ComputerInputPause};

        cx.update(preview::init);
        let (shell, cx) = setup(cx);
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, cx| {
                let mut driven = chat("a");
                driven.config = serde_json::from_value(serde_json::json!({
                    "harness": "claude-code", "sandbox": "workspace-write"
                }))
                .ok();
                state.chats = vec![driven, chat("b")];
                state.chats_synced = true;
                state.spaces_synced = true;
                state.apply_sessions(vec![working("a"), working("b")]);
                state.select_chat(Some("a".into()), cx);
            });
            shell.on_state_changed(&shell.state.clone(), cx);
            shell.open_chat("b".into(), cx);
            shell.on_state_changed(&shell.state.clone(), cx);
        });
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(cx.debug_bounds("session-tab-harness-a").is_some());
        let store = cx.update(|_, cx| preview::store(cx).unwrap());
        let mut apply = |state: zeron_proto::computer::ThreadComputerState| {
            cx.update(|window, cx| {
                preview::update_store(&store, cx, |store| {
                    store.apply_event(&ComputerEvent::ThreadState {
                        state: Box::new(state),
                    })
                });
                window.draw(cx).clear();
            });
            (
                cx.debug_bounds("session-tab-computer-a").is_some(),
                cx.debug_bounds("session-tab-harness-a").is_some(),
                cx.debug_bounds("session-tab-computer-b").is_some(),
            )
        };
        assert_eq!(apply(thread_state("a", true)), (true, false, false));
        let mut paused = thread_state("a", true);
        paused.input_pause = Some(ComputerInputPause {
            window_id: Some("w1".into()),
            pid: None,
            message: "Screen locked".into(),
            user_takeover: false,
        });
        assert_eq!(
            apply(paused),
            (true, false, false),
            "paused still holds the desktop"
        );
        let mut revoked = thread_state("a", false);
        revoked.input_stopped = Some(true);
        assert_eq!(
            apply(revoked),
            (false, true, false),
            "revoked wears the harness mark"
        );
        assert_eq!(apply(thread_state("a", false)), (false, true, false));
    }

    fn invest(id: &str, text: &str, cx: &mut gpui::App) {
        crate::draft_store::set(
            id,
            crate::draft_store::DraftEntry {
                prompt: text.into(),
                attachments: Vec::new(),
                appshots: Vec::new(),
            },
            cx,
        );
    }

    #[gpui::test]
    fn drafts_are_tabs_and_empty_ones_die_when_left(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state.chats_synced = true;
                state.spaces_synced = true;
            });
            // Boot: the canvas is a draft from birth, with a tab (d18/d20).
            shell.on_state_changed(&shell.state.clone(), cx);
            let first = shell
                .state
                .read(cx)
                .active_draft
                .clone()
                .expect("boot draft");
            assert_eq!(shell.open_tab_ids(cx), [first.clone()]);
            // ⌘N on an empty draft reuses it.
            shell.open_new_session(None, cx);
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(
                shell.state.read(cx).active_draft.as_deref(),
                Some(first.as_str())
            );
            assert_eq!(shell.open_tab_ids(cx), [first.clone()]);
            // ⌘N on an invested draft opens a second one beside it.
            invest(&first, "keep me", cx);
            shell.open_new_session(None, cx);
            shell.on_state_changed(&shell.state.clone(), cx);
            let second = shell.state.read(cx).active_draft.clone().unwrap();
            assert_ne!(second, first);
            assert_eq!(shell.open_tab_ids(cx), [first.clone(), second.clone()]);
            // Going back to the invested draft discards the empty one.
            shell.navigate_to(Some(first.clone()), false, cx);
            shell.on_state_changed(&shell.state.clone(), cx);
            assert!(!shell.state.read(cx).is_draft(&second));
            assert_eq!(shell.open_tab_ids(cx), [first.clone()]);
            // Closing an invested draft's tab keeps its session (sidebar row)
            // and lands on a fresh canvas draft.
            shell.close_session_tab(first.clone(), cx);
            shell.on_state_changed(&shell.state.clone(), cx);
            assert!(shell.state.read(cx).is_draft(&first));
            let fresh = shell.state.read(cx).active_draft.clone().unwrap();
            assert_ne!(fresh, first);
            assert_eq!(shell.open_tab_ids(cx), [fresh]);
            // Discarding drops the session and its stored text.
            shell.discard_draft(first.clone(), cx);
            assert!(!shell.state.read(cx).is_draft(&first));
            assert!(!crate::draft_store::has_content(&first, cx));
        });
    }

    #[gpui::test]
    fn a_draft_promotes_into_the_chat_that_carries_its_id(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state.chats_synced = true;
                state.spaces_synced = true;
            });
            shell.on_state_changed(&shell.state.clone(), cx);
            let draft = shell.state.read(cx).active_draft.clone().unwrap();
            invest(&draft, "send me", cx);
            // Send selects the chat minted with the draft's id: the session
            // is gone, nothing re-keyed, the tab is the same string.
            shell
                .state
                .update(cx, |s, cx| s.select_chat(Some(draft.clone()), cx));
            shell.on_state_changed(&shell.state.clone(), cx);
            let state = shell.state.read(cx);
            assert!(!state.is_draft(&draft));
            assert_eq!(state.active_draft, None);
            assert_eq!(state.selected_chat.as_deref(), Some(draft.as_str()));
            drop(state);
            // The stored tab list keeps the id through the send→doc-frame
            // gap (selected exempt), and the strip shows it once the row lands.
            assert_eq!(
                shell.settings.open_tabs.as_deref(),
                Some(&[draft.clone()][..])
            );
            shell
                .state
                .update(cx, |state, _| state.chats.push(chat(&draft)));
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(shell.open_tab_ids(cx), [draft]);
        });
    }

    /// A drop on the new-session canvas splits beside the draft instead of
    /// opening the session; the pair survives state syncs, rides the
    /// draft's promotion into the chat carrying its id, and never shows the
    /// draft as a secondary (it has no doc).
    #[gpui::test]
    fn dropping_on_the_draft_canvas_splits_and_survives_promotion(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state.chats = vec![chat("b")];
                state.chats_synced = true;
                state.spaces_synced = true;
                // Stay on the canvas: no boot landing into b.
                state.auto_selected = true;
            });
            shell.on_state_changed(&shell.state.clone(), cx);
            let draft = shell.state.read(cx).active_draft.clone().unwrap();
            invest(&draft, "keep me", cx);
            shell.drop_session_on_side("b".into(), split::SplitSide::Right, cx);
            assert_eq!(
                shell.state.read(cx).selected_chat,
                None,
                "drop must not open b"
            );
            assert_eq!(shell.split_chat_id(), Some("b"));
            assert_eq!(
                shell.split_pair,
                Some(split::SplitPair {
                    left: draft.clone(),
                    right: "b".into(),
                })
            );
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(
                shell.split_chat_id(),
                Some("b"),
                "the draft pair survives a sync"
            );
            // The chat side of the pair shows full width: no draft pane.
            shell.open_chat("b".into(), cx);
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(shell.split_chat_id(), None);
            assert!(shell.split_pair.is_some());
            shell.navigate_to(Some(draft.clone()), false, cx);
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(
                shell.split_chat_id(),
                Some("b"),
                "back on the draft the split returns"
            );
            // Send: the chat minted with the draft's id is selected before
            // its row syncs, and the pair holds through the gap.
            shell
                .state
                .update(cx, |s, cx| s.select_chat(Some(draft.clone()), cx));
            shell.on_state_changed(&shell.state.clone(), cx);
            assert!(!shell.state.read(cx).is_draft(&draft));
            assert_eq!(shell.split_chat_id(), Some("b"));
            shell
                .state
                .update(cx, |state, _| state.chats.push(chat(&draft)));
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(shell.split_chat_id(), Some("b"));
            assert_eq!(shell.open_tab_ids(cx), [draft, "b".into()]);
        });
    }

    /// Archiving the SELECTED member dissolves the pair on that very sync,
    /// so the partner's sidebar row drops its side glyph without a click.
    #[gpui::test]
    fn archiving_the_selected_member_dissolves_the_pair(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, cx| {
                state.chats = vec![chat("a"), chat("b")];
                state.chats_synced = true;
                state.spaces_synced = true;
                state.apply_sessions(vec![working("a"), working("b")]);
                state.select_chat(Some("a".into()), cx);
            });
            shell.on_state_changed(&shell.state.clone(), cx);
            shell.drop_session_on_side("b".into(), split::SplitSide::Right, cx);
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(shell.split_chat_id(), Some("b"));
            shell
                .state
                .update(cx, |state, _| state.chats[0].archived = true);
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(shell.state.read(cx).selected_chat.as_deref(), Some("a"));
            assert_eq!(shell.split_pair, None, "archived member must end the pair");
            assert_eq!(shell.split_chat_id(), None);
        });
    }

    /// Each pane's close removes that pane: the selected one hands the
    /// column to its partner, the secondary leaves the selection full.
    #[gpui::test]
    fn closing_the_selected_pane_hands_the_column_to_its_partner(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, cx| {
                state.chats = vec![chat("a"), chat("b")];
                state.chats_synced = true;
                state.spaces_synced = true;
                state.select_chat(Some("a".into()), cx);
            });
            shell.on_state_changed(&shell.state.clone(), cx);
            shell.drop_session_on_side("b".into(), split::SplitSide::Right, cx);
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(shell.split_chat_id(), Some("b"));
            shell.close_primary_split(cx);
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(shell.state.read(cx).selected_chat.as_deref(), Some("b"));
            assert_eq!(shell.split_pair, None);
            assert_eq!(shell.split_chat_id(), None);
        });
    }

    /// The sidebar keeps the pair's card while another session is shown.
    #[gpui::test]
    fn the_sidebar_card_outlives_leaving_the_split(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, cx| {
                state.chats = vec![chat("a"), chat("b"), chat("c")];
                state.chats_synced = true;
                state.spaces_synced = true;
                state.select_chat(Some("a".into()), cx);
            });
            shell.on_state_changed(&shell.state.clone(), cx);
            shell.drop_session_on_side("b".into(), split::SplitSide::Right, cx);
            shell.on_state_changed(&shell.state.clone(), cx);
            shell.open_chat("c".into(), cx);
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(shell.split_chat_id(), None);
            assert_eq!(
                shell.sidebar_split_card(&[]),
                Some(split::SplitPair {
                    left: "a".into(),
                    right: "b".into(),
                })
            );
            assert_eq!(shell.sidebar_split_card(&["a".into()]), None);
        });
    }

    #[gpui::test]
    fn background_start_keeps_the_tab_and_mints_the_next_draft(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state.chats_synced = true;
                state.spaces_synced = true;
            });
            shell.on_state_changed(&shell.state.clone(), cx);
            let draft = shell.state.read(cx).active_draft.clone().unwrap();
            invest(&draft, "run in the background", cx);
            shell.state.update(cx, |s, cx| {
                s.record_draft_checkout(Some("feat/x".into()), true, cx);
                s.begin_pending_send(&draft, "m1", Utc::now());
                s.start_draft_in_background(&draft, cx);
            });
            shell.on_state_changed(&shell.state.clone(), cx);
            let state = shell.state.read(cx);
            let next = state.active_draft.clone().expect("next draft");
            assert_ne!(next, draft);
            assert_eq!(state.selected_chat, None);
            assert!(!state.is_draft(&draft));
            let session = state.draft_session(&next).unwrap();
            assert_eq!(session.branch.as_deref(), Some("feat/x"));
            assert!(session.new_worktree);
            drop(state);
            // The started session's tab survives on its pending send until
            // the doc frame lands; the next draft sits beside it.
            assert_eq!(shell.open_tab_ids(cx), [draft, next]);
        });
    }

    #[gpui::test]
    fn bulk_close_closes_idle_tabs_like_any_other(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, cx| {
                state.chats = ["left", "idle", "anchor", "right"].map(chat).to_vec();
                state.chats_synced = true;
                state.spaces_synced = true;
                state.apply_sessions(vec![working("left"), working("anchor"), working("right")]);
                state.select_chat(Some("anchor".into()), cx);
            });
            for (keep_left, keep_right, expected) in [
                (false, false, vec!["anchor"]),
                (true, false, vec!["left", "idle", "anchor"]),
                (false, true, vec!["anchor", "right"]),
            ] {
                shell.settings.open_tabs = Some(
                    ["left", "idle", "anchor", "right"]
                        .map(String::from)
                        .to_vec(),
                );
                shell.close_tabs_around("anchor", keep_left, keep_right, cx);
                assert_eq!(shell.settings.open_tabs.as_ref().unwrap(), &expected);
            }
        });
    }

    #[gpui::test]
    fn idle_tabs_stay_where_they_are(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, cx| {
                state.chats = vec![chat("busy"), chat("quiet")];
                state.chats_synced = true;
                state.spaces_synced = true;
                state.apply_sessions(vec![working("busy")]);
                state.select_chat(Some("busy".into()), cx);
            });
            shell.on_state_changed(&shell.state.clone(), cx);
            shell.open_chat("quiet".into(), cx);
            shell.on_state_changed(&shell.state.clone(), cx);
            shell.open_chat("busy".into(), cx);
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(shell.open_tab_ids(cx), ["busy", "quiet"]);
        });
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(cx.debug_bounds("session-tab-quiet").is_some());
    }

    fn loose_chat(id: &str, extra: serde_json::Value) -> zeron_proto::Chat {
        let mut json = serde_json::json!({
            "id": id, "deviceId": "local", "archived": false,
            "createdAt": Utc::now() - chrono::Duration::hours(1),
        });
        json.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::from_value(json).unwrap()
    }

    fn with_status(id: &str, status: zeron_proto::SessionStatus) -> zeron_proto::Session {
        zeron_proto::Session {
            status,
            ..working(id)
        }
    }

    #[gpui::test]
    fn runs_and_questions_open_their_tab_once(cx: &mut TestAppContext) {
        use zeron_proto::SessionStatus::{AwaitingInput, Idle, Working};
        let (shell, cx) = setup(cx);
        let sync =
            |shell: &mut Shell, sessions: Vec<zeron_proto::Session>, cx: &mut Context<Shell>| {
                shell.state.update(cx, |state, _| {
                    state.apply_sessions(sessions);
                });
                shell.on_state_changed(&shell.state.clone(), cx);
            };
        shell.update(cx, |shell, cx| {
            shell.settings.open_tabs = Some(vec!["home".into()]);
            shell.state.update(cx, |state, cx| {
                state.chats = vec![
                    loose_chat("home", serde_json::json!({})),
                    loose_chat("boot", serde_json::json!({})),
                    loose_chat("phone", serde_json::json!({})),
                    loose_chat("spawned", serde_json::json!({ "createdByChatId": "home" })),
                    loose_chat("side", serde_json::json!({ "sidechatSourceChatId": "home", "lastMessageAt": Utc::now() })),
                ];
                state.chats_synced = true;
                state.spaces_synced = true;
                state.select_chat(Some("home".into()), cx);
            });
            // Already running when the viewer attaches: baseline, no tab.
            sync(shell, vec![with_status("boot", Working)], cx);
            assert_eq!(shell.open_tab_ids(cx), ["home"]);

            // A person's chat starting a run opens; agent-made ones don't.
            sync(
                shell,
                vec![
                    with_status("boot", Working),
                    with_status("phone", Working),
                    with_status("spawned", Working),
                    with_status("side", Working),
                ],
                cx,
            );
            assert_eq!(shell.open_tab_ids(cx), ["home", "phone"]);

            // Any chat waiting on the user opens.
            sync(
                shell,
                vec![
                    with_status("boot", Working),
                    with_status("phone", Working),
                    with_status("spawned", AwaitingInput),
                    with_status("side", Working),
                ],
                cx,
            );
            assert_eq!(shell.open_tab_ids(cx), ["home", "phone", "spawned"]);

            // A closed tab stays closed for the rest of that run...
            shell.close_session_tab("phone".into(), cx);
            sync(
                shell,
                vec![with_status("phone", Working), with_status("spawned", Working)],
                cx,
            );
            assert_eq!(shell.open_tab_ids(cx), ["home", "spawned"]);
            // A host that slept past the lease is the same run, not a new one.
            let mut asleep = with_status("phone", Working);
            asleep.updated_at = Utc::now() - chrono::Duration::minutes(5);
            sync(shell, vec![asleep], cx);
            sync(shell, vec![with_status("phone", Working)], cx);
            assert_eq!(shell.open_tab_ids(cx), ["home", "spawned"]);
            // ...and comes back with the next one.
            sync(shell, vec![with_status("phone", Idle)], cx);
            sync(shell, vec![with_status("phone", Working)], cx);
            assert_eq!(shell.open_tab_ids(cx), ["home", "spawned", "phone"]);
        });
    }

    #[gpui::test]
    fn upgrade_drops_the_tabs_the_activity_strip_kept_hidden(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        shell.update(cx, |shell, cx| {
            shell.settings.open_tabs = Some(["a", "hidden", "b"].map(String::from).to_vec());
            shell.state.update(cx, |state, cx| {
                state.chats = ["a", "hidden", "b"].map(chat).to_vec();
                state.chats_synced = true;
                state.spaces_synced = true;
                state.select_chat(Some("a".into()), cx);
            });
            // Without a sessions frame every chat reads Idle: wait.
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(shell.open_tab_ids(cx), ["a", "hidden", "b"]);
            shell.state.update(cx, |state, _| {
                state.apply_sessions(vec![working("b")]);
            });
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(shell.open_tab_ids(cx), ["a", "b"]);
            assert!(shell.settings.open_tabs_stable);
            // Once only: later idle tabs stay.
            shell.open_chat("b".into(), cx);
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(shell.open_tab_ids(cx), ["a", "b"]);
        });
    }

    #[gpui::test]
    fn close_idle_tabs_keeps_focus_and_active_ones(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, cx| {
                state.chats = ["a", "b", "c", "d"].map(chat).to_vec();
                state.chats_synced = true;
                state.spaces_synced = true;
                state.apply_sessions(vec![working("c")]);
                state.select_chat(Some("a".into()), cx);
            });
            shell.settings.open_tabs = Some(["a", "b", "c", "d"].map(String::from).to_vec());
            shell.close_idle_tabs(cx);
            assert_eq!(shell.open_tab_ids(cx), ["a", "c"]);
        });
    }

    #[gpui::test]
    fn switcher_cap_holds_against_idle_strip_tabs(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        shell.update(cx, |shell, cx| {
            let ids = ["a", "b", "c", "d", "e", "f"];
            shell.state.update(cx, |state, cx| {
                state.chats = ids
                    .map(|id| {
                        let mut chat = chat(id);
                        chat.space_id = None;
                        chat.last_message_at = Some(Utc::now());
                        chat.last_seen_at = chat.last_message_at;
                        chat
                    })
                    .to_vec();
                state.chats_synced = true;
                state.spaces_synced = true;
                state.apply_sessions(vec![working("c")]);
                state.select_chat(Some("a".into()), cx);
            });
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(shell.sidebar_visible_order(cx).len(), 6);
            shell.settings.open_tabs = Some(ids.map(String::from).to_vec());
            shell.touch_session_mru("a");
            shell.settings.switcher_limit = 2;
            shell.switcher_step(true, cx);
            let order = &shell.switcher.as_ref().unwrap().order;
            assert_eq!(order[0], "a");
            assert!(order.contains(&"c".to_string()));
            assert!(order.len() <= 3, "{order:?}");
        });
    }

    #[gpui::test]
    fn switcher_opens_on_the_previous_session_and_commits_on_release(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, cx| {
                state.chats = vec![chat("a"), chat("b"), chat("c")];
                state.chats_synced = true;
                state.spaces_synced = true;
                state.apply_sessions(vec![working("a"), working("b"), working("c")]);
                state.select_chat(Some("a".into()), cx);
            });
            shell.on_state_changed(&shell.state.clone(), cx);
            for id in ["c", "b"] {
                shell.open_chat(id.into(), cx);
                shell.on_state_changed(&shell.state.clone(), cx);
            }
            assert_eq!(shell.session_mru, ["b", "c", "a"]);

            // First press: the overlay opens with the previous session
            // highlighted, cards in most-recently-used order.
            shell.switcher_step(true, cx);
            let sw = shell.switcher.as_ref().expect("switcher open");
            assert_eq!(sw.order, ["b", "c", "a"]);
            assert_eq!(sw.hi, 1);
            // Second press moves on; a back press returns.
            shell.switcher_step(true, cx);
            assert_eq!(shell.switcher.as_ref().unwrap().hi, 2);
            shell.switcher_step(false, cx);
            assert_eq!(shell.switcher.as_ref().unwrap().hi, 1);

            // Letting go of every modifier lands on the highlight.
            shell.on_modifiers_changed(
                &gpui::ModifiersChangedEvent {
                    modifiers: gpui::Modifiers::default(),
                    capslock: gpui::Capslock::default(),
                },
                cx,
            );
            assert!(shell.switcher.is_none());
            assert_eq!(shell.state.read(cx).selected_chat.as_deref(), Some("c"));

            // Escape leaves the selection alone.
            shell.switcher_step(true, cx);
            shell.switcher_close(cx);
            assert!(shell.switcher.is_none());
            assert_eq!(shell.state.read(cx).selected_chat.as_deref(), Some("c"));
        });
    }

    #[gpui::test]
    fn switcher_offers_the_sidebar_within_the_recency_window(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        shell.update(cx, |shell, cx| {
            // Project-less, already-seen rows: the sidebar lists them without
            // a space row and the strip has no reason to show them.
            let loose = |id: &str, hours_ago: i64| {
                let mut chat = chat(id);
                chat.space_id = None;
                chat.last_message_at = Some(Utc::now() - chrono::Duration::hours(hours_ago));
                chat.last_seen_at = chat.last_message_at;
                chat
            };
            shell.state.update(cx, |state, cx| {
                state.chats = vec![loose("a", 0), loose("stale", 9), loose("fresh", 1)];
                state.chats_synced = true;
                state.spaces_synced = true;
                // Only "a" runs: the upgrade pass drops the other two idle
                // sidebar rows from the strip.
                state.apply_sessions(vec![working("a")]);
                state.select_chat(Some("a".into()), cx);
            });
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(shell.open_tab_ids(cx), ["a"]);

            // Default window: every sidebar row is a card.
            shell.switcher_step(true, cx);
            let mut order = shell.switcher.as_ref().unwrap().order.clone();
            order.sort();
            assert_eq!(order, ["a", "fresh", "stale"]);
            shell.switcher_close(cx);

            // A cap of 2 keeps the MRU head (the selected session) and one more.
            shell.settings.switcher_limit = 2;
            shell.switcher_step(true, cx);
            assert_eq!(shell.switcher.as_ref().unwrap().order.len(), 2);
            assert_eq!(shell.switcher.as_ref().unwrap().order[0], "a");
            shell.switcher_close(cx);
            shell.settings.switcher_limit = 0;

            // An 8h window drops the 9h-old row; the selected session stays.
            shell.settings.switcher_window = settings::SwitcherWindow::Hours8;
            shell.switcher_step(true, cx);
            let mut order = shell.switcher.as_ref().unwrap().order.clone();
            order.sort();
            assert_eq!(order, ["a", "fresh"]);
            shell.switcher_close(cx);

            // The selected session is offered even when it is older than the
            // window, because it is on the strip.
            shell.state.update(cx, |state, _| {
                state.chats[0].last_message_at = Some(Utc::now() - chrono::Duration::hours(20));
            });
            shell.on_state_changed(&shell.state.clone(), cx);
            shell.switcher_step(true, cx);
            let mut order = shell.switcher.as_ref().unwrap().order.clone();
            order.sort();
            assert_eq!(order, ["a", "fresh"]);

            // Committing onto a sidebar-only row selects it and puts it on
            // the strip, exactly like clicking its sidebar row.
            let sw = shell.switcher.as_ref().unwrap();
            assert_eq!(sw.order[sw.hi], "fresh");
            shell.on_modifiers_changed(
                &gpui::ModifiersChangedEvent {
                    modifiers: gpui::Modifiers::default(),
                    capslock: gpui::Capslock::default(),
                },
                cx,
            );
            assert!(shell.switcher.is_none());
            assert_eq!(shell.state.read(cx).selected_chat.as_deref(), Some("fresh"));
            shell.on_state_changed(&shell.state.clone(), cx);
            assert!(shell.open_tab_ids(cx).contains(&"fresh".to_string()));
        });
    }
}

#[cfg(test)]
mod close_tests {
    use super::next_after_close;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn close_selects_next_then_previous_then_canvas() {
        let order = ids(&["a", "b", "c"]);
        assert_eq!(next_after_close(&order, "a").as_deref(), Some("b"));
        assert_eq!(next_after_close(&order, "b").as_deref(), Some("c"));
        // Last tab: fall back to the previous one.
        assert_eq!(next_after_close(&order, "c").as_deref(), Some("b"));
        // Only tab: canvas.
        assert_eq!(next_after_close(&ids(&["solo"]), "solo"), None);
        // Unknown id: no opinion.
        assert_eq!(next_after_close(&order, "zz"), None);
    }
}
