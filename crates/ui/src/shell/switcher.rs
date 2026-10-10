//! Ctrl+Tab session switcher: a held-modifier overlay over the sidebar's
//! sessions (narrowed by the Settings → Shortcuts recency window, plus the
//! strip's tabs) in most-recently-used order (macOS Cmd+Tab: the first press
//! lands on the previous session, release commits, Escape cancels). Each card
//! carries the last frame this device saw of that session — a capture of our
//! own window cropped to the conversation column, taken as the selection
//! LEAVES it — or, before the session has been viewed, its title and last
//! message.

use super::*;
use std::sync::Arc;
use zeron_proto::ChatIndicator;

/// Card width in points; the thumbnail keeps the column's aspect inside it.
const CARD_WIDTH: f32 = 220.0;
const THUMB_HEIGHT: f32 = 140.0;
/// Thumbnail pixel width — twice the card for Retina; the source column is
/// wider than this at nominal resolution, so it is always a downscale.
const THUMB_WIDTH: u32 = 440;

/// One window frame as the window server holds it: tightly packed BGRA at
/// nominal (1px per point) resolution — the byte order `RenderImage` consumes.
pub(super) struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// The open switcher: sessions in MRU order (the current session first) and
/// the highlighted index.
pub(super) struct Switcher {
    pub order: Vec<String>,
    pub hi: usize,
    pub scroll: gpui::ScrollHandle,
}

/// Capture this app's key window. Own-process windows need no Screen
/// Recording grant (verified 2026-09-17 on macOS 27). `None` off macOS,
/// without a window (tests), or when the window server hands back an
/// unexpected pixel layout.
#[cfg(target_os = "macos")]
pub(super) fn capture_window() -> Option<CapturedFrame> {
    use core_graphics::geometry::{CGPoint, CGRect, CGSize};
    use core_graphics::window::{
        create_image, kCGWindowImageBoundsIgnoreFraming, kCGWindowImageNominalResolution,
        kCGWindowImageShouldBeOpaque, kCGWindowListOptionIncludingWindow,
    };
    let mtm = objc2::MainThreadMarker::new()?;
    let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    let window = app
        .keyWindow()
        .or_else(|| app.mainWindow())
        .or_else(|| app.windows().firstObject())?;
    let id = u32::try_from(window.windowNumber()).ok()?;
    // CGRectNull: the image takes the window's own bounds.
    let null_rect = CGRect::new(
        &CGPoint::new(f64::INFINITY, f64::INFINITY),
        &CGSize::new(0.0, 0.0),
    );
    // Opaque: the glass shell's translucent regions would otherwise carry
    // their alpha into the card and show the card's own fill through.
    let image = create_image(
        null_rect,
        kCGWindowListOptionIncludingWindow,
        id,
        kCGWindowImageNominalResolution
            | kCGWindowImageBoundsIgnoreFraming
            | kCGWindowImageShouldBeOpaque,
    )?;
    let (width, height) = (image.width(), image.height());
    let stride = image.bytes_per_row();
    if image.bits_per_pixel() != 32 || width == 0 || height == 0 {
        return None;
    }
    let data = image.data();
    let bytes = data.bytes();
    let mut bgra = Vec::with_capacity(width * height * 4);
    for row in 0..height {
        let start = row * stride;
        bgra.extend_from_slice(bytes.get(start..start + width * 4)?);
    }
    Some(CapturedFrame {
        width: width as u32,
        height: height as u32,
        bgra,
    })
}

#[cfg(not(target_os = "macos"))]
pub(super) fn capture_window() -> Option<CapturedFrame> {
    None
}

/// Whether a sidebar session is recent enough for the switcher: its last
/// activity (the sidebar's own recency stamp) is at or after `cutoff`, or
/// there is no cutoff. Pure.
pub(super) fn within_window(
    chat: &zeron_proto::Chat,
    cutoff: Option<chrono::DateTime<Utc>>,
) -> bool {
    cutoff.is_none_or(|cutoff| chat.last_message_at.unwrap_or(chat.created_at) >= cutoff)
}

/// The switcher's candidates: the sidebar's rows in their drawn order,
/// narrowed to the recency window, then the strip's tabs that the window (or
/// the sidebar filter) left out — the selected session, a split pair, and
/// drafts are always reachable. Pure.
pub(super) fn switcher_candidates(
    sidebar: impl IntoIterator<Item = (String, bool)>,
    strip: &[String],
) -> Vec<String> {
    let mut ids: Vec<String> = sidebar
        .into_iter()
        .filter_map(|(id, recent)| recent.then_some(id))
        .collect();
    for id in strip {
        if !ids.contains(id) {
            ids.push(id.clone());
        }
    }
    ids
}

