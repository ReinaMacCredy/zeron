use std::path::PathBuf;
use std::rc::Rc;

use gpui::{
    App, AsyncApp, ClickEvent, Context, Entity, FocusHandle, ObjectFit, PathPromptOptions, Render,
    SharedString, StyledImage, Task, Window, div, img, prelude::*, px,
};

use crate::attachments::{self, AttachmentSnapshot, LightboxSet, PreviewImage, StagedAttachment};
use crate::composer::ComposerInputEvent;
use crate::icons::{self, icon};
use crate::state::AppState;
use crate::theme::Theme;

pub(crate) const SPEC_THUMB: (f32, f32) = (72.0, 52.0);
pub(crate) const ROW_THUMB: (f32, f32) = (30.0, 22.0);
fn device(state: &AppState) -> String {
    state.local_device_id.clone().unwrap_or_default()
}
pub(crate) fn pasted(event: &ComposerInputEvent, cx: &mut App) -> Option<Vec<StagedAttachment>> {
    match event {
        ComposerInputEvent::PastedImages(images) => Some(
            images
                .iter()
                .cloned()
                .map(attachments::stage_clipboard_image)
                .collect(),
        ),
        ComposerInputEvent::PastedPaths(paths) => Some(staged_files(paths, cx)),
        _ => None,
    }
}

pub(crate) fn staged_files(paths: &[PathBuf], cx: &mut App) -> Vec<StagedAttachment> {
    let mut staged = Vec::new();
    for path in paths {
        if attachments::format_by_extension(path).is_none() {
            let name = path.file_name().map_or_else(
                || path.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            );
            crate::toast::error(cx, format!("{name} is not an image."));
            continue;
        }
        match attachments::stage_file(path) {
            Ok(att) => staged.push(att),
            Err(error) => crate::toast::error(cx, error),
        }
    }
    staged
}
pub(crate) fn pick(cx: &mut App) -> Task<Vec<PathBuf>> {
    let rx = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: true,
        prompt: Some("Add image".into()),
    });
    cx.spawn(async move |_| match rx.await {
        Ok(Ok(Some(paths))) => paths,
        _ => Vec::new(),
    })
}
pub(crate) fn upload(
    state: &Entity<AppState>,
    staged: Vec<StagedAttachment>,
    cx: &mut App,
) -> Task<Vec<String>> {
    let (engine, device) = {
        let state = state.read(cx);
        (state.engine().cloned(), device(state))
    };
    cx.spawn(async move |cx: &mut AsyncApp| {
        let Some(engine) = engine else {
            cx.update(|cx| crate::toast::error(cx, "Engine not connected"));
            return Vec::new();
        };
        let mut paths = Vec::new();
        for att in staged.iter().filter(|att| att.image().is_some()) {
            let upload_id = uuid::Uuid::new_v4().to_string();
            match attachments::upload_attachment(
                &engine,
                cx.background_executor(),
                None,
                &upload_id,
                att,
                None,
            )
            .await
            {
                Ok(path) => {
                    attachments::seed_staged(&device, &path, att);
                    paths.push(path);
                }
                Err(error) => {
                    tracing::warn!(name = %att.name, %error, "task image upload failed");
                    let message = format!("Couldn't add {}.", att.name);
                    cx.update(|cx| crate::toast::error(cx, message));
                }
            }
        }
        paths
    })
}

pub(crate) fn append(images: &mut Vec<String>, paths: Vec<String>, cx: &mut App) {
    let room = zeron_doc::TASK_IMAGES_MAX.saturating_sub(images.len());
    if paths.len() > room {
        crate::toast::error(
            cx,
            format!(
                "A task or step holds up to {} images.",
                zeron_doc::TASK_IMAGES_MAX
            ),
        );
    }
    images.extend(paths.into_iter().take(room));
}
fn image_at(
    state: &Entity<AppState>,
    path: &str,
    cx: &mut App,
) -> Option<attachments::CachedAttachmentImage> {
    let (engine, device) = {
        let state = state.read(cx);
        (state.engine().cloned(), device(state))
    };
    if let AttachmentSnapshot::Loaded(image) = attachments::attachment_snapshot(&device, path) {
        return Some(image);
    }
    if attachments::begin_load(&device, path) {
        let path = path.to_string();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let loaded = match engine {
                Some(engine) => {
                    attachments::read_attachment_image(
                        &engine,
                        cx.background_executor(),
                        None,
                        &path,
                        None,
                    )
                    .await
                }
                None => None,
            };
            match loaded {
                Some(loaded) => {
                    attachments::store_loaded(&device, &path, loaded.name.into(), loaded.image)
                }
                None => attachments::store_error(&device, &path),
            }
            cx.update(|cx| cx.refresh_windows());
        })
        .detach();
    }
    None
}

