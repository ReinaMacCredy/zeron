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
/// Width of one fixed trailing titlebar button (summary, right-pane toggle).
const TITLEBAR_FIXED_BUTTON: f32 = 28.0;
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
pub(super) struct TabDragPayload {
    from: usize,
    pub(super) chat_id: String,
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

/// The split's double chip: both sessions in one selected pill, left half
/// for the left pane. A half clicks, drags (into the drop zones or along
/// the strip) and middle-closes like a tab; the pane header carries the
/// split's own close.
fn split_tab_chip(
    ix: usize,
    halves: [(
        String,
        SharedString,
        Option<zeron_proto::HarnessId>,
        ChatIndicator,
    ); 2],
    selected: Option<&str>,
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
        let brand = harness.map(crate::pickers::harness_brand_icon);
        let dot = spaces::status_dot_color(status, theme);
        let select_id = id.clone();
        let middle_id = id.clone();
        let menu_id = id.clone();
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
                .pr(px(6.0))
                .rounded(px(8.0))
                .text_color(if is_primary {
                    theme.text
                } else if active {
                    theme.text.opacity(0.7)
                } else {
                    theme.text_muted.opacity(0.6)
                })
                .cursor_pointer()
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
                .when(crate::click_activation_drag_enabled(), |el| {
                    el.on_drag(
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
                })
                .when_some(brand, |el, (path, tint)| {
                    el.child(
                        icon(path)
                            .size(px(14.0))
                            .flex_none()
                            .text_color(tint.unwrap_or(theme.text_muted).opacity(0.9)),
                    )
                })
                .child(div().flex_1().min_w_0().truncate().child(title))
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
                ),
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

impl Shell {
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
            state
                .overview_chats(Utc::now())
                .first()
                .map(|(_, c)| c.id.clone())
        };
        if let Some(first) = first {
            self.focus_composer(cx);
            self.state
                .update(cx, |s, cx| s.select_chat(Some(first), cx));
        }
    }

    /// Open a session from the sidebar: select it, the main area follows.
    /// The session being left is snapshotted first for the switcher.
    pub(crate) fn open_chat(&mut self, chat_id: String, cx: &mut Context<Self>) {
        if self.state.read(cx).selected_chat.as_deref() != Some(chat_id.as_str()) {
            self.snapshot_current_chat(cx);
        }
        self.select_existing_chat(chat_id, cx);
    }