/// The switcher's card order: the candidates most recently used first; those
/// never selected this launch follow in candidate order. Pure.
pub(super) fn switcher_order(mru: &[String], visible: &[String]) -> Vec<String> {
    let mut order: Vec<String> = mru
        .iter()
        .filter(|id| visible.contains(id))
        .cloned()
        .collect();
    let rest: Vec<String> = visible
        .iter()
        .filter(|id| !order.contains(id))
        .cloned()
        .collect();
    order.extend(rest);
    order
}

/// Cap the card count at the Settings → Shortcuts limit (0 for none). The
/// strip's tabs stay reachable past the cap, as they do past the recency
/// window. Pure.
pub(super) fn switcher_capped(mut order: Vec<String>, limit: u32, strip: &[String]) -> Vec<String> {
    if limit == 0 {
        return order;
    }
    order.truncate(limit as usize);
    for id in strip {
        if !order.contains(id) {
            order.push(id.clone());
        }
    }
    order
}

/// The conversation column inside a nominal-resolution window frame, as a
/// pixel rect `(x, y, w, h)`: right of the sidebar, left of the right pane,
/// below the titlebar. `None` when the column has no area. Pure.
pub(super) fn column_crop(
    sidebar: f32,
    right_pane: f32,
    frame_width: u32,
    frame_height: u32,
) -> Option<(u32, u32, u32, u32)> {
    let x0 = sidebar.max(0.0).round() as u32;
    let y0 = Theme::TITLEBAR_HEIGHT.round() as u32;
    let x1 = (frame_width as f32 - right_pane.max(0.0)).round() as u32;
    let x1 = x1.min(frame_width);
    if x1 <= x0 + 40 || frame_height <= y0 + 40 {
        return None;
    }
    Some((x0, y0, x1 - x0, frame_height - y0))
}

/// The next highlighted index, wrapping. Pure.
pub(super) fn step_index(hi: usize, len: usize, forward: bool) -> usize {
    if len == 0 {
        return 0;
    }
    if forward {
        (hi + 1) % len
    } else {
        (hi + len - 1) % len
    }
}

/// Crop and downscale a captured frame to the card thumbnail. Runs off the
/// main thread. `None` for an all-black crop (the window server handed back
/// an empty surface) so the card keeps its text fallback.
fn thumbnail(frame: CapturedFrame, crop: (u32, u32, u32, u32)) -> Option<Arc<gpui::RenderImage>> {
    let source = image::RgbaImage::from_raw(frame.width, frame.height, frame.bgra)?;
    let (x, y, w, h) = crop;
    let column = image::imageops::crop_imm(&source, x, y, w, h).to_image();
    if !column.pixels().any(|p| p[0] | p[1] | p[2] != 0) {
        return None;
    }
    let scale = THUMB_WIDTH as f32 / w as f32;
    let height = ((h as f32 * scale).round() as u32).max(1);
    let resized = image::imageops::resize(
        &column,
        THUMB_WIDTH,
        height,
        image::imageops::FilterType::Triangle,
    );
    Some(Arc::new(gpui::RenderImage::new([image::Frame::new(
        resized,
    )])))
}

/// "project @ device" for a draft, from the target it was minted onto.
pub(super) fn draft_target_label(
    draft: &crate::draft_store::DraftSession,
    state: &crate::state::AppState,
) -> String {
    let folder = draft
        .space_id
        .as_deref()
        .filter(|_| !draft.no_project)
        .and_then(|id| state.space_row(id))
        .map(|s| s.display_name().to_string())
        .unwrap_or_else(|| "~".to_string());
    match draft
        .device_id
        .as_deref()
        .and_then(|id| state.device_name(id))
    {
        Some(device) => format!("{folder} @ {device}"),
        None => folder,
    }
}

impl Shell {
    /// Record a selection for the switcher's order: most recent first.
    pub(super) fn touch_session_mru(&mut self, chat_id: &str) {
        self.session_mru.retain(|id| id != chat_id);
        self.session_mru.insert(0, chat_id.to_string());
    }

