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
//! Each tab: agent brand icon (the activity glyph while it runs) + title + a
//! trailing slot that shows the status dot at rest and a close button on the
//! selected or hovered tab; right-click opens the
//! close/close-others/close-right/close-left menu. Tabs shrink evenly from
//! 216 to 144px, then the strip scrolls horizontally with edge fades. Drag-
//! reorder mirrors the right pane's surface strip (`render_right_tab_strip`):
//! drop-index math, 150ms sibling slide, and drag ghost.

use super::terminal_tabs::{is_terminal_tab, keep_terminal_tab};
use super::*;
use crate::provider_instances::InstanceIcon;
use crate::terminal::panel::{drop_index, reorder_tabs, slide_offset};
use crate::theme::InsetTones;
use zeron_proto::ChatIndicator;

const RIGHT_PANE_HEADER_CONTROLS_MIN_WIDTH: f32 = 8.0 + 4.0 + 4.0 + 28.0;

/// Synara's tab sizes (d122): tabs start at the max width and shrink evenly
/// to the min before the strip scrolls.
const TAB_MAX_WIDTH: f32 = 216.0;
const TAB_MIN_WIDTH: f32 = 144.0;
const TAB_GROUP_TITLE_MIN_WIDTH: f32 = 72.0;
const TAB_HEIGHT: f32 = 32.0;
/// Flex gap between tabs — part of the drop-index slot width.
const TAB_GAP: f32 = 4.0;
/// Width of the overflow edge fades. Wide enough that per-glyph fade steps
/// (title text fades glyph-by-glyph on glass) stay gentle.
const FADE_WIDTH: f32 = 36.0;
const FOLDER_FLARE: f32 = 8.0;
const FOLDER_DROP: f32 = (Theme::TITLEBAR_HEIGHT - TAB_HEIGHT) / 2.0;

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
    brand: Option<InstanceIcon>,
}

/// The floating tab rendered at the cursor while dragging (tab or sidebar row).
pub(super) struct TabGhost {
    pub(super) title: SharedString,
    pub(super) brand: Option<InstanceIcon>,
}