    pub(super) fn select_existing_chat(&mut self, chat_id: String, cx: &mut Context<Self>) {
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
    /// re-homes the canvas onto that project; under "All" the current pick
    /// (the last selected project, restored from composer defaults) stands.
    pub(super) fn open_new_session(&mut self, cx: &mut Context<Self>) {
        self.snapshot_current_chat(cx);
        self.command_palette = None;
        self.route = Route::Chat;
        self.focus_composer(cx);
        let target = {
            let state = self.state.read(cx);
            self.settings
                .space_filter
                .clone()
                .filter(|id| state.space_row(id).is_some())
        };
        let defaults = crate::settings::composer::ComposerDefaults::load(&self.data_dir);
        self.state.update(cx, |s, cx| {
            // The draft comes first so the target picks below record onto
            // IT, never onto an invested draft being left behind (d20).
            s.select_chat(None, cx);
            s.new_draft(cx);
            if target.is_some() {
                s.select_space(target, cx);
            } else if defaults.no_project {
                // Opening an existing project session (including boot's last
                // session) must not erase the saved new-session opt-out.
                s.select_space(None, cx);
                if let Some(device) = defaults.device {
                    s.select_device(device, cx);
                }
            }
        });
        self.freeze_draft_row(cx);
        cx.notify();
    }

    /// The open tabs in VISUAL order: `open_tabs` minus ids the doc no longer
    /// backs with a live (non-archived) chat, and minus idle sessions — the
    /// strip is an activity strip (user request): a tab shows while its
    /// session is working, waiting on input, or finished-but-unseen, plus the
    /// selected one. Idle tabs stay in the stored vec and resurface when the
    /// session runs again. The stored vec is pruned separately by
    /// [`Shell::sync_open_tabs`]; this read-only view just skips rows that
    /// haven't been reconciled yet.
    pub(super) fn open_tab_ids(&self, cx: &App) -> Vec<String> {
        let Some(open) = self.settings.open_tabs.as_ref() else {
            return Vec::new();
        };
        let state = self.state.read(cx);
        let now = Utc::now();
        let pair = self.split_pair.as_ref();
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
        open.iter()
            .filter(|id| {
                // A draft is always a place the user is working in (d18); a
                // session started in the background shows from its send on.
                drafts.contains(id.as_str())
                    || state.send_pending(id, now)
                    || chats.get(id.as_str()).is_some_and(|c| {
                        !c.archived
                            && (state.selected_chat.as_deref() == Some(c.id.as_str())
                                || pair.as_ref().is_some_and(|p| p.contains(&c.id))
                                || crate::state::display_status(
                                    c,
                                    sessions.get(c.id.as_str()).copied(),
                                    now,
                                ) != ChatIndicator::Idle)
                    })
            })
            .cloned()
            .collect()
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
            // First run with tabs (or a pre-tabs settings file): every
            // session the sidebar is showing opens as a tab, so the strip
            // doesn't start out as the single boot-selected session. The
            // sidebar list is space-gated, so wait for the spaces frame too —
            // chats alone seeded a single tab (the boot selection).
            if !self.state.read(cx).spaces_synced {
                return;
            }
            self.settings.open_tabs = Some(self.sidebar_visible_order(cx));
            changed = true;
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
        for pinned in selected.iter().chain(pair.iter()) {
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
    /// the hidden (idle) ids kept after it so they resurface later
    /// (device-local; unreconciled ids drop out, which the prune would do
    /// anyway).
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
            Option<zeron_proto::HarnessId>,
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
                            None,
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
                        chat.config.as_ref().map(|c| c.harness),
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
            (SharedString, Option<zeron_proto::HarnessId>, ChatIndicator),
        > = tabs
            .iter()
            .map(|(id, title, harness, status, _)| (id.clone(), (title.clone(), *harness, *status)))
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
                    let (text_color, bg) = if is_selected {
                        (theme.text, crate::theme::glass_selected_bg())
                    } else if is_hovered {
                        (theme.text_muted.opacity(0.8), theme.glass_hover())
                    } else {
                        (theme.text_muted.opacity(0.6), crate::theme::wash(0.0))
                    };
                    let glyph_alpha = if is_selected { 0.9 } else { 0.6 };
                    let brand = harness.map(crate::pickers::harness_brand_icon);
                    let select_id = id.clone();
                    let close_id = id.clone();
                    let middle_id = id.clone();
                    let menu_id = id.clone();
                    let hover_id = id.clone();
                    // NB: no `.occlude()` on the close button — the TAB already
                    // occludes (for the titlebar drag region), and an occluding
                    // child would block the tab's own hover hit-test: a flicker
                    // loop (user-reported). `stop_propagation` on click is enough.
                    let trailing: AnyElement = if is_hovered {
                        div()
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
                            .into_any_element()
                    } else {
                        // Working animates (the sidebar's miniaturized glyph
                        // spinner) instead of a static dot; every other
                        // non-idle status stays a dot.
                        let dot = spaces::status_dot_color(status, &theme);
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
                    };
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
                        .when(crate::click_activation_drag_enabled(), |el| {
                            el.on_drag(
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
                        })
                        .when_some(brand, |el, (path, tint)| {
                            el.child(
                                icon(path).size(px(14.0)).flex_none().text_color(
                                    tint.unwrap_or(theme.text_muted).opacity(glyph_alpha),
                                ),
                            )
                        })
                        // A draft wears the amber pen where a session wears
                        // its harness brand (d18).
                        .when(is_draft, |el| {
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
        // one-frame lag is invisible). On GLASS the fades are an EdgeFade
        // scope (per-glyph gradient); painted overlays only exist on opaque
        // platforms, in the SHELL surface tone the strip sits on.
        let scrolled = -f32::from(self.tabs_scroll.offset().x);
        let max_scroll = f32::from(self.tabs_scroll.max_offset().x);
        let fade_left = scrolled > 1.0;
        let fade_right = scrolled < max_scroll - 1.0;
        let glass = theme.is_glass();
        let bar_bg = theme.surface;
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
            )
            .when(fade_left && !glass, |el| {
                el.child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .w(px(FADE_WIDTH))
                        .bg(gpui::linear_gradient(
                            90.0,
                            gpui::linear_color_stop(bar_bg, 0.0),
                            gpui::linear_color_stop(bar_bg.opacity(0.0), 1.0),
                        )),
                )
            })
            .when(fade_right && !glass, |el| {
                el.child(
                    div()
                        .absolute()
                        .right_0()
                        .top_0()
                        .bottom_0()
                        .w(px(FADE_WIDTH))
                        .bg(gpui::linear_gradient(
                            270.0,
                            gpui::linear_color_stop(bar_bg, 0.0),
                            gpui::linear_color_stop(bar_bg.opacity(0.0), 1.0),
                        )),
                )
            });
        if glass {
            crate::edge_fade::edge_faded(FADE_WIDTH, false, false, tab_region)
                .fade_left(fade_left)
                .fade_right(fade_right)
                .into_any_element()
        } else {
            tab_region.into_any_element()
        }
    }

    /// The unified titlebar in chat mode:
    /// `[new-session +] [session tabs] … [toggle-changes]`.
    /// Carries the titlebar duties (drag region, animated left inset, the
    /// right-pane controls on git projects).
    pub(super) fn render_session_title_bar(&mut self, cx: &mut Context<Self>) -> AnyElement {
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
        // click. Closed, it is just the stable open/close toggle. Hidden on
        // the new-session canvas (user request) — nothing to diff yet.
        let takeover = !on_canvas && self.right_pane_open(cx) && self.right_pane_expanded;
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
        let trailing: Option<gpui::AnyElement> = if on_canvas {
            None
        } else {
            let right_open = self.right_pane_open(cx);
            let mut controls = div()
                .id("right-titlebar-controls")
                .flex_none()
                .h_full()
                .flex()
                .flex_row()
                .items_center();
            if right_open {
                let right_now = self.eval_tween(self.right_tween, self.right_target(cx));
                let pr = self.titlebar_right_pad(TITLEBAR_ACTION_EDGE_INSET);
                // The row's own left padding is part of its content box: a strip
                // wider than what's left after it overflows and clips at the right
                // edge (flex_none never shrinks) — cap to the available width. The
                // row's 8px child gaps sit OUTSIDE the strip's width (one before
                // the strip in takeover, two with the tabs row present): without
                // budgeting them the capped strip overflows by exactly one gap and
                // the buttons slide right on expand (user report).
                let gap_budget = if takeover { 8.0 } else { 16.0 };
                let avail = self.viewport_width - row_left - pr - gap_budget;
                // The right pane's SURFACE TABS (t3 RightPanelTabs) — the diff
                // options that used to live here moved into the pane's own
                // second row; expand stays in this band (user request).
                let tabs = self.render_right_tab_strip(cx);
                // The summary + toggle pair is the fixed right-edge anchor,
                // like the left sidebar control. Only the tabs + expand
                // section reveals to its left; including the toggle in this
                // animated width compressed both icons into the same clipped
                // box at open.
                let animated_width =
                    ((right_now - pr).min(avail) - 2.0 * TITLEBAR_FIXED_BUTTON).max(0.0);
                controls = controls.child(
                    div()
                        .w(px(animated_width))
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
                            &theme,
                            cx.listener(|this, _, _, cx| this.toggle_right_pane_expand(cx)),
                        )),
                );
            }
            // Keep the trigger mounted at one fixed position while the pane
            // controls reveal to its left.
            Some(
                controls
                    .child(self.render_summary_trigger(cx))
                    .child(header_icon_button(
                        "toggle-changes",
                        icons::SIDEBAR_MINIMALISTIC,
                        &theme,
                        cx.listener(|this, _, _, cx| this.toggle_right_pane(cx)),
                    ))
                    .into_any_element(),
            )
        };

        // In panel takeover the header strip spans the whole band — the tabs
        // would sit UNDER it (both flex_none, the row overflows and paint
        // order stacks them), so they hide for the duration.
        let session_tabs = (!takeover).then(|| self.render_session_tabs(cx));

        let inner = div()
            .size_full()
            .flex()
            .items_center()
            .pt(px(Theme::TITLEBAR_TOP_PAD))
            .gap(px(8.0))
            .pl(px(row_left))
            .pr(px(self.titlebar_right_pad(TITLEBAR_ACTION_EDGE_INSET)))
            .children(session_tabs)
            .child(div().flex_1())
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
        Some(popover::menu_at(
            "tab-context-menu",
            position,
            menu.into_any_element(),
            closing,
        ))
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
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.shell.update(cx, |shell, cx| {
                div()
                    .w(px(900.))
                    .h(px(60.))
                    .child(shell.render_session_title_bar(cx))
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
            shell.open_new_session(cx);
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(
                shell.state.read(cx).active_draft.as_deref(),
                Some(first.as_str())
            );
            assert_eq!(shell.open_tab_ids(cx), [first.clone()]);
            // ⌘N on an invested draft opens a second one beside it.
            invest(&first, "keep me", cx);
            shell.open_new_session(cx);
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
    fn bulk_close_preserves_hidden_idle_tabs(cx: &mut TestAppContext) {
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
                (false, false, vec!["idle", "anchor"]),
                (true, false, vec!["left", "idle", "anchor"]),
                (false, true, vec!["idle", "anchor", "right"]),
            ] {
                shell.settings.open_tabs = Some(
                    ["left", "idle", "anchor", "right"]
                        .map(String::from)
                        .to_vec(),
                );
                shell.close_tabs_around("anchor", keep_left, keep_right, cx);
                assert_eq!(shell.settings.open_tabs.as_ref().unwrap(), &expected);
            }
            shell.state.update(cx, |state, _| {
                state.apply_sessions(vec![working("idle"), working("anchor"), working("right")]);
            });
            assert_eq!(shell.open_tab_ids(cx), ["idle", "anchor", "right"]);
        });
    }

    #[gpui::test]
    fn idle_tabs_hide_until_selected_or_active_again(cx: &mut TestAppContext) {
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
            // The selected tab always shows, whatever its status.
            shell.open_chat("quiet".into(), cx);
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(shell.open_tab_ids(cx), ["busy", "quiet"]);
            // Deselected and idle: the tab hides but stays stored.
            shell.open_chat("busy".into(), cx);
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(shell.open_tab_ids(cx), ["busy"]);
            assert_eq!(
                shell.settings.open_tabs.as_deref(),
                Some(&["busy".to_string(), "quiet".to_string()][..])
            );
        });
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(cx.debug_bounds("session-tab-busy").is_some());
        assert!(cx.debug_bounds("session-tab-quiet").is_none());

        shell.update(cx, |shell, cx| {
            // The session runs again: the tab resurfaces in its stored slot.
            shell.state.update(cx, |state, _| {
                state.apply_sessions(vec![working("busy"), working("quiet")]);
            });
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(shell.open_tab_ids(cx), ["busy", "quiet"]);
        });
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(cx.debug_bounds("session-tab-quiet").is_some());
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
                // Only "a" runs: the other two are idle sidebar rows with no
                // tab on the strip.
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
