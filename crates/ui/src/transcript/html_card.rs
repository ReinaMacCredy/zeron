//! The transcript card for an agent-published HTML page (`html_render`):
//! a header over a placeholder the live page is laid on (macOS), sized to
//! the height the page reports.
use super::*;
use crate::html_render::{clamp_height, frame_height, scrolls_inside};

/// Live pages a transcript keeps at once; one scrolled away longest closes
/// first and reloads when it scrolls back.
const MAX_LIVE_PAGES: usize = 6;
/// Pages (text and measured height) kept for rows that left the transcript,
/// so returning to a chat shows them at once instead of reloading.
const KEPT_PAGES: usize = 24;

#[derive(Clone, Copy, PartialEq)]
enum SheetView {
    Live,
    Source,
}

/// A page opened full size over the conversation.
struct Sheet {
    row: SharedString,
    title: SharedString,
    view: SheetView,
    #[cfg(target_os = "macos")]
    page: Option<crate::browser::inline::InlineWebView>,
}

#[derive(Clone)]
enum PageSource {
    Loading,
    Ready(Arc<str>),
    Failed,
}

pub(super) struct HtmlPages {
    #[cfg(target_os = "macos")]
    pages: HashMap<SharedString, crate::browser::inline::InlineWebView>,
    sources: HashMap<SharedString, PageSource>,
    used: HashMap<SharedString, u64>,
    use_clock: u64,
    heights: HashMap<SharedString, f32>,
    sheet: Option<Sheet>,
    sheet_focus: gpui::FocusHandle,
    theme_generation: u32,
    /// The list's viewport as of this render: rows lay out while the list
    /// state is borrowed, so they cannot ask it.
    pub(super) viewport: Bounds<Pixels>,
    #[cfg(target_os = "macos")]
    tx: crate::browser::inline::InlineSender,
    #[cfg(target_os = "macos")]
    _events: Task<()>,
}

