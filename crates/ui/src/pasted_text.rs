use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use gpui::{
    AnyElement, App, InteractiveElement as _, IntoElement as _, ListAlignment, ListState,
    MouseButton, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    TextRun, Window, div, px,
};

use crate::icons::{self, icon};
use crate::markdown::render;
use crate::theme::Theme;

const BLOCK_LINES: usize = 50;
const BLOCK_BYTES: usize = 8 * 1024;
const CACHE_BUDGET_BYTES: usize = 32 * 1024 * 1024;

/// Whole-line blocks so a multi-megabyte paste never lays out as one text element.
pub(crate) struct PastedText {
    pub(crate) blocks: Vec<SharedString>,
    pub(crate) lines: usize,
    bytes: usize,
}

impl PastedText {
    pub(crate) fn decode(bytes: &[u8]) -> Self {
        let text = String::from_utf8_lossy(bytes);
        let mut blocks = Vec::new();
        let mut block = String::new();
        let mut in_block = 0;
        let mut lines = 0;
        for line in text.lines() {
            lines += 1;
            if in_block > 0 && (in_block == BLOCK_LINES || block.len() + line.len() > BLOCK_BYTES) {
                blocks.push(std::mem::take(&mut block).into());
                in_block = 0;
            }
            if in_block > 0 {
                block.push('\n');
            }
            block.push_str(line);
            in_block += 1;
        }
        if in_block > 0 {
            blocks.push(block.into());
        }
        Self {
            blocks,
            lines,
            bytes: bytes.len(),
        }
    }
}

pub(crate) fn pill_label(index: u32, lines: Option<usize>) -> String {
    match lines {
        Some(1) => format!("Pasted text {index} · 1 line"),
        Some(lines) => format!("Pasted text {index} · {lines} lines"),
        None => format!("Pasted text {index}"),
    }
}

#[derive(Default)]
struct Cache {
    map: HashMap<(String, String), (Arc<PastedText>, u64)>,
    tick: u64,
    bytes: usize,
}

impl Cache {
    fn insert(&mut self, key: (String, String), text: Arc<PastedText>) {
        self.tick += 1;
        self.bytes += text.bytes;
        if let Some((old, _)) = self.map.insert(key.clone(), (text, self.tick)) {
            self.bytes -= old.bytes;
        }
        while self.bytes > CACHE_BUDGET_BYTES {
            let Some(oldest) = self
                .map
                .iter()
                .filter(|(k, _)| **k != key)
                .min_by_key(|(_, (_, tick))| *tick)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some((old, _)) = self.map.remove(&oldest) {
                self.bytes -= old.bytes;
            }
        }
    }
}

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn alias_path(upload_id: &str) -> String {
    let id8: String = upload_id.chars().take(8).collect();
    format!("upload-alias://{id8}")
}

/// A queued send's host-rewritten path resolves through the alias its send seeded.
pub(crate) fn cached(device_ids: &[String], path: &str) -> Option<Arc<PastedText>> {
    let mut cache = cache().lock().unwrap();
    cache.tick += 1;
    let tick = cache.tick;
    for device in device_ids {
        if let Some((text, last_used)) = cache.map.get_mut(&(device.clone(), path.to_owned())) {
            *last_used = tick;
            return Some(text.clone());
        }
    }
    let id8 = crate::attachments::upload_alias_id8(path)?;
    for device in device_ids {
        if let Some(text) = cache
            .map
            .get(&(device.clone(), alias_path(&id8)))
            .map(|(text, _)| text.clone())
        {
            cache.insert((device.clone(), path.to_owned()), text.clone());
            return Some(text);
        }
    }
    None
}

pub(crate) fn store(device: &str, path: &str, text: Arc<PastedText>) {
    cache()
        .lock()
        .unwrap()
        .insert((device.to_owned(), path.to_owned()), text);
}

pub(crate) fn seed(device: &str, path: &str, bytes: &[u8]) {
    store(device, path, Arc::new(PastedText::decode(bytes)));
}

pub(crate) fn seed_alias(device: &str, upload_id: &str, bytes: &[u8]) {
    seed(device, &alias_path(upload_id), bytes);
}

pub(crate) struct PastedTextPreview {
    pub(crate) path: SharedString,
    pub(crate) title: SharedString,
    pub(crate) list: ListState,
    pub(crate) failed: bool,
    pub(crate) selection_prefix: String,
}

impl PastedTextPreview {
    pub(crate) fn new(path: SharedString, title: SharedString, selection_prefix: String) -> Self {
        Self {
            path,
            title,
            list: ListState::new(0, ListAlignment::Top, px(800.0)),
            failed: false,
            selection_prefix,
        }
    }
}