impl Render for TabGhost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        div()
            .w(px(TAB_MAX_WIDTH))
            .h(px(TAB_HEIGHT))
            .px(px(10.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .rounded(px(Theme::CONTROL_RADIUS))
            .bg(theme.surface_raised)
            .border_1()
            .border_color(theme.border_strong)
            .text_size(px(12.0))
            .text_color(theme.text)
            .opacity(0.85)
            .when_some(self.brand.as_ref(), |el, brand| {
                el.child(brand.svg(theme.text_muted).size(px(14.0)).flex_none())
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

pub(super) fn group_tab_order(
    order: &[String],
    members: &std::collections::HashMap<String, crate::tasks::TabMember>,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(order.len());
    for id in order {
        if out.contains(id) {
            continue;
        }
        let Some(member) = members.get(id) else {
            out.push(id.clone());
            continue;
        };
        let mut group: Vec<&String> = order
            .iter()
            .filter(|other| {
                members
                    .get(*other)
                    .is_some_and(|other| other.task_id == member.task_id)
            })
            .collect();
        group.sort_by_key(|other| members[*other].step.map_or(0, |step| step + 1));
        out.extend(group.into_iter().cloned());
    }
    out
}

fn strip_child_index<'a>(
    ids: impl Iterator<Item = &'a str>,
    selected: &str,
    members: &std::collections::HashMap<String, crate::tasks::TabMember>,
    split_right: Option<&str>,
) -> Option<usize> {
    let mut children = 0usize;
    let mut prev_task = None;
    for id in ids {
        let task = members.get(id).map(|member| member.task_id.as_str());
        let joins = Some(id) == split_right || (task.is_some() && task == prev_task);
        if !joins {
            children += 1;
        }
        if id == selected {
            return children.checked_sub(1);
        }
        prev_task = task;
    }
    None
}

/// The strip hugs its tabs until they shrink, then fills the row.
fn tab_width(strip_width: f32, slots: usize) -> f32 {
    let slots = slots.max(1) as f32;
    ((strip_width - TAB_GAP * (slots - 1.0)) / slots).clamp(TAB_MIN_WIDTH, TAB_MAX_WIDTH)
}

/// Sliding transform while a sibling is dragged over: animate 150ms between
/// committed offsets. `slots` is how many tab slots the element spans.
fn slide_tab<E: gpui::Element + gpui::Styled + 'static>(
    tab_el: E,
    ix: usize,
    id: String,
    slots: usize,
    width: f32,
    drag: Option<(usize, usize, usize, usize)>,
) -> AnyElement {
    match drag {
        Some((from, over, epoch, prev_over)) if ix != from => {
            let slot = width + TAB_GAP;
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
            .w(px(width * slots as f32 + TAB_GAP * (slots - 1) as f32))
            .h(px(TAB_HEIGHT))
            .flex_none()
            .into_any_element(),
        _ => tab_el.into_any_element(),
    }
}

/// A tab's trailing slot: the close button on the selected or hovered tab,
/// else the status dot. Fixed at 24px so the title never reflows on hover.
///
/// NB: no `.occlude()` on the close button — the TAB already occludes (for
/// the titlebar drag region), and an occluding child would block the tab's
/// own hover hit-test: a flicker loop (user-reported). `stop_propagation`
/// on click is enough.
fn tab_trailing(
    id: &str,
    title: &SharedString,
    status: ChatIndicator,
    show_close: bool,
    theme: &Theme,
    cx: &mut Context<Shell>,
) -> AnyElement {
    if show_close {
        let close_id = id.to_string();
        return div()
            .id(SharedString::from(format!("session-tab-close-{id}")))
            .role(gpui::Role::Button)
            .aria_label(SharedString::from(format!("Close {title}")))
            .size(px(24.0))
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
                    .size(px(16.0))
                    .text_color(theme.text_muted),
            )
            .into_any_element();
    }
    let dot = spaces::status_dot_color(status, theme);
    div()
        .size(px(24.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .when(
            !matches!(status, ChatIndicator::Idle | ChatIndicator::Working),
            |el| el.child(div().size(px(6.0)).rounded_full().bg(dot)),
        )
        .into_any_element()
}

fn tab_spinner(id: &str, theme: &Theme, cx: &mut Context<Shell>) -> AnyElement {
    let selector = format!("session-tab-working-{id}");
    div()
        .debug_selector(move || selector)
        .size(px(14.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .child(loaders::mini_glyph_spinner(
            format!("tab-working-{id}"),
            3.0,
            theme.glyph,
            cx.entity_id(),
            cx,
        ))
        .into_any_element()
}

fn folder_outline(path: &mut gpui::PathBuilder, bounds: gpui::Bounds<Pixels>) {
    let at = |x: f32, y: f32| gpui::point(bounds.origin.x + px(x), bounds.origin.y + px(y));
    let w = f32::from(bounds.size.width);
    let j = f32::from(bounds.size.height) - 1.0;
    let (f, r) = (FOLDER_FLARE, 8.0);
    path.curve_to(at(f, j - f), at(f, j));
    path.line_to(at(f, r));
    path.curve_to(at(f + r, 0.0), at(f, 0.0));
    path.line_to(at(w - f - r, 0.0));
    path.curve_to(at(w - f, r), at(w - f, 0.0));
    path.line_to(at(w - f, j - f));
    path.curve_to(at(w, j), at(w - f, j));
}

fn folder_fill(bounds: gpui::Bounds<Pixels>) -> Option<gpui::Path<Pixels>> {
    let mut fill = gpui::PathBuilder::fill();
    fill.move_to(bounds.bottom_left());
    fill.line_to(gpui::point(bounds.left(), bounds.bottom() - px(1.0)));
    folder_outline(&mut fill, bounds);
    fill.line_to(bounds.bottom_right());
    fill.close();
    fill.build().ok()
}

fn folder_wedges(bounds: gpui::Bounds<Pixels>) -> [Option<gpui::Path<Pixels>>; 2] {
    let at = |x: f32, y: f32| gpui::point(bounds.origin.x + px(x), bounds.origin.y + px(y));
    let w = f32::from(bounds.size.width);
    let j = f32::from(bounds.size.height) - 1.0;
    let (f, r) = (FOLDER_FLARE, 8.0);
    let mut left = gpui::PathBuilder::fill();
    left.move_to(at(0.0, 0.0));
    left.line_to(at(0.0, j));
    left.curve_to(at(f, j - f), at(f, j));
    left.line_to(at(f, r));
    left.curve_to(at(f + r, 0.0), at(f, 0.0));
    left.close();
    let mut right = gpui::PathBuilder::fill();
    right.move_to(at(w, 0.0));
    right.line_to(at(w - f - r, 0.0));
    right.curve_to(at(w - f, r), at(w - f, 0.0));
    right.line_to(at(w - f, j - f));
    right.curve_to(at(w, j), at(w - f, j));
    right.close();
    [left.build().ok(), right.build().ok()]
}

fn folder_tab_backdrop(
    tones: InsetTones,
    backdrop: Option<crate::chat_background::Backdrop>,
) -> AnyElement {
    div()
        .absolute()
        .top_0()
        .left(px(-FOLDER_FLARE))
        .right(px(-FOLDER_FLARE))
        .h(px(TAB_HEIGHT + FOLDER_DROP + 1.0))
        .child(
            gpui::canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    if let Some(path) = folder_fill(bounds) {
                        window.paint_path(path, tones.body);
                    }
                },
            )
            .size_full(),
        )
        .when_some(backdrop, |el, backdrop| {
            let (shade, alpha) = (backdrop.scrim_color, backdrop.settings.tab_shade);
            el.child(crate::chat_background::surface(
                backdrop,
                move |own, layer, window| {
                    for wedge in folder_wedges(own).into_iter().flatten() {
                        window.paint_path(wedge, shade.opacity(alpha));
                    }
                    if let Some(path) = folder_fill(own) {
                        window.paint_path(
                            path,
                            crate::chat_background::tab_fade(
                                shade,
                                alpha,
                                f32::from(own.top() - layer.top()),
                                f32::from(own.bottom() - layer.top()),
                            ),
                        );
                    }
                },
            ))
        })
        .child(
            gpui::canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let mut edge = gpui::PathBuilder::stroke(px(0.5));
                    edge.move_to(gpui::point(bounds.left(), bounds.bottom() - px(1.0)));
                    folder_outline(&mut edge, bounds);
                    if let Ok(path) = edge.build() {
                        window.paint_path(path, tones.edge);
                    }
                },
            )
            .absolute()
            .inset_0(),
        )
        .into_any_element()
}

fn tab_grip(theme: &Theme) -> AnyElement {
    icon(icons::DRAG_HANDLE)
        .size(px(14.0))
        .flex_none()
        .text_color(theme.text_muted)
        .into_any_element()
}

/// The split's double chip: both sessions in one selected pill, left half
/// for the left pane. A half clicks, hovers a close, drags (into the drop
/// zones or along the strip) and middle-closes like a tab; the pane header
/// carries the split's own close.
fn split_tab_chip(
    ix: usize,
    halves: [(String, SharedString, Vec<InstanceIcon>, ChatIndicator); 2],
    selected: Option<&str>,
    hovered: Option<&str>,
    tones: Option<InsetTones>,
    backdrop: Option<crate::chat_background::Backdrop>,
    theme: &Theme,
    cx: &mut Context<Shell>,
) -> gpui::Stateful<gpui::Div> {
    // The chip is "selected" while either member is on screen; another
    // session selected shows full width and the pair waits here.
    let active = halves.iter().any(|(id, ..)| selected == Some(id.as_str()));
    let mut chip = div()
        .id("session-tab-split")
        .w(px(TAB_MAX_WIDTH * 2.0 + TAB_GAP))
        .min_w(px(TAB_MIN_WIDTH * 2.0 + TAB_GAP))
        .h(px(TAB_HEIGHT))
        .flex()
        .flex_row()
        .items_center()
        .rounded(px(8.0))
        .text_size(px(12.0))
        .when(active, |el| match tones {
            Some(tones) => el.relative().child(folder_tab_backdrop(tones, backdrop)),
            None => el
                .bg(crate::theme::glass_selected_bg())
                .shadow(crate::theme::glass_selected_shadows()),
        })
        .when(!active, |el| {
            let wash = tones.map_or_else(|| theme.glass_hover(), |tones| tones.wash);
            el.hover(move |s| s.bg(wash))
        })
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
        let half_hovered = hovered == Some(id.as_str());
        let brand = harness.last().cloned();
        let trailing = tab_trailing(&id, &title, status, is_primary || half_hovered, theme, cx);
        let select_id = id.clone();
        let middle_id = id.clone();
        let menu_id = id.clone();
        let hover_id = id.clone();
        chip = chip.child(
            div()
                .id(SharedString::from(format!("session-tab-{id}")))
                .debug_selector(|| format!("session-tab-{id}"))
                .role(gpui::Role::Tab)
                .aria_label(title.clone())
                .aria_selected(is_primary)
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .pl(px(10.0))
                .pr(px(4.0))
                .rounded(px(8.0))
                .text_color(if is_primary || half_hovered {
                    theme.text
                } else if active {
                    theme.text.opacity(0.7)
                } else {
                    theme.text_muted
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
                        let brand = payload.brand.clone();
                        cx.stop_propagation();
                        cx.new(|_| TabGhost { title, brand })
                    },
                )
                .map(|el| {
                    match crate::computer::presence::chat_glyph(
                        "session-tab",
                        &id,
                        14.0,
                        0.9,
                        theme,
                        cx,
                    ) {
                        Some(glyph) => el.child(glyph),
                        None if status == ChatIndicator::Working => {
                            el.child(tab_spinner(&id, theme, cx))
                        }
                        None if half_hovered && !harness.is_empty() => el.child(tab_grip(theme)),
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

pub(super) struct TitlebarTrailing {
    fit: HorizontalPanelFit,
    right_pane_visible: bool,
    takeover: bool,
    row_left: f32,
    row_gap: f32,
    right_pad: f32,
    widths: PanelTitlebarWidths,
    content_width: f32,
}

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

/// A target the opener names for the new-session canvas, over the sidebar
/// filter and the canvas's own last pick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum NewSessionTarget {
    /// This project (space id).
    Project(String),
    /// No project, on this device (id): a "No project" sidebar group.
    NoProject(String),
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
            || self.new_tab_menu.get().is_some()
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
        self.chat_composer(cx).update(cx, |composer, cx| {
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
        // Opening any session steps the voice stage aside — including the one
        // already selected under it. The call keeps running in the background.
        self.set_voice_stage_open(false, cx);
        self.set_route(Route::Chat, cx);
        self.focus_composer(cx);
        self.nav.push(NavEntry::Chat(chat_id.clone()));
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
            Some(id) if is_terminal_tab(&id) => self.show_terminal_tab(id, cx),
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
        if matches!(self.route, Route::Chat)
            && self.state.read(cx).focused_id().as_deref() == Some(draft_id.as_str())
        {
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
    /// `target` (the `+` on a sidebar group header) homes the canvas on that
    /// project, or on no project on that device, and wins over the sidebar
    /// filter.
    pub(super) fn open_new_session(
        &mut self,
        target: Option<NewSessionTarget>,
        cx: &mut Context<Self>,
    ) {
        self.snapshot_current_chat(cx);
        self.command_palette = None;
        self.set_voice_stage_open(false, cx);
        self.set_route(Route::Chat, cx);
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
        let project = match &target {
            Some(NewSessionTarget::Project(project)) => Some(project.clone()),
            Some(NewSessionTarget::NoProject(_)) => None,
            None => self.settings.space_filter.clone(),
        }
        .filter(|id| self.state.read(cx).space_row(id).is_some());
        let defaults = crate::settings::composer::ComposerDefaults::load(&self.data_dir);
        self.state.update(cx, |s, cx| {
            // The draft comes first so the target picks below record onto
            // IT, never onto an invested draft being left behind (d20).
            // Leaving a chat puts the canvas's own target back; the filter
            // (an explicit standing choice) then applies on top of it.
            let restores = s.canvas_target.is_some();
            s.select_chat(None, cx);
            s.new_draft(cx);
            if let Some(NewSessionTarget::NoProject(device)) = target {
                // The composer's "Don't work in a project" pick, on that device.
                s.select_space(None, cx);
                s.select_device(device, cx);
            } else if project.is_some() {
                s.select_space(project, cx);
            } else if !restores && defaults.no_project {
                // No canvas target was set aside (e.g. after a runtime swap):
                // fall back to the saved new-session opt-out.
                s.select_space(None, cx);
                if let Some(device) = defaults.device {
                    s.select_device(device, cx);
                }
            }
        });
        let entry = self.focus_nav_entry(cx);
        self.record_nav_focus(entry, cx);
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
        let terminals = self.live_terminal_tab_ids(cx);
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
                terminals.contains(id.as_str())
                    || drafts.contains(id.as_str())
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
        let members = crate::tasks::tab_members(state);
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
                && (chat.delegated_by().is_none() || members.contains_key(&chat.id));
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
                            || is_terminal_tab(id)
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
        let terminals = self.live_terminal_tab_ids(cx);
        let terminals_synced = self.terminals_synced;
        let tabs = self.settings.open_tabs.get_or_insert_with(Vec::new);
        let before = tabs.len();
        tabs.retain(|id| {
            if is_terminal_tab(id) {
                return keep_terminal_tab(id, terminals_synced, &terminals);
            }
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
        if is_terminal_tab(&chat_id) {
            return self.close_terminal_tab(&chat_id, cx);
        }
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
        if matches!(self.route, Route::Chat) && focused.as_deref() == Some(chat_id.as_str()) {
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
        if matches!(self.route, Route::Terminal)
            && let Some(id) = self.active_terminal_tab.clone()
        {
            self.close_terminal_tab(&id, cx);
            return true;
        }
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
        let selected = self.selected_tab_id(cx);
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
        let tones = self.settings.inset_shell.then(|| theme.inset_tones());
        let now = Utc::now();
        // A drag that ended off-strip (no drop event) must not strand the
        // sibling slide offsets.
        if self.tab_drag.is_some() && !cx.has_active_drag() {
            self.tab_drag = None;
        }
        let mut order: Vec<String> = self.open_tab_ids(cx);
        let mut members = crate::tasks::tab_members(self.state.read(cx));
        if let Some(pair) = &self.split_pair {
            members.retain(|id, _| !pair.contains(id));
        }
        order = group_tab_order(&order, &members);
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
        let terminals: std::collections::HashMap<String, super::terminal_tabs::TerminalTabRow> =
            self.terminal_tab_rows(cx)
                .into_iter()
                .map(|row| (row.id.clone(), row))
                .collect();
        let tabs: Vec<(String, SharedString, Vec<InstanceIcon>, ChatIndicator, bool)> = {
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
                    if let Some(row) = terminals.get(id) {
                        return Some((
                            id.clone(),
                            row.title.clone(),
                            Vec::new(),
                            ChatIndicator::Idle,
                            false,
                        ));
                    }
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
                    let lead = members.get(id).is_some_and(|member| member.step.is_none());
                    Some((
                        chat.id.clone(),
                        if lead {
                            SharedString::from("Lead")
                        } else {
                            SharedString::from(transcript::single_line(
                                &chat.title.clone().unwrap_or_else(|| "New session".into()),
                            ))
                        },
                        crate::pickers::chat_marks(chat, cx),
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
        let selected = self.selected_tab_id(cx);
        let split = self.split_chat_id().map(str::to_string);
        // Keep the selected tab visible: on selection change, scroll it into
        // view (minimal movement — a new session's tab materializes at the far
        // right of an overflowing strip and would otherwise be stranded
        // off-screen).
        match selected.as_deref() {
            Some(sel) if self.tabs_scrolled_to.as_deref() != Some(sel) => {
                if let Some(ix) = strip_child_index(
                    tabs.iter().map(|(id, ..)| id.as_str()),
                    sel,
                    &members,
                    group.as_ref().map(|(_, right)| right.as_str()),
                ) {
                    self.tabs_scroll.scroll_to_item(ix);
                }
                self.tabs_scrolled_to = Some(sel.to_string());
            }
            Some(_) => {}
            None => self.tabs_scrolled_to = None,
        }
        let hovered = self.tab_hover.clone();
        let count = tabs.len();
        let strip_pad = if tones.is_some() { FOLDER_FLARE } else { 0.0 };
        let width = tab_width(
            f32::from(self.tabs_scroll.bounds().size.width) - strip_pad * 2.0,
            count,
        );
        let drag = self
            .tab_drag
            .as_ref()
            .map(|d| (d.from, d.over, d.epoch, d.prev_over));

        let tabs_by_id: std::collections::HashMap<
            String,
            (SharedString, Vec<InstanceIcon>, ChatIndicator),
        > = tabs
            .iter()
            .map(|(id, title, harness, status, _)| (id.clone(), (title.clone(), harness.clone(), *status)))
            .collect();
        let tab_elements: Vec<(String, AnyElement)> = tabs
            .into_iter()
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
                            tones,
                            self.chat_backdrop.clone(),
                            &theme,
                            cx,
                        );
                        return Some((id.clone(), slide_tab(chip, ix, id.clone(), 2, width, drag)));
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
                let terminal = terminals.get(&id).map(|row| row.launch.brand());
                let (text_color, bg) = match (is_selected, is_hovered, tones) {
                    (true, _, Some(_)) => (theme.text, crate::theme::wash(0.0)),
                    (true, _, None) => (theme.text, crate::theme::glass_selected_bg()),
                    (false, true, Some(tones)) => (theme.text, tones.wash),
                    (false, true, None) => (theme.text, theme.glass_hover()),
                    (false, false, _) => (theme.text_muted, crate::theme::wash(0.0)),
                };
                let glyph_alpha = 0.9;
                let brand = terminal
                    .map(|(path, tint)| InstanceIcon::Brand(path, tint))
                    .or_else(|| harness.last().cloned());
                let select_id = id.clone();
                let middle_id = id.clone();
                let menu_id = id.clone();
                let hover_id = id.clone();
                let trailing =
                    tab_trailing(&id, &title, status, is_selected || is_hovered, &theme, cx);
                let grouped = members.contains_key(&id);
                let tab_el = div()
                    .id(SharedString::from(format!("session-tab-{id}")))
                    .debug_selector(|| format!("session-tab-{id}"))
                    .role(gpui::Role::Tab)
                    .aria_label(title.clone())
                    .aria_selected(is_selected)
                    .w(px(TAB_MAX_WIDTH))
                    .min_w(px(TAB_MIN_WIDTH))
                    .h(px(TAB_HEIGHT))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .pl(px(10.0))
                    .pr(px(4.0))
                    .rounded(px(8.0))
                    .text_size(px(12.0))
                    .text_color(text_color)
                    .bg(bg)
                    .when(is_selected, |el| match tones {
                        Some(tones) => el
                            .relative()
                            .child(folder_tab_backdrop(tones, self.chat_backdrop.clone())),
                        None => el.shadow(crate::theme::glass_selected_shadows()),
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
                    .when(!grouped, |el| {
                        el.on_drag(
                            TabDragPayload {
                                from: ix,
                                chat_id: id.clone(),
                                title: title.clone(),
                                brand,
                            },
                            |payload, _point, _, cx| {
                                let title = payload.title.clone();
                                let brand = payload.brand.clone();
                                cx.stop_propagation();
                                cx.new(|_| TabGhost { title, brand })
                            },
                        )
                    })
                    .when_some(terminal.filter(|_| !is_hovered), |el, (path, tint)| {
                        el.child(
                            icon(path)
                                .size(px(14.0))
                                .flex_none()
                                .text_color(tint.unwrap_or(theme.text_muted).opacity(glyph_alpha)),
                        )
                    })
                    .when(terminal.is_some() && is_hovered, |el| {
                        el.child(tab_grip(&theme))
                    })
                    .when(terminal.is_none(), |el| {
                        el.map(|el| {
                            match crate::computer::presence::chat_glyph(
                                "session-tab",
                                &id,
                                14.0,
                                glyph_alpha,
                                &theme,
                                cx,
                            ) {
                                Some(glyph) => el.child(glyph),
                                None if status == ChatIndicator::Working => {
                                    el.child(tab_spinner(&id, &theme, cx))
                                }
                                None if is_hovered && !harness.is_empty() => {
                                    el.child(tab_grip(&theme))
                                }
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
                    })
                    // A draft wears the amber pen where a session wears
                    // its harness brand (d18); a session with unsent
                    // composer text wears both, like its sidebar row.
                    .when(
                        terminal.is_none()
                            && (is_draft || crate::draft_store::has_content(&id, cx)),
                        |el| {
                            el.child(
                                icon(icons::PEN)
                                    .size(px(13.0))
                                    .flex_none()
                                    .text_color(theme.warning.opacity(glyph_alpha)),
                            )
                        },
                    )
                    .child(div().flex_1().min_w_0().truncate().child(title))
                    .child(trailing);

                if grouped {
                    return Some((id, tab_el.into_any_element()));
                }
                Some((id.clone(), slide_tab(tab_el, ix, id, 1, width, drag)))
            })
            .collect();
        let tab_elements = self.assemble_tab_groups(
            tab_elements,
            &members,
            &tabs_by_id,
            selected.as_deref(),
            width,
            cx,
        );

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
        let folder = tones.is_some();
        let tab_region = div()
            .relative()
            .min_w_0()
            .when(folder, |el| {
                el.self_start().h(px(Theme::TITLEBAR_HEIGHT + 1.0))
            })
            .child(
                div()
                    .id("session-tabs-scroll")
                    .role(gpui::Role::TabList)
                    .aria_label("Sessions")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(TAB_GAP))
                    .min_w_0()
                    .when(folder, |el| {
                        el.h_full()
                            .items_start()
                            .pt(px(FOLDER_DROP))
                            .px(px(FOLDER_FLARE))
                    })
                    .overflow_x_scroll()
                    .track_scroll(&self.tabs_scroll)
                    .on_scroll_wheel(crate::swipe::claim_scrollable(&self.tabs_scroll))
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
                                - strip_pad
                                - f32::from(scroll_for_drag.offset().x);
                            let over = drop_index(rel_x, width + TAB_GAP, count);
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

    fn toggle_tab_group(&mut self, task_id: &str, cx: &mut Context<Self>) {
        let tasks = &self.state.read(cx).tasks;
        let groups = &mut self.settings.collapsed_tab_groups;
        groups.retain(|id| tasks.iter().any(|task| &task.id == id && !task.deleted));
        match groups.iter().position(|id| id == task_id) {
            Some(ix) => {
                groups.remove(ix);
            }
            None => groups.push(task_id.to_owned()),
        }
        self.mark_onboarded(super::intros::MISSION_TABS, cx);
        self.schedule_save(cx);
        cx.notify();
    }

    fn close_tab_group_menu(&mut self, cx: &mut Context<Self>) {
        self.tab_group_hover = None;
        self.tab_group_menu_hover = None;
        cx.notify();
    }

    fn assemble_tab_groups(
        &mut self,
        elements: Vec<(String, AnyElement)>,
        members: &std::collections::HashMap<String, crate::tasks::TabMember>,
        tabs: &std::collections::HashMap<String, (SharedString, Vec<InstanceIcon>, ChatIndicator)>,
        selected: Option<&str>,
        width: f32,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut out = Vec::new();
        let mut elements = elements.into_iter().peekable();
        while let Some((id, element)) = elements.next() {
            let Some(member) = members.get(&id) else {
                out.push(element);
                continue;
            };
            let mut group = vec![(id, element)];
            while let Some(next) = elements.next_if(|(next, _)| {
                members
                    .get(next)
                    .is_some_and(|next| next.task_id == member.task_id)
            }) {
                group.push(next);
            }
            out.push(self.render_tab_group(member, group, tabs, selected, width, cx));
        }
        out
    }

    fn render_tab_group(
        &mut self,
        member: &crate::tasks::TabMember,
        group: Vec<(String, AnyElement)>,
        tabs: &std::collections::HashMap<String, (SharedString, Vec<InstanceIcon>, ChatIndicator)>,
        selected: Option<&str>,
        width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let tones = self.settings.inset_shell.then(|| theme.inset_tones());
        let task_id = member.task_id.clone();
        let title = SharedString::from(
            self.state
                .read(cx)
                .tasks
                .iter()
                .find(|task| task.id == task_id)
                .map(|task| transcript::single_line(&task.title))
                .unwrap_or_default(),
        );
        let collapsed = self.settings.collapsed_tab_groups.contains(&task_id);
        let on_page = matches!(self.route, Route::TaskCenter)
            && self.task_center_task.as_deref() == Some(member.lead.as_str());
        let shown = |id: &str| !collapsed || id == member.lead || selected == Some(id);
        let status_of = |id: &str| tabs.get(id).map(|(_, _, status)| *status);
        let hidden_asks = group
            .iter()
            .any(|(id, _)| !shown(id) && status_of(id) == Some(ChatIndicator::AwaitingInput));
        let menu_open = collapsed
            && [&self.tab_group_hover, &self.tab_group_menu_hover]
                .iter()
                .any(|hover| hover.as_deref() == Some(task_id.as_str()));
        let ids: Vec<String> = group.iter().map(|(id, _)| id.clone()).collect();
        let menu =
            menu_open.then(|| self.render_tab_group_menu(&task_id, &ids, tabs, selected, cx));
        let reduced = self.reduced_motion;
        let mut min_width = TAB_GROUP_TITLE_MIN_WIDTH;
        let members: Vec<AnyElement> = group
            .into_iter()
            .filter_map(|(id, element)| {
                let t = motion::state_t(
                    &format!("tab-group-member-{id}"),
                    shown(&id),
                    motion::TAB_GROUP,
                    reduced,
                );
                if t >= 1.0 {
                    min_width += TAB_MIN_WIDTH;
                    return Some(element);
                }
                (t > 0.0).then(|| {
                    min_width += width * t;
                    div()
                        .flex_none()
                        .w(px(width * t))
                        .opacity(t)
                        .overflow_hidden()
                        .child(div().w(px(width)).flex().child(element))
                        .into_any_element()
                })
            })
            .collect();
        let tone = motion::mix(theme.accent, theme.text, 0.2);
        let toggle_id = task_id.clone();
        let hover_id = task_id.clone();
        let title_el = div()
            .id(SharedString::from(format!("tab-group-{task_id}")))
            .debug_selector(|| format!("tab-group-{task_id}"))
            .role(gpui::Role::Button)
            .aria_label(title.clone())
            .aria_expanded(!collapsed)
            .relative()
            .flex_shrink(100.0)
            .min_w(px(TAB_GROUP_TITLE_MIN_WIDTH))
            .max_w(px(TAB_MAX_WIDTH))
            .h(px(TAB_HEIGHT))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .px(px(10.0))
            .rounded(px(8.0))
            .text_size(px(12.0))
            .text_color(if on_page { theme.text } else { tone })
            .cursor_pointer()
            .when(on_page, |el| match tones {
                Some(tones) => el.child(folder_tab_backdrop(tones, self.chat_backdrop.clone())),
                None => el
                    .bg(crate::theme::glass_selected_bg())
                    .shadow(crate::theme::glass_selected_shadows()),
            })
            .when(!on_page, |el| {
                let wash = tones.map_or_else(|| theme.glass_hover(), |tones| tones.wash);
                el.hover(move |s| s.bg(wash))
            })
            .block_mouse_except_scroll()
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    this.tab_group_hover = Some(hover_id.clone());
                } else if this.tab_group_hover.as_deref() == Some(hover_id.as_str()) {
                    this.tab_group_hover = None;
                }
                cx.notify();
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.toggle_tab_group(&toggle_id, cx);
            }))
            .child(
                icon(icons::MISSION_CONTROL)
                    .size(px(14.0))
                    .flex_none()
                    .text_color(tone),
            )
            .child(div().min_w_0().truncate().child(title))
            .when(hidden_asks, |el| {
                el.child(div().size(px(6.0)).flex_none().rounded_full().bg(
                    spaces::status_dot_color(ChatIndicator::AwaitingInput, &theme),
                ))
            })
            .children(menu);
        div()
            .id(SharedString::from(format!("tab-group-box-{task_id}")))
            .min_w(px(min_width))
            .h(px(TAB_HEIGHT))
            .flex()
            .flex_row()
            .items_center()
            .rounded(px(10.0))
            .bg(crate::theme::wash(0.05))
            .child(title_el)
            .children(members)
            .into_any_element()
    }

    fn render_tab_group_menu(
        &mut self,
        task_id: &str,
        ids: &[String],
        tabs: &std::collections::HashMap<String, (SharedString, Vec<InstanceIcon>, ChatIndicator)>,
        selected: Option<&str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let tone = motion::mix(theme.accent, theme.text, 0.2);
        let hover_id = task_id.to_owned();
        let page_id = task_id.to_owned();
        let mut card = popover::popover_card(&theme)
            .w(px(270.0))
            .flex()
            .flex_col()
            .id(SharedString::from(format!("tab-group-menu-{task_id}")))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    this.tab_group_menu_hover = Some(hover_id.clone());
                } else if this.tab_group_menu_hover.as_deref() == Some(hover_id.as_str()) {
                    this.tab_group_menu_hover = None;
                }
                cx.notify();
            }))
            .child(
                popover::menu_row(&theme, false, format!("tab-group-page-{task_id}"))
                    .id(SharedString::from(format!("tab-group-page-{task_id}")))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.close_tab_group_menu(cx);
                        this.open_task_center(page_id.clone(), cx);
                    }))
                    .child(
                        icon(icons::MISSION_CONTROL)
                            .size(px(14.0))
                            .flex_none()
                            .text_color(tone),
                    )
                    .child(
                        div()
                            .text_color(theme.text_muted)
                            .child(crate::tasks::TASK_PAGE),
                    ),
            );
        for id in ids {
            let Some((label, harness, status)) = tabs.get(id).cloned() else {
                continue;
            };
            let open_id = id.clone();
            card = card.child(
                popover::menu_row(
                    &theme,
                    selected == Some(id.as_str()),
                    format!("tab-group-row-{id}"),
                )
                .id(SharedString::from(format!("tab-group-row-{id}")))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.close_tab_group_menu(cx);
                    this.navigate_to(Some(open_id.clone()), true, cx);
                }))
                .children(
                    harness
                        .last()
                        .map(|brand| brand.svg(theme.text_muted).size(px(14.0)).flex_none()),
                )
                .child(div().flex_1().min_w_0().truncate().child(label))
                .when(
                    !matches!(status, ChatIndicator::Idle | ChatIndicator::Working),
                    |el| {
                        el.child(
                            div()
                                .size(px(6.0))
                                .flex_none()
                                .rounded_full()
                                .bg(spaces::status_dot_color(status, &theme)),
                        )
                    },
                ),
            );
        }
        popover::hover_menu_below(format!("tab-group-menu-{task_id}"), card.into_any_element())
    }

    /// The trailing titlebar strip follows the columns beneath it.
    pub(super) fn titlebar_trailing_layout(&self, cx: &App) -> TitlebarTrailing {
        let sidebar_edge = self.sidebar_edge();
        let plus_inset = TITLEBAR_ACTION_SLOT_WIDTH * self.titlebar_plus_alpha(cx);

        // Content starts at the inset card's left edge while the sidebar is
        // open, and slides toward the control cluster as it collapses.
        let content_left =
            (sidebar_edge + Theme::SPACE_LG).max(self.title_bar_content_start() + plus_inset);

        // Trailing titlebar section. With the changes pane open this is the
        // PANE'S HEADER — a strip exactly as wide as the pane carrying its
        // controls (scope dropdown, ref selector, fold-all from the Changes
        // entity; expand + close shell-side). It lives up here because the
        // titlebar overlay owns this band's hit-testing: controls mounted in
        // the pane itself would sit under the drag region and never see a
        // click. Closed, it is just the stable open/close toggle.
        let fit = self.horizontal_fit();
        let right_pane_visible = (fit.right
            && (self.right_pane_open(cx) || self.tween_active(self.right_tween)))
            || self.fit_exit_width(1).is_some();
        let takeover = right_pane_visible
            && (self.right_pane_expanded || self.tween_active(self.main_takeover_tween));
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
            (sidebar_edge - 8.0).max(cluster_end)
        } else {
            content_left
        };
        let row_gap = 8.0;
        let files_width = self.files_visible_width(cx);
        let right_pad = self.titlebar_right_pad(TITLEBAR_ACTION_EDGE_INSET);
        // The row's gaps are outside the fixed-width panel controls.
        let gap_budget = if takeover { row_gap } else { row_gap * 3.0 };
        // Match the surface's visible width, including its resize bounce,
        // so the header follows the pane seam as the sidebar moves.
        let right_visible = self.right_visible_width(cx);
        let widths = panel_titlebar_widths(
            right_visible,
            files_width,
            self.viewport_width - self.content_inset() - row_left - right_pad - gap_budget,
            right_pad,
        );
        // Match the pane's stable inner width during open/close. Reflowing
        // the scroller on every masked frame clamps its scroll offset and
        // makes the tabs jump when the close is reversed.
        let content_width = panel_titlebar_widths(
            self.right_content_width(self.right_target(cx)),
            files_width,
            self.viewport_width - self.content_inset() - row_left - right_pad - gap_budget,
            right_pad,
        )
        .surface_reveal
        .max(widths.surface_reveal);
        TitlebarTrailing {
            fit,
            right_pane_visible,
            takeover,
            row_left,
            row_gap,
            right_pad,
            widths,
            content_width,
        }
    }

    /// The unified titlebar in chat mode:
    /// `[new-session +] [session tabs] … [toggle-changes]`.
    /// Carries the titlebar duties (drag region, animated left inset, the
    /// right-pane controls on git projects).
    pub(super) fn render_session_title_bar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).clone();
        // The canvas shows no tabs-adjacent chrome beyond the strip itself;
        // the bar keeps its height, drag region, and buttons.
        let on_canvas = self.state.read(cx).selected_chat.is_none();

        // The new-session `+` renders in the WINDOW-CONTROL CLUSTER whenever a
        // session is selected (`render_titlebar_cluster`) — this row budgets
        // one button slot so the tabs never sit under it.
        let TitlebarTrailing {
            fit,
            right_pane_visible,
            takeover,
            row_left,
            row_gap,
            right_pad,
            widths,
            content_width,
        } = self.titlebar_trailing_layout(cx);

        let show_chat_actions = matches!(self.route, Route::Chat) && !on_canvas && !takeover;
        if !show_chat_actions {
            self.drop_hidden_chat_action_menus(cx);
        }
        let chat_actions = show_chat_actions.then(|| {
            let review_chip = self.composer.update(cx, |composer, cx| {
                composer.render_review_chip(&theme, window, cx)
            });
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(8.0))
                .children(review_chip)
                .child(self.render_handoff_button(&theme, cx))
        });
        let page_controls = match self.route {
            Route::Tasks => Some(self.tasks_view_switch(&theme, cx)),
            Route::TaskCenter => Some(self.tree_view_switch(&theme, cx)),
            _ => None,
        };
        let running_subagents = {
            let state = self.state.read(cx);
            state
                .selected_chat
                .as_deref()
                .map_or(0, |chat| state.running_subagents_for(chat, Utc::now()))
        };
        let trailing: Option<gpui::AnyElement> = {
            let mut controls = div()
                .id("right-titlebar-controls")
                .debug_selector(|| "right-titlebar-controls".into())
                .flex_none()
                .h_full()
                .flex()
                .flex_row()
                .items_center()
                // The transcript extends under this band (it fades beneath
                // the tab strip); a wheel over the tabs must scroll the
                // strip, never the surface behind it.
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation());
            if right_pane_visible {
                // The right pane's SURFACE TABS (t3 RightPanelTabs) — the diff
                // options that used to live here moved into the pane's own
                // second row; expand stays in this band (user request).
                let show_pane_controls =
                    widths.surface_reveal >= RIGHT_PANE_HEADER_CONTROLS_MIN_WIDTH;
                // The summary + toggles are the fixed right-edge anchor, like
                // the left sidebar control. Only the tabs + expand section
                // reveals to its left; including the toggle in this animated
                // width compressed both icons into the same clipped box at
                // open.
                controls = controls.child(
                    div()
                        .w(px(widths.surface_reveal))
                        .h_full()
                        .flex_none()
                        .relative()
                        .overflow_hidden()
                        .child(
                            div()
                                .debug_selector(|| "right-titlebar-content".into())
                                .absolute()
                                .right_0()
                                .top_0()
                                .w(px(content_width))
                                .h_full()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(4.0))
                                .overflow_hidden()
                                // 8 + the trigger's own 8px pad = the pane's 16px
                                // text gutter. The 4px right padding is the stable
                                // gap before the fixed toggle.
                                .when(show_pane_controls, |el| {
                                    el.pl(px(8.0))
                                        .pr(px(4.0))
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .h_full()
                                                .overflow_hidden()
                                                .child(self.render_right_tab_strip(cx)),
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
                                            cx.listener(|this, _, _, cx| {
                                                this.toggle_right_pane_expand(cx)
                                            }),
                                        ))
                                }),
                        ),
                );
            }
            let files_panel_label = if self.files_panel_open(cx) && fit.files {
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
                                        icons::FOLDER_TREE,
                                        files_panel_label,
                                        &theme,
                                        cx.listener(|this, _, window, cx| {
                                            this.toggle_files_panel(window, cx)
                                        }),
                                    )
                                    .role(gpui::Role::Button)
                                    .aria_label(files_panel_label)
                                    .when(self.files_panel_open(cx) && fit.files, |button| {
                                        button.bg(crate::theme::wash(0.09))
                                    })
                                    // Running subagents live in this panel: mark the
                                    // button so they are findable while it is closed.
                                    .when(running_subagents > 0, |button| {
                                        crate::running_pill::mark_files_button(
                                            button,
                                            running_subagents,
                                            &theme,
                                        )
                                    }),
                                )
                            })
                            .child(header_icon_button_with(
                                "toggle-changes",
                                icons::sidebar_glyph(
                                    motion::state_t(
                                        "toggle-changes",
                                        self.right_pane_open(cx) && fit.right,
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
        let triage = self.triage_frame();
        let triage_pill = (!takeover)
            .then(|| self.render_triage_pill(&triage, window, cx))
            .flatten();

        let inner = div()
            .size_full()
            .flex()
            .items_center()
            .gap(px(row_gap))
            .pl(px(row_left))
            .pr(px(right_pad))
            .children(session_tabs)
            .child(div().flex_1())
            .children(triage_pill)
            .children(chat_actions)
            .children(page_controls)
            .children(trailing);

        // The unified window titlebar: full-width on the glass shell, ABOVE
        // the inset card. No bottom border — the card's own hairline is the
        // separation; the glass gutter shows between.
        let seam = (!takeover).then(|| self.titlebar_seam_divider(cx));
        let bar = div()
            .relative()
            .h(px(Theme::TITLEBAR_HEIGHT))
            .flex_none()
            .children(seam.flatten())
            .child(inner);
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
            self.shell.update(cx, |shell, cx| {
                div()
                    .w(px(900.))
                    .h(px(60.))
                    .child(shell.render_session_title_bar(window, cx))
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
            running_subagents: 0,
            compact_plan: None,
            classifying: None,
            last_turn_outcome: Default::default(),
            terminal_activity: Default::default(),
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

    fn add_terminal(shell: &mut Shell, terminal_id: &str, cx: &mut Context<Shell>) {
        let panel = shell.main_terminal_panel(cx);
        panel.update(cx, |panel, cx| {
            panel.reserve_attached_for_test("term", terminal_id, cx);
        });
    }

    #[gpui::test]
    fn terminal_tab_takes_the_main_column_without_selecting_a_chat(cx: &mut TestAppContext) {
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
            add_terminal(shell, "t1", cx);

            shell.navigate_to(Some("term:t1".into()), true, cx);
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(shell.route, Route::Terminal);
            assert_eq!(shell.state.read(cx).selected_chat.as_deref(), Some("b"));
            assert_eq!(shell.selected_tab_id(cx).as_deref(), Some("term:t1"));
            assert_eq!(shell.open_tab_ids(cx), ["a", "b", "term:t1"]);

            shell.navigate_to(Some("b".into()), true, cx);
            assert_eq!(shell.route, Route::Chat);
            assert_eq!(shell.selected_tab_id(cx).as_deref(), Some("b"));
        });
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(cx.debug_bounds("session-tab-term:t1").is_some());
    }

    #[gpui::test]
    fn closing_a_terminal_tab_detaches_and_the_sidebar_kill_drops_it(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, cx| {
                state.chats = vec![chat("a")];
                state.chats_synced = true;
                state.spaces_synced = true;
                state.apply_sessions(vec![working("a")]);
                state.select_chat(Some("a".into()), cx);
            });
            shell.on_state_changed(&shell.state.clone(), cx);
            add_terminal(shell, "t1", cx);
            shell.show_terminal_tab("term:t1".into(), cx);

            assert!(shell.close_focused_tab(cx));
            assert_eq!(shell.route, Route::Chat);
            assert_eq!(shell.open_tab_ids(cx), ["a"]);
            assert_eq!(shell.terminal_tab_rows(cx).len(), 1);

            shell.show_terminal_tab("term:t1".into(), cx);
            assert_eq!(shell.open_tab_ids(cx), ["a", "term:t1"]);
            shell.kill_terminal_tab("term:t1", cx);
            assert_eq!(shell.route, Route::Chat);
            assert!(shell.terminal_tab_rows(cx).is_empty());
            assert_eq!(shell.open_tab_ids(cx), ["a"]);
        });
    }

    #[gpui::test]
    fn stored_terminal_tabs_wait_for_the_engine_list(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, cx| {
                state.chats = vec![chat("a")];
                state.chats_synced = true;
                state.spaces_synced = true;
                state.apply_sessions(vec![working("a")]);
                state.select_chat(Some("a".into()), cx);
            });
            shell.settings.open_tabs = Some(vec!["a".into(), "term:gone".into(), "term:t1".into()]);
            shell.settings.open_tabs_stable = true;
            add_terminal(shell, "t1", cx);
            shell.sync_open_tabs(cx);
            assert_eq!(
                shell.settings.open_tabs.as_deref().unwrap(),
                ["a", "term:gone", "term:t1"]
            );
            assert_eq!(shell.open_tab_ids(cx), ["a", "term:t1"]);

            shell.terminals_synced = true;
            shell.sync_open_tabs(cx);
            assert_eq!(
                shell.settings.open_tabs.as_deref().unwrap(),
                ["a", "term:t1"]
            );
        });
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
    fn a_driving_chat_wears_the_cursor_glyph_and_gets_its_activity_glyph_back_after(
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
        assert!(cx.debug_bounds("session-tab-working-a").is_some());
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
                cx.debug_bounds("session-tab-working-a").is_some(),
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
            "revoked wears the activity glyph again"
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
    fn walking_back_from_a_chat_lands_on_the_draft_left_for_it(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state.chats = vec![chat("a")];
                state.chats_synced = true;
                state.spaces_synced = true;
            });
            shell.on_state_changed(&shell.state.clone(), cx);
            let draft = shell.state.read(cx).active_draft.clone().unwrap();
            invest(&draft, "keep me", cx);
            shell.select_existing_chat("a".into(), cx);
            shell.on_state_changed(&shell.state.clone(), cx);

            shell.navigate_back(cx);
            shell.on_state_changed(&shell.state.clone(), cx);
            let state = shell.state.read(cx);
            assert_eq!(state.selected_chat, None);
            assert_eq!(state.active_draft.as_deref(), Some(draft.as_str()));

            shell.navigate_forward(cx);
            shell.on_state_changed(&shell.state.clone(), cx);
            assert_eq!(shell.state.read(cx).selected_chat.as_deref(), Some("a"));
            assert!(!shell.nav.can_forward());
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

#[cfg(test)]
mod tab_group_tests {
    use super::{group_tab_order, strip_child_index};
    use crate::tasks::TabMember;

    #[test]
    fn mission_tabs_gather_at_the_first_member_lead_first() {
        let member = |step| TabMember {
            task_id: "t".into(),
            lead: "lead".into(),
            step,
        };
        let members = std::collections::HashMap::from([
            ("lead".to_string(), member(None)),
            ("s1".to_string(), member(Some(0))),
            ("s2".to_string(), member(Some(1))),
        ]);
        let order: Vec<String> = ["a", "s2", "b", "lead", "c", "s1"]
            .map(String::from)
            .to_vec();
        assert_eq!(
            group_tab_order(&order, &members),
            ["a", "lead", "s1", "s2", "b", "c"]
                .map(String::from)
                .to_vec()
        );
        let strip = ["a", "lead", "s1", "s2", "b", "c"];
        let ix = |selected| strip_child_index(strip.into_iter(), selected, &members, Some("c"));
        assert_eq!(
            ["a", "s2", "b", "c"].map(ix),
            [Some(0), Some(1), Some(2), Some(2)],
            "a group and a split pair each scroll as one child"
        );
    }
}