    /// Snapshot the conversation column of the selected session from the
    /// frame on screen. Called BEFORE a selection change (the painted frame
    /// still shows the session being left) and when the switcher opens; never
    /// with an overlay up, which would be in the picture.
    pub(super) fn snapshot_current_chat(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.route, Route::Chat)
            || self.switcher.is_some()
            || self.command_palette.is_some()
            || self.add_space.is_some()
        {
            return;
        }
        let Some(chat_id) = self.state.read(cx).focused_id() else {
            return;
        };
        let Some(frame) = capture_window() else {
            return;
        };
        let Some(crop) = column_crop(
            self.sidebar_now(),
            self.right_visible_width(cx) + self.files_reserved_width(cx),
            frame.width,
            frame.height,
        ) else {
            return;
        };
        let task = cx
            .background_executor()
            .spawn(async move { thumbnail(frame, crop) });
        cx.spawn(async move |this, cx| {
            let Some(image) = task.await else {
                return;
            };
            this.update(cx, |shell, cx| {
                if let Some(old) = shell.session_thumbs.insert(chat_id, image) {
                    shell.stale_thumbs.push(old);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Ctrl+Tab / Ctrl+Shift+Tab: open the switcher on the previous (next)
    /// session, or move the highlight while it is up; with the cycle-tabs
    /// setting, step straight to the neighboring strip tab. Settings closes
    /// first, as a jump does; quiet under any other overlay.
    pub(super) fn switcher_step(&mut self, forward: bool, cx: &mut Context<Self>) {
        if matches!(self.route, Route::Settings(_)) {
            self.close_settings(cx);
        }
        if matches!(self.route, Route::PullRequests) {
            self.set_route(Route::Chat, cx);
        }
        if !matches!(self.route, Route::Chat | Route::Terminal) {
            return;
        }
        if let Some(sw) = self.switcher.as_mut() {
            sw.hi = step_index(sw.hi, sw.order.len(), forward);
            sw.scroll.scroll_to_item(sw.hi);
            cx.notify();
            return;
        }
        if self.overlay_owns_keyboard(cx) {
            return;
        }
        if self.settings.switcher_cycles_tabs {
            let order = self.open_tab_ids(cx);
            let focused = self.selected_tab_id(cx);
            let next = match focused.and_then(|id| order.iter().position(|t| *t == id)) {
                Some(ix) => step_index(ix, order.len(), forward),
                None if forward => 0,
                None => order.len().saturating_sub(1),
            };
            if let Some(id) = order.get(next).cloned() {
                self.navigate_to(Some(id), true, cx);
            }
            return;
        }
        let cutoff = self.settings.switcher_window.cutoff(Utc::now());
        let sidebar = self.sidebar_visible_order(cx);
        let recent: Vec<(String, bool)> = {
            let state = self.state.read(cx);
            sidebar
                .into_iter()
                .map(|id| {
                    let recent = state
                        .chats
                        .iter()
                        .find(|c| c.id == id)
                        .is_some_and(|c| within_window(c, cutoff));
                    (id, recent)
                })
                .collect()
        };
        // Idle tabs linger dimmed on the strip; only live ones bypass the cap.
        let idle = self.idle_tab_ids(cx);
        let strip: Vec<String> = self
            .open_tab_ids(cx)
            .into_iter()
            .filter(|id| !idle.contains(id) && !terminal_tabs::is_terminal_tab(id))
            .collect();
        let candidates = switcher_candidates(recent, &strip);
        let order = switcher_capped(
            switcher_order(&self.session_mru, &candidates),
            self.settings.switcher_limit,
            &strip,
        );
        match order.len() {
            0 => return,
            // One tab and nothing selected (the canvas): just go there.
            1 if self.state.read(cx).selected_chat.is_none() => {
                self.open_chat(order[0].clone(), cx);
                return;
            }
            1 => return,
            _ => {}
        }
        self.snapshot_current_chat(cx);
        let hi = step_index(0, order.len(), forward);
        let scroll = gpui::ScrollHandle::new();
        scroll.scroll_to_item(hi);
        self.switcher = Some(Switcher { order, hi, scroll });
        cx.notify();
    }

    /// Modifier release or a card click: land on the highlighted session.
    pub(super) fn switcher_commit(&mut self, cx: &mut Context<Self>) {
        let Some(sw) = self.switcher.take() else {
            return;
        };
        if let Some(id) = sw.order.get(sw.hi).cloned()
            && self.state.read(cx).focused_id().as_deref() != Some(id.as_str())
        {
            // No snapshot: the frame on screen is this overlay, not a
            // snapshot-worthy view of the session being left.
            self.navigate_to(Some(id), false, cx);
        }
        cx.notify();
    }

    pub(super) fn switcher_close(&mut self, cx: &mut Context<Self>) {
        if self.switcher.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn render_switcher(
        &mut self,
        viewport: gpui::Size<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        // Replaced thumbnails leave the atlas here, where a window is at hand.
        for old in self.stale_thumbs.drain(..) {
            let _ = window.drop_image(old);
        }
        let sw = self.switcher.as_ref()?;
        let theme = Theme::of(cx).for_popup();
        let now = Utc::now();
        let hi = sw.hi;
        let scroll = sw.scroll.clone();
        let cards: Vec<AnyElement> = sw
            .order
            .iter()
            .enumerate()
            .map(|(ix, id)| {
                let state = self.state.read(cx);
                let chat = state.chats.iter().find(|c| &c.id == id);
                let draft = state.draft_session(id);
                let title: SharedString = chat
                    .map(|c| transcript::single_line(c.title.as_deref().unwrap_or("New session")))
                    .or_else(|| draft.map(|_| "New session".to_string()))
                    .unwrap_or_default()
                    .into();
                let target: SharedString = chat
                    .map(|c| {
                        let folder = state
                            .space_for_chat(c)
                            .map(|s| s.display_name().to_string())
                            .unwrap_or_else(|| "~".to_string());
                        let device = state.device_name(&c.device_id).unwrap_or("Unknown device");
                        format!("{folder} @ {device}")
                    })
                    .or_else(|| draft.map(|d| draft_target_label(d, state)))
                    .unwrap_or_default()
                    .into();
                let status = chat
                    .map(|c| state.display_status_for(c, now))
                    .unwrap_or(ChatIndicator::Idle);
                let status_label = match status {
                    ChatIndicator::Working => Some("Working"),
                    ChatIndicator::AwaitingInput => Some("Input"),
                    ChatIndicator::Errored => Some("Failed"),
                    ChatIndicator::Completed => Some("Done"),
                    ChatIndicator::Idle => None,
                };
                // A draft card shows the typed text where an unviewed
                // session shows its last message (d18).
                let preview: Option<SharedString> = chat
                    .and_then(|c| c.last_message_preview.clone())
                    .or_else(|| draft.and_then(|d| crate::draft_store::entry_prompt(&d.id, cx)))
                    .map(|p| transcript::single_line(&p).into());
                let brand = chat
                    .and_then(|c| c.config.as_ref().map(|c| c.harness))
                    .map(crate::pickers::harness_brand_icon)
                    .or_else(|| draft.map(|_| (icons::PEN, Some(theme.warning))));
                let thumb = self.session_thumbs.get(id).cloned();
                let is_hi = ix == hi;

                let visual: AnyElement = match thumb {
                    Some(image) => div()
                        .w(px(CARD_WIDTH))
                        .h(px(THUMB_HEIGHT))
                        .rounded(px(8.0))
                        .overflow_hidden()
                        .border_1()
                        .border_color(theme.border)
                        .child(
                            gpui::img(image)
                                .size_full()
                                .object_fit(gpui::ObjectFit::Cover),
                        )
                        .into_any_element(),
                    None => div()
                        .w(px(CARD_WIDTH))
                        .h(px(THUMB_HEIGHT))
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(theme.border)
                        .bg(crate::theme::wash(0.04))
                        .p(px(10.0))
                        .overflow_hidden()
                        .text_size(px(12.0))
                        .line_height(px(17.0))
                        .text_color(theme.text_muted)
                        .child(preview.unwrap_or_else(|| "No messages yet".into()))
                        .into_any_element(),
                };
                div()
                    .id(("session-switcher-card", ix))
                    .flex_none()
                    .p(px(8.0))
                    .rounded(px(12.0))
                    .border_1()
                    .border_color(if is_hi {
                        theme.border_strong
                    } else {
                        gpui::transparent_black()
                    })
                    .when(is_hi, |el| el.bg(crate::theme::glass_selected_bg()))
                    .cursor_pointer()
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered
                            && let Some(sw) = this.switcher.as_mut()
                            && sw.hi != ix
                        {
                            sw.hi = ix;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(sw) = this.switcher.as_mut() {
                            sw.hi = ix;
                        }
                        this.switcher_commit(cx);
                    }))
                    .child(visual)
                    .child(
                        div()
                            .mt(px(8.0))
                            .w(px(CARD_WIDTH))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.0))
                            .when_some(brand, |el, (path, tint)| {
                                el.child(
                                    icon(path)
                                        .size(px(14.0))
                                        .flex_none()
                                        .text_color(tint.unwrap_or(theme.text_muted)),
                                )
                            })
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(13.0))
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(theme.text)
                                    .child(title),
                            ),
                    )
                    .child(
                        div()
                            .w(px(CARD_WIDTH))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.0))
                            .text_size(px(11.0))
                            .text_color(theme.text_muted.opacity(0.7))
                            .child(div().min_w_0().truncate().child(target))
                            .when_some(status_label, |el, label| {
                                el.child(
                                    div()
                                        .flex_none()
                                        .text_color(spaces::status_dot_color(status, &theme))
                                        .child(SharedString::from(label)),
                                )
                            }),
                    )
                    .into_any_element()
            })
            .collect();

        let max_width = f32::from(viewport.width) - 48.0;
        let max_height = f32::from(viewport.height) - 48.0;
        let panel = div()
            .id("session-switcher")
            .max_w(px(max_width))
            .max_h(px(max_height))
            .overflow_y_scroll()
            .track_scroll(&scroll)
            .p(px(10.0))
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(px(6.0))
            .rounded(px(16.0))
            .border_1()
            .border_color(theme.border)
            .when(!theme.is_frost(), |el| el.shadow_lg())
            .bg(popover::surface_bg(&theme))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.switcher_close(cx)))
            .children(cards);
        Some(popover::modal_glass(
            "session-switcher-modal",
            viewport,
            panel.into_any_element(),
            16.0,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn order_is_mru_then_unvisited_strip_order() {
        let mru = ids(&["c", "a", "gone"]);
        let visible = ids(&["a", "b", "c", "d"]);
        assert_eq!(switcher_order(&mru, &visible), ids(&["c", "a", "b", "d"]));
        assert_eq!(switcher_order(&[], &visible), visible);
        assert!(switcher_order(&mru, &[]).is_empty());
    }

    #[test]
    fn candidates_are_recent_sidebar_rows_then_leftover_strip_tabs() {
        let sidebar = vec![
            ("a".to_string(), true),
            ("old".to_string(), false),
            ("b".to_string(), true),
        ];
        let strip = ids(&["b", "old", "draft"]);
        assert_eq!(
            switcher_candidates(sidebar, &strip),
            ids(&["a", "b", "old", "draft"])
        );
    }

    #[test]
    fn cap_keeps_the_head_and_the_strip_tabs() {
        let order = ids(&["c", "a", "b", "d"]);
        assert_eq!(
            switcher_capped(order.clone(), 2, &ids(&["d"])),
            ids(&["c", "a", "d"])
        );
        assert_eq!(
            switcher_capped(order.clone(), 2, &ids(&["a"])),
            ids(&["c", "a"])
        );
        assert_eq!(switcher_capped(order.clone(), 0, &ids(&["d"])), order);
    }

    #[test]
    fn window_uses_last_message_then_creation() {
        let now = Utc::now();
        let chat = |created_h: i64, last_h: Option<i64>| -> zeron_proto::Chat {
            let mut value = serde_json::json!({
                "id": "c", "deviceId": "local", "archived": false,
                "createdAt": now - chrono::Duration::hours(created_h),
            });
            if let Some(h) = last_h {
                value["lastMessageAt"] = serde_json::json!(now - chrono::Duration::hours(h));
            }
            serde_json::from_value(value).unwrap()
        };
        let cutoff = crate::settings::SwitcherWindow::Hours8.cutoff(now);
        assert!(within_window(&chat(30, Some(2)), cutoff));
        assert!(!within_window(&chat(30, Some(9)), cutoff));
        assert!(within_window(&chat(2, None), cutoff));
        assert!(!within_window(&chat(9, None), cutoff));
        assert!(within_window(
            &chat(300, Some(200)),
            crate::settings::SwitcherWindow::All.cutoff(now)
        ));
    }

    #[test]
    fn step_wraps_both_ways() {
        assert_eq!(step_index(0, 3, true), 1);
        assert_eq!(step_index(2, 3, true), 0);
        assert_eq!(step_index(0, 3, false), 2);
        assert_eq!(step_index(0, 0, true), 0);
    }

    #[test]
    fn crop_excludes_sidebar_titlebar_and_right_pane() {
        let title = Theme::TITLEBAR_HEIGHT.round() as u32;
        assert_eq!(
            column_crop(260.0, 300.0, 1500, 900),
            Some((260, title, 940, 900 - title))
        );
        // Collapsed sidebar and closed pane: the whole width.
        assert_eq!(
            column_crop(0.0, 0.0, 1500, 900),
            Some((0, title, 1500, 900 - title))
        );
        // A takeover pane leaves no column.
        assert_eq!(column_crop(260.0, 1240.0, 1500, 900), None);
        assert_eq!(column_crop(0.0, 0.0, 1500, 20), None);
    }
}
