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
        #[cfg(not(target_os = "macos"))]
        let _ = cx;
        Self {
            #[cfg(target_os = "macos")]
            pages: HashMap::new(),
            sources: HashMap::new(),
            used: HashMap::new(),
            use_clock: 0,
            heights: HashMap::new(),
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
            InlineEvent::Height(height) => {
                let height = height.round();
                let previous = self.html_pages.heights.insert(key.clone(), height);
                if previous.is_none_or(|previous| (previous - height).abs() >= 1.0) {
                    self.diagram_layout_changed(&[key], cx);
                }
            }
            InlineEvent::OpenLink(url) => cx.open_url(&url),
            InlineEvent::Crashed => {
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
            .chain(gone.into_iter().take(KEPT_PAGES).map(|(key, _)| key.clone()))
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
            .child(div().truncate().child(title.clone()));
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
                let _ = (html, window);
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