impl HtmlPages {
    pub(super) fn new(cx: &mut Context<Transcript>) -> Self {
        #[cfg(target_os = "macos")]
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        #[cfg(target_os = "macos")]
        let events = cx.spawn(async move |this, cx| {
            while let Some((key, event)) = rx.recv().await {
                if this
                    .update(cx, |this, cx| this.on_html_page_event(key, event, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            #[cfg(target_os = "macos")]
            pages: HashMap::new(),
            sources: HashMap::new(),
            used: HashMap::new(),
            use_clock: 0,
            heights: HashMap::new(),
            sheet: None,
            sheet_focus: cx.focus_handle(),
            viewport: Bounds::default(),
            theme_generation: crate::theme::style_generation(),
            #[cfg(target_os = "macos")]
            tx,
            #[cfg(target_os = "macos")]
            _events: events,
        }
    }
}

pub(super) fn html_render_row(entry: &SessionMessageEntry, part: &MessagePart) -> Option<Row> {
    let MessagePart::Tool {
        call,
        html_render: Some(page),
        ..
    } = part
    else {
        return None;
    };
    if !call.is_html_render() {
        return None;
    }
    Some(Row {
        id: format!("{}#html-{}", entry.id, part.id()).into(),
        version: fnv1a(format!("{}\0{}\0{}", page.path, page.title, page.height).as_bytes()),
        turn_start: false,
        kind: RowKind::HtmlRender {
            owner: entry.device_id.clone().into(),
            path: page.path.clone().into(),
            title: page.title.clone().into(),
            height: page.height as f32,
        },
        entry_id: entry.id.clone().into(),
        timestamp: None,
        copy_text: None,
        compact_fold: None,
    })
}

impl Transcript {
    #[cfg(target_os = "macos")]
    fn on_html_page_event(
        &mut self,
        key: SharedString,
        event: crate::browser::inline::InlineEvent,
        cx: &mut Context<Self>,
    ) {
        use crate::browser::inline::InlineEvent;
        match event {
            InlineEvent::Height(_) if key.ends_with(SHEET_SUFFIX) => {}
            InlineEvent::Height(height) => {
                let height = height.round();
                let previous = self.html_pages.heights.insert(key.clone(), height);
                if previous.is_none_or(|previous| (previous - height).abs() >= 1.0) {
                    self.diagram_layout_changed(&[key], cx);
                }
            }
            InlineEvent::OpenLink(url) => cx.open_url(&url),
            InlineEvent::Crashed => {
                if let Some(sheet) = &mut self.html_pages.sheet
                    && key.ends_with(SHEET_SUFFIX)
                {
                    sheet.page = None;
                }
                self.html_pages.pages.remove(&key);
                cx.notify();
            }
        }
    }

    /// Pages whose rows left the transcript are closed.
    /// Rows that left keep their page while it is among the most recently
    /// shown; older ones are released.
    pub(super) fn retain_html_pages(&mut self) {
        let alive: HashSet<&SharedString> = self
            .rows
            .iter()
            .filter(|row| matches!(row.kind, RowKind::HtmlRender { .. }))
            .map(|row| &row.id)
            .collect();
        let mut gone: Vec<(&SharedString, u64)> = self
            .html_pages
            .used
            .iter()
            .filter(|(key, _)| !alive.contains(key))
            .map(|(key, at)| (key, *at))
            .collect();
        gone.sort_by_key(|(_, at)| std::cmp::Reverse(*at));
        let keep: HashSet<SharedString> = alive
            .iter()
            .map(|key| (*key).clone())
            .chain(
                gone.into_iter()
                    .take(KEPT_PAGES)
                    .map(|(key, _)| key.clone()),
            )
            .collect();
        let pages = &mut self.html_pages;
        pages.heights.retain(|key, _| keep.contains(key));
        pages.sources.retain(|key, _| keep.contains(key));
        pages.used.retain(|key, _| keep.contains(key));
        #[cfg(target_os = "macos")]
        pages.pages.retain(|key, _| keep.contains(key));
    }

    fn html_page_source(
        &mut self,
        row_id: &SharedString,
        owner: &str,
        path: &SharedString,
        cx: &mut Context<Self>,
    ) -> PageSource {
        if let Some(source) = self.html_pages.sources.get(row_id) {
            return source.clone();
        }
        self.html_pages
            .sources
            .insert(row_id.clone(), PageSource::Loading);
        let devices = self.generated_attachment_device_ids(owner, cx);
        let local = self.state.read(cx).local_device_id.clone();
        let engine = self.state.read(cx).engine().cloned();
        let (key, path) = (row_id.clone(), path.to_string());
        cx.spawn(async move |this, cx| {
            let mut html = None;
            if let Some(engine) = engine {
                for device in devices {
                    let target = (local.as_deref() != Some(device.as_str())).then_some(device);
                    html = crate::attachments::read_attachment_text(
                        &engine,
                        cx.background_executor(),
                        target.as_deref(),
                        &path,
                    )
                    .await;
                    if html.is_some() {
                        break;
                    }
                }
            }
            this.update(cx, |this, cx| {
                let source = html.map_or(PageSource::Failed, |html| PageSource::Ready(html.into()));
                this.html_pages.sources.insert(key, source);
                cx.notify();
            })
            .ok();
        })
        .detach();
        PageSource::Loading
    }

    pub(super) fn render_html_render(
        &mut self,
        row_id: &SharedString,
        owner: &SharedString,
        path: &SharedString,
        title: &SharedString,
        agent_height: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let source = self.html_page_source(row_id, owner, path, cx);
        self.html_pages.use_clock += 1;
        self.html_pages
            .used
            .insert(row_id.clone(), self.html_pages.use_clock);
        let content = self.html_pages.heights.get(row_id).copied();
        let height = frame_height(clamp_height(agent_height), content);
        let header = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .h(px(22.0))
            .text_size(crate::typography::ui_rems(12.0))
            .text_color(theme.text_muted)
            .child(
                crate::icons::icon(crate::icons::GLOBE)
                    .size(px(12.0))
                    .text_color(theme.text_faint),
            )
            .child(div().flex_1().min_w_0().truncate().child(title.clone()))
            .child(
                div()
                    .id(SharedString::from(format!("{row_id}-expand")))
                    .debug_selector(|| "html-render-expand".into())
                    .flex_none()
                    .p(px(4.0))
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .hover(|style| style.bg(crate::theme::ink(0.06)))
                    .child(
                        crate::icons::icon(crate::icons::EXPAND_ARROWS)
                            .size(px(13.0))
                            .text_color(theme.text_faint),
                    )
                    .on_click(cx.listener({
                        let row = row_id.clone();
                        let title = title.clone();
                        move |this, _, window, cx| {
                            this.open_html_sheet(row.clone(), title.clone(), window, cx)
                        }
                    })),
            );
        let frame = div()
            .id(SharedString::from(format!("{row_id}-page")))
            .debug_selector(|| "html-render-page".into())
            .relative()
            .w_full()
            .h(px(height));
        let frame = match source {
            PageSource::Ready(html) => {
                #[cfg(target_os = "macos")]
                let frame =
                    frame.child(self.html_page_layer(row_id, &html, height, content, window, cx));
                #[cfg(not(target_os = "macos"))]
                let frame = {
                    let _ = (html, window);
                    frame.child(page_placeholder(
                        "Live pages show on macOS. Expand to read the page source.",
                        &theme,
                    ))
                };
                frame
            }
            PageSource::Loading => frame,
            PageSource::Failed => frame.child(page_placeholder(
                "This page is not available on this device.",
                &theme,
            )),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .w_full()
            .child(header)
            .child(frame)
            .into_any_element()
    }

    #[cfg(target_os = "macos")]
    fn html_page_layer(
        &mut self,
        row_id: &SharedString,
        html: &str,
        height: f32,
        content: Option<f32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let generation = crate::theme::style_generation();
        if self.html_pages.theme_generation != generation {
            self.html_pages.theme_generation = generation;
            let script = crate::html_render::retheme_script(Theme::of(cx), true);
            for page in self.html_pages.pages.values() {
                page.eval(&script);
            }
            if let Some(page) = self.html_pages.sheet.as_ref().and_then(|s| s.page.as_ref()) {
                page.eval(&script);
            }
        }
        if !self.html_pages.pages.contains_key(row_id) {
            if self.html_pages.pages.len() >= MAX_LIVE_PAGES
                && let Some(oldest) = self
                    .html_pages
                    .pages
                    .keys()
                    .min_by_key(|key| self.html_pages.used.get(*key).copied().unwrap_or(0))
                    .cloned()
            {
                self.html_pages.pages.remove(&oldest);
            }
            let html = crate::html_render::inject_bootstrap(html, Theme::of(cx), true);
            match crate::browser::inline::InlineWebView::new(
                window,
                row_id.clone(),
                &html,
                self.html_pages.tx.clone(),
            ) {
                Ok(page) => {
                    self.html_pages.pages.insert(row_id.clone(), page);
                }
                Err(error) => {
                    tracing::warn!(%error, "html render page unavailable");
                    return gpui::Empty.into_any_element();
                }
            }
        }
        if self.html_pages.sheet.is_some() {
            return gpui::Empty.into_any_element();
        }
        let page = self.html_pages.pages[row_id].clone();
        page.set_scrolls(scrolls_inside(height, content));
        let top = if self.doc_override.is_some() {
            0.0
        } else {
            Theme::TITLEBAR_HEIGHT
        };
        let bottom = self.bottom_clearance + if self.show_jump_button { 44.0 } else { 0.0 };
        let region = native_mask(self.html_pages.viewport, top, bottom, self.preview_inset);
        gpui::canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let mask = window.content_mask().bounds.intersect(&region);
                page.sync_on_present(bounds, mask, window);
            },
        )
        .absolute()
        .inset_0()
        .into_any_element()
    }
}

/// The transcript region a native page may cover: the list's clip minus the
/// chrome painted over the list (titlebar, composer stack and jump pill,
/// docked computer preview), which a native view would otherwise hide.
pub(super) fn native_mask(
    list: Bounds<Pixels>,
    top: f32,
    bottom: f32,
    right: f32,
) -> Bounds<Pixels> {
    let top_edge = list.top() + px(top);
    let bottom_edge = (list.bottom() - px(bottom)).max(top_edge);
    let right_edge = (list.right() - px(right)).max(list.left());
    Bounds::from_corners(point(list.left(), top_edge), point(right_edge, bottom_edge))
}

const SHEET_SUFFIX: &str = "#sheet";

/// A readable download name: the title without characters file systems reject.
fn page_file_name(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                ' '
            } else {
                c
            }
        })
        .collect();
    let name: String = cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(120)
        .collect();
    let name = name.trim();
    format!("{}.html", if name.is_empty() { "Page" } else { name })
}