fn missing(state: &Entity<AppState>, path: &str, cx: &App) -> bool {
    let device = device(state.read(cx));
    matches!(
        attachments::attachment_snapshot(&device, path),
        AttachmentSnapshot::Error { .. }
    )
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn strip<V: 'static>(
    key: &str,
    paths: &[String],
    (width, height): (f32, f32),
    state: &Entity<AppState>,
    theme: &Theme,
    open: impl Fn(&mut V, usize, &mut Window, &mut Context<V>) + Clone + 'static,
    remove: Option<impl Fn(&mut V, usize, &mut Window, &mut Context<V>) + Clone + 'static>,
    cx: &mut Context<V>,
) -> gpui::Div {
    let large = height >= SPEC_THUMB.1;
    let radius = if large { 9.0 } else { 6.0 };
    let mut row = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(if large { 8.0 } else { 4.0 }));
    for (ix, path) in paths.iter().enumerate() {
        let image = image_at(state, path, cx);
        let name = path.rsplit('/').next().unwrap_or(path).to_string();
        let selector = format!("{key}-image-{ix}");
        let group = SharedString::from(selector.clone());
        let open = open.clone();
        let mut frame = div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector.clone())
            .group(group.clone())
            .role(gpui::Role::Button)
            .aria_label(format!("Open image {} of {}", ix + 1, paths.len()))
            .relative()
            .flex_none()
            .w(px(width))
            .h(px(height))
            .rounded(px(radius))
            .overflow_hidden()
            .border_1()
            .border_color(theme.border)
            .bg(theme.wash(0.06))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|s| s.border_color(theme.text_faint))
            .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                cx.stop_propagation();
                open(view, ix, window, cx);
            }));
        frame = match image {
            Some(image) => frame.child(
                img(image.image)
                    .w(px(width - 2.0))
                    .h(px(height - 2.0))
                    .rounded(px(radius - 1.0))
                    .object_fit(ObjectFit::Cover),
            ),
            None if missing(state, path, cx) => frame
                .tooltip(crate::settings::widgets::text_tooltip(format!(
                    "{name} could not be loaded"
                )))
                .child(
                    icon(icons::GALLERY)
                        .size(px(height * 0.4))
                        .text_color(theme.text_faint),
                ),
            None => frame,
        };
        if let Some(remove) = remove.clone() {
            let selector = format!("{key}-remove-{ix}");
            frame = frame.child(
                div()
                    .id(SharedString::from(selector.clone()))
                    .debug_selector(move || selector.clone())
                    .role(gpui::Role::Button)
                    .aria_label(format!("Remove image {}", ix + 1))
                    .absolute()
                    .top(px(3.0))
                    .right(px(3.0))
                    .size(px(17.0))
                    .rounded_full()
                    .bg(gpui::black().opacity(0.78))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .opacity(0.0)
                    .group_hover(group, |s| s.opacity(1.0))
                    .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        remove(view, ix, window, cx);
                    }))
                    .child(icon(icons::CLOSE).size(px(10.0)).text_color(gpui::white())),
            );
        }
        row = row.child(frame);
    }
    row
}
pub(crate) fn add_tile(id: &str, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    let selector = id.to_string();
    div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector.clone())
        .role(gpui::Role::Button)
        .aria_label("Add image")
        .flex_none()
        .h(px(SPEC_THUMB.1 - 2.0))
        .px(px(12.0))
        .rounded(px(9.0))
        .border_1()
        .border_dashed()
        .border_color(theme.border)
        .flex()
        .items_center()
        .gap(px(5.0))
        .text_size(crate::typography::ui_rems(12.0))
        .text_color(theme.text_muted)
        .cursor_pointer()
        .hover(|s| s.border_color(theme.text_faint).text_color(theme.text))
        .child("＋ Add image")
}
pub(crate) struct ImageGallery {
    state: Entity<AppState>,
    paths: Vec<String>,
    index: usize,
    preview: Option<PreviewImage>,
    focus: FocusHandle,
    focus_pending: bool,
}

impl ImageGallery {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        Self {
            state,
            paths: Vec::new(),
            index: 0,
            preview: None,
            focus: cx.focus_handle(),
            focus_pending: false,
        }
    }

    pub fn open(&mut self, paths: Vec<String>, index: usize, cx: &mut Context<Self>) {
        if index >= paths.len() {
            return;
        }
        self.paths = paths;
        self.index = index;
        self.preview = None;
        self.focus_pending = true;
        cx.notify();
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.paths.len() as isize;
        if count < 2 {
            return;
        }
        self.index = (self.index as isize + delta).rem_euclid(count) as usize;
        self.preview = None;
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.paths.clear();
        self.preview = None;
        cx.notify();
    }
}

impl Render for ImageGallery {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(path) = self.paths.get(self.index).cloned() else {
            return gpui::Empty.into_any_element();
        };
        if self.preview.is_none()
            && let Some(image) = image_at(&self.state, &path, cx)
        {
            self.preview = Some(PreviewImage::new(image.name, image.image));
        }
        if self.focus_pending {
            self.focus_pending = false;
            window.focus(&self.focus, cx);
        }
        let Some(preview) = self.preview.clone() else {
            return gpui::Empty.into_any_element();
        };
        let weak = cx.entity().downgrade();
        let step_weak = weak.clone();
        let set = LightboxSet {
            index: self.index,
            count: self.paths.len(),
            step: Rc::new(move |delta, _, cx| {
                step_weak
                    .update(cx, |gallery, cx| gallery.step(delta, cx))
                    .ok();
            }),
        };
        attachments::lightbox_in_set(
            window,
            &preview,
            &self.focus,
            set,
            move |_, cx| {
                weak.update(cx, |gallery, cx| gallery.close(cx)).ok();
            },
            cx,
        )
    }
}