pub(crate) fn overlay(
    window: &mut Window,
    preview: &PastedTextPreview,
    text: Option<Arc<PastedText>>,
    focus: &gpui::FocusHandle,
    on_close: impl Fn(&mut Window, &mut App) + 'static,
    cx: &mut App,
) -> AnyElement {
    let theme = Theme::of(cx).clone();
    let viewport = window.viewport_size();
    let on_close = std::rc::Rc::new(on_close);
    let close_on_key = on_close.clone();
    let close_on_scrim = on_close.clone();
    let subtitle = match &text {
        Some(text) if text.lines == 1 => "1 line".to_owned(),
        Some(text) => format!("{} lines", text.lines),
        None if preview.failed => "Couldn't load the pasted text".to_owned(),
        None => "Loading…".to_owned(),
    };
    let body = match text {
        Some(text) => {
            if preview.list.item_count() != text.blocks.len() {
                preview.list.reset(text.blocks.len());
            }
            let prefix = preview.selection_prefix.clone();
            let font = gpui::font(theme.font_mono.clone());
            let color = theme.text;
            let wash = theme.selection;
            gpui::list(preview.list.clone(), move |ix, _, _| {
                let block = text.blocks[ix].clone();
                let runs = vec![TextRun {
                    len: block.len(),
                    font: font.clone(),
                    color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }];
                div()
                    .w_full()
                    .px(px(16.0))
                    .child(render::selectable_text_element(
                        format!("{prefix}{ix}").into(),
                        block,
                        runs,
                        wash,
                    ))
                    .into_any_element()
            })
            .flex_1()
            .w_full()
            .py(px(8.0))
            .into_any_element()
        }
        None => div().flex_1().into_any_element(),
    };
    let close = div()
        .id("pasted-text-close")
        .role(gpui::Role::Button)
        .aria_label("Close")
        .flex_none()
        .size(px(24.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .cursor_pointer()
        .text_color(theme.text_muted)
        .hover(|s| s.bg(theme.glass_hover()).text_color(theme.text))
        .child(icon(icons::LUCIDE_X).size(px(14.0)))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on_close(window, cx);
        });
    let header = div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.0))
        .px(px(16.0))
        .py(px(10.0))
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .min_w_0()
                .truncate()
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(preview.title.clone()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(theme.text_muted)
                .child(subtitle),
        )
        .child(close);
    let card = crate::popover::popover_card_flush(&theme)
        .w(px((f32::from(viewport.width) * 0.8).min(960.0)))
        .h(px(f32::from(viewport.height) * 0.8))
        .flex()
        .flex_col()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(render::selection_surface_reset(
            preview.selection_prefix.clone(),
        ))
        .child(header)
        .child(body);
    gpui::deferred(
        gpui::anchored()
            .position(gpui::point(px(0.0), px(0.0)))
            .child(
                div()
                    .id("pasted-text-preview")
                    .occlude()
                    .track_focus(focus)
                    .w(viewport.width)
                    .h(viewport.height)
                    .bg(crate::popover::scrim_alpha(0.7))
                    .flex()
                    .items_center()
                    .justify_center()
                    .on_key_down(move |event: &gpui::KeyDownEvent, window, cx| {
                        let keystroke = &event.keystroke;
                        if keystroke.key == "escape" {
                            cx.stop_propagation();
                            close_on_key(window, cx);
                        } else if keystroke.key == "c"
                            && (keystroke.modifiers.platform || keystroke.modifiers.control)
                            && let Some(text) = crate::markdown::selection::selected_text()
                        {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                            cx.stop_propagation();
                        }
                    })
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        cx.stop_propagation();
                        close_on_scrim(window, cx);
                    })
                    .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                    .child(crate::frost::frosted(
                        crate::popover::CARD_RADIUS,
                        crate::frost::MENU_BLUR,
                        card,
                    )),
            ),
    )
    .priority(3)
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_splits_whole_lines_into_blocks_and_counts_them() {
        let source: String = (0..120).map(|i| format!("line {i}\r\n")).collect();
        let text = PastedText::decode(source.as_bytes());
        assert_eq!(text.lines, 120);
        assert_eq!(text.blocks.len(), 3);
        let rejoined = text
            .blocks
            .iter()
            .map(|b| b.as_ref())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(rejoined, source.replace("\r\n", "\n").trim_end());

        let long = "x".repeat(BLOCK_BYTES + 1);
        let text = PastedText::decode(format!("a\n{long}\nb").as_bytes());
        assert_eq!(text.lines, 3);
        assert_eq!(
            text.blocks.iter().map(|b| b.as_ref()).collect::<Vec<_>>(),
            ["a", long.as_str(), "b"]
        );

        let text = PastedText::decode(b"ok \xff end");
        assert_eq!(text.blocks[0].as_ref(), "ok \u{fffd} end");
    }

    #[test]
    fn pill_label_names_the_paste_and_its_line_count() {
        assert_eq!(pill_label(2, None), "Pasted text 2");
        assert_eq!(pill_label(2, Some(1)), "Pasted text 2 · 1 line");
        assert_eq!(pill_label(3, Some(312)), "Pasted text 3 · 312 lines");
    }
}