impl Transcript {
    fn open_html_sheet(
        &mut self,
        row: SharedString,
        title: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.html_pages.sheet = Some(Sheet {
            row,
            title,
            view: SheetView::Live,
            #[cfg(target_os = "macos")]
            page: None,
        });
        window.focus(&self.html_pages.sheet_focus, cx);
        cx.notify();
    }

    fn close_html_sheet(&mut self, cx: &mut Context<Self>) {
        self.html_pages.sheet = None;
        cx.notify();
    }

    fn save_html_sheet(&mut self, cx: &mut Context<Self>) {
        let Some(sheet) = &self.html_pages.sheet else {
            return;
        };
        let Some(PageSource::Ready(html)) = self.html_pages.sources.get(&sheet.row).cloned() else {
            return;
        };
        let name = page_file_name(&sheet.title);
        let downloads = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .map(|home| home.join("Downloads"))
            .unwrap_or_else(std::env::temp_dir);
        let chosen = cx.prompt_for_new_path(&downloads, Some(&name));
        cx.spawn(async move |_, _| {
            if let Ok(Ok(Some(path))) = chosen.await
                && let Err(error) = std::fs::write(&path, html.as_bytes())
            {
                tracing::warn!(%error, path = %path.display(), "html page not saved");
            }
        })
        .detach();
    }

    /// The expanded page, over the conversation column and under the
    /// composer. Inline pages hide while it is open: they are native views
    /// and would draw over it.
    pub(super) fn render_html_sheet(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let sheet = self.html_pages.sheet.as_ref()?;
        let (row, title, view) = (sheet.row.clone(), sheet.title.clone(), sheet.view);
        let theme = Theme::of(cx).clone();
        let source = match self.html_pages.sources.get(&row) {
            Some(PageSource::Ready(html)) => html.clone(),
            _ => {
                self.html_pages.sheet = None;
                return None;
            }
        };
        let top = if self.doc_override.is_some() {
            8.0
        } else {
            Theme::TITLEBAR_HEIGHT + 8.0
        };
        let segment = |label: &'static str, id: &'static str, this_view: SheetView| {
            let on = view == this_view;
            div()
                .id(id)
                .debug_selector(move || format!("html-sheet-{id}"))
                .px(px(10.0))
                .py(px(4.0))
                .rounded(px(7.0))
                .cursor_pointer()
                .text_size(crate::typography::ui_rems(12.5))
                .text_color(if on { theme.text } else { theme.text_muted })
                .when(on, |el| el.bg(crate::theme::ink(0.08)))
                .hover(|style| style.text_color(theme.text))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(sheet) = &mut this.html_pages.sheet {
                        sheet.view = this_view;
                    }
                    cx.notify();
                }))
        };
        let header = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .h(px(42.0))
            .px(px(14.0))
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(crate::typography::ui_rems(13.0))
                    .text_color(theme.text)
                    .child(title),
            )
            .child(
                div()
                    .flex()
                    .gap(px(2.0))
                    .child(segment("Live", "live", SheetView::Live))
                    .child(segment("Source", "source", SheetView::Source)),
            )
            .child(
                crate::popover::btn_ghost(&theme, "Save .html", "html-sheet-save")
                    .id("html-sheet-save")
                    .debug_selector(|| "html-sheet-save".into())
                    .on_click(cx.listener(|this, _, _, cx| this.save_html_sheet(cx))),
            )
            .child(
                div()
                    .id("html-sheet-close")
                    .debug_selector(|| "html-sheet-close".into())
                    .p(px(5.0))
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .hover(|style| style.bg(crate::theme::ink(0.06)))
                    .child(
                        crate::icons::icon(crate::icons::CLOSE)
                            .size(px(14.0))
                            .text_color(theme.text_muted),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.close_html_sheet(cx))),
            );
        let body = div().relative().flex_1().min_h_0();
        let body = match view {
            SheetView::Source => body.child(
                div()
                    .id("html-sheet-source")
                    .debug_selector(|| "html-sheet-source".into())
                    .size_full()
                    .overflow_y_scroll()
                    .p(px(16.0))
                    .font_family(theme.font_mono.clone())
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text)
                    .child(source.to_string()),
            ),
            SheetView::Live => {
                #[cfg(target_os = "macos")]
                let body = body.child(self.html_sheet_layer(&row, &source, window, cx));
                #[cfg(not(target_os = "macos"))]
                let body = body.child(page_placeholder(
                    "Live pages show on macOS. Switch to Source to read the page.",
                    &theme,
                ));
                body
            }
        };
        let _ = window;
        Some(
            div()
                .id("html-sheet")
                .debug_selector(|| "html-sheet".into())
                .track_focus(&self.html_pages.sheet_focus)
                .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                    if event.keystroke.key == "escape" {
                        this.close_html_sheet(cx);
                    }
                }))
                .occlude()
                .absolute()
                .top(px(top))
                .bottom(px(self.bottom_clearance + 8.0))
                .left(px(24.0))
                .right(px(24.0 + self.preview_inset))
                .flex()
                .flex_col()
                .rounded(px(14.0))
                .overflow_hidden()
                .bg(crate::popover::surface_bg(&theme))
                .border_1()
                .border_color(theme.border)
                .child(header)
                .child(body)
                .into_any_element(),
        )
    }

    #[cfg(target_os = "macos")]
    fn html_sheet_layer(
        &mut self,
        row: &SharedString,
        source: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let sheet = self.html_pages.sheet.as_mut().expect("sheet is open");
        if sheet.page.is_none() {
            let html = crate::html_render::inject_bootstrap(source, Theme::of(cx), true);
            match crate::browser::inline::InlineWebView::new(
                window,
                format!("{row}{SHEET_SUFFIX}").into(),
                &html,
                self.html_pages.tx.clone(),
            ) {
                Ok(page) => sheet.page = Some(page),
                Err(error) => {
                    tracing::warn!(%error, "html sheet page unavailable");
                    return gpui::Empty.into_any_element();
                }
            }
        }
        let page = sheet.page.clone().expect("sheet page");
        page.set_scrolls(true);
        gpui::canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let mask = window.content_mask().bounds;
                page.sync_on_present(bounds, mask, window);
            },
        )
        .absolute()
        .inset_0()
        .into_any_element()
    }
}

fn page_placeholder(text: &'static str, theme: &Theme) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(10.0))
        .bg(theme.ink(0.035))
        .text_size(crate::typography::ui_rems(12.5))
        .text_color(theme.text_muted)
        .child(text)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeron_doc::{MessageRole, MessageStatus};

    fn parse(_: &str, text: &str) -> Arc<BlockTree> {
        Arc::new(parse_full(text))
    }

    fn entry(status: MessageStatus, parts: Vec<MessagePart>) -> SessionMessageEntry {
        SessionMessageEntry {
            id: "m1".into(),
            role: MessageRole::Assistant,
            parts,
            created_at: 0,
            device_id: "dev-a".into(),
            status: Some(status),
            continuation_of: None,
            from_chat_id: None,
            fork_import: false,
            answers_request_id: None,
            duration_ms: Some(4000),
        }
    }

    fn render_part(page: Option<zeron_proto::html_render::HtmlRenderRef>) -> MessagePart {
        MessagePart::Tool {
            id: "p-html".into(),
            call: ToolCall::Mcp {
                server: "zeron".into(),
                tool: "html_render".into(),
                input: None,
            },
            is_error: false,
            resolved: true,
            output: None,
            diff: None,
            output_ref: None,
            output_bytes: None,
            diff_ref: None,
            diff_stats: None,
            subagent_ref: None,
            subagent_status: None,
            subagent_tail: None,
            html_render: page,
        }
    }

    fn page() -> zeron_proto::html_render::HtmlRenderRef {
        zeron_proto::html_render::HtmlRenderRef {
            path: "/u/html/c1/ab.html".into(),
            title: "Chart".into(),
            height: 420,
        }
    }

    fn exec(id: &str) -> MessagePart {
        MessagePart::Tool {
            id: id.into(),
            call: ToolCall::Exec {
                command: "ls".into(),
            },
            is_error: false,
            resolved: true,
            output: None,
            diff: None,
            output_ref: None,
            output_bytes: None,
            diff_ref: None,
            diff_stats: None,
            subagent_ref: None,
            subagent_status: None,
            subagent_tail: None,
            html_render: None,
        }
    }

    fn text(id: &str, text: &str) -> MessagePart {
        MessagePart::Text {
            id: id.into(),
            text: text.into(),
        }
    }

    fn kinds(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|row| match &row.kind {
                RowKind::HtmlRender { title, .. } => format!("page:{title}"),
                RowKind::ToolGroup { compact_shell, .. } => {
                    format!("tools{}", if *compact_shell { ":fold" } else { "" })
                }
                RowKind::Markdown { .. } | RowKind::LiveMarkdown { .. } => "text".into(),
                _ => "other".into(),
            })
            .filter(|kind| kind != "other")
            .collect()
    }

    #[test]
    fn a_settled_turn_shows_the_page_between_the_fold_and_the_reply() {
        let parts = vec![
            exec("p1"),
            render_part(Some(page())),
            exec("p2"),
            text("t1", "Bars are tokens."),
        ];
        let open = rows_for_entry(
            &entry(MessageStatus::Complete, parts.clone()),
            false,
            false,
            &mut parse,
        );
        assert_eq!(kinds(&open), ["tools", "page:Chart", "tools", "text"]);
        let settled = rows_for_entry(
            &entry(MessageStatus::Complete, parts.clone()),
            false,
            true,
            &mut parse,
        );
        let visible: Vec<Row> = settled
            .iter()
            .filter(|row| row.compact_fold.is_none())
            .cloned()
            .collect();
        assert_eq!(kinds(&visible), ["tools:fold", "page:Chart", "text"]);
        assert_eq!(
            settled
                .iter()
                .filter(|row| matches!(row.kind, RowKind::HtmlRender { .. }))
                .count(),
            1,
            "the folded copy of the work never repeats the page"
        );
        let streaming = rows_for_entry(
            &entry(MessageStatus::Streaming, parts),
            false,
            false,
            &mut parse,
        );
        let id = |rows: &[Row]| {
            rows.iter()
                .find(|row| matches!(row.kind, RowKind::HtmlRender { .. }))
                .map(|row| row.id.clone())
        };
        assert_eq!(id(&streaming), id(&settled));
        assert_eq!(id(&settled).as_deref(), Some("m1#html-p-html"));
    }

    #[test]
    fn a_call_without_a_published_page_shows_no_card() {
        let rows = rows_for_entry(
            &entry(
                MessageStatus::Complete,
                vec![render_part(None), text("t1", "done")],
            ),
            false,
            false,
            &mut parse,
        );
        assert!(
            !rows
                .iter()
                .any(|row| matches!(row.kind, RowKind::HtmlRender { .. }))
        );
    }

    #[test]
    fn the_card_carries_the_owner_device_and_page_reference() {
        let rows = rows_for_entry(
            &entry(MessageStatus::Complete, vec![render_part(Some(page()))]),
            false,
            false,
            &mut parse,
        );
        let Some(RowKind::HtmlRender {
            owner,
            path,
            title,
            height,
        }) = rows
            .iter()
            .find(|row| matches!(row.kind, RowKind::HtmlRender { .. }))
            .map(|row| row.kind.clone())
        else {
            panic!("no page row");
        };
        assert_eq!(
            (owner.as_ref(), path.as_ref(), title.as_ref(), height),
            ("dev-a", "/u/html/c1/ab.html", "Chart", 420.0)
        );
    }

    #[test]
    fn native_pages_never_cover_the_titlebar_composer_or_docked_preview() {
        let list = Bounds::from_corners(point(px(0.0), px(0.0)), point(px(800.0), px(900.0)));
        let mask = native_mask(list, 38.0, 160.0, 220.0);
        assert_eq!(mask.top(), px(38.0));
        assert_eq!(mask.bottom(), px(740.0));
        assert_eq!(mask.right(), px(580.0));
        let tiny = native_mask(list, 38.0, 2000.0, 2000.0);
        assert_eq!(tiny.size.height, px(0.0));
        assert_eq!(tiny.size.width, px(0.0));
    }

    #[test]
    fn download_names_drop_characters_file_systems_reject() {
        assert_eq!(page_file_name("Q3: tokens / turn?"), "Q3 tokens turn.html");
        assert_eq!(page_file_name("  "), "Page.html");
        assert_eq!(page_file_name(&"a".repeat(300)).len(), 125);
    }
}
