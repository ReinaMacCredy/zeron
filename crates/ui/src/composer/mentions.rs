use std::ops::Range;
use std::time::Duration;

use gpui::{
    App, Context, Entity, EventEmitter, Focusable as _, Hsla, MouseButton, ObjectFit, SharedString,
    StyledImage as _, Task, Window, div, img, prelude::*, px,
};
use zeron_proto::FileSearchMatch;
use zeron_proto::file_mentions::{local_file_link, local_path_is_safe};
use zeron_proto::task_mentions::{task_mention_link, task_mention_links};
use zeron_rpc::{RpcError, methods};

use super::{
    AttachmentRow, ComposerInput, FileMentionState, MentionScope, MentionToken, background,
    mention_error_message, mention_list_ix, mention_response_is_current, mention_token,
    task_matches, task_scope_pill, task_scope_query, thread_link, thread_matches,
};
use crate::state::AppState;
use crate::theme::Theme;

#[derive(Default)]
pub(crate) struct MentionTarget {
    pub(crate) context: String,
    pub(crate) device: Option<(String, Option<String>)>,
    pub(crate) space: Option<String>,
    pub(crate) workspace: Option<serde_json::Value>,
    pub(crate) attachments: Vec<AttachmentRow>,
    pub(crate) done_tasks: bool,
}

pub(crate) enum MentionMenuEvent {
    Changed,
}

pub(crate) struct MentionMenu {
    state: Entity<AppState>,
    input: Entity<ComposerInput>,
    pub(super) mention: FileMentionState,
    search: Option<Task<()>>,
    scroll: gpui::ScrollHandle,
    bar: crate::popover::MenuScrollbarState,
    attachments: Vec<AttachmentRow>,
}

impl EventEmitter<MentionMenuEvent> for MentionMenu {}

impl MentionMenu {
    pub(crate) fn new(state: Entity<AppState>, input: Entity<ComposerInput>) -> Self {
        Self {
            state,
            input,
            mention: FileMentionState::default(),
            search: None,
            scroll: gpui::ScrollHandle::new(),
            bar: crate::popover::MenuScrollbarState::default(),
            attachments: Vec::new(),
        }
    }

    pub(crate) fn token(&self) -> Option<&MentionToken> {
        self.mention.token.as_ref()
    }

    pub(crate) fn is_open(&self) -> bool {
        self.mention.token.is_some()
    }

    pub(crate) fn is_searching(&self) -> bool {
        self.search.is_some()
    }

    pub(crate) fn has_selection(&self) -> bool {
        self.mention.active.is_some()
    }

    /// Tear down the entire completion lifecycle. Advancing the generation is
    /// important even when the spawned task is dropped: an RPC response may
    /// already be queued for delivery on the UI executor.
    pub(crate) fn reset(
        &mut self,
        dismissed: Option<(Range<usize>, String)>,
        cx: &mut Context<Self>,
    ) {
        let request = self.mention.request.wrapping_add(1);
        self.search = None;
        self.mention = FileMentionState {
            context: self.mention.context.clone(),
            request,
            dismissed,
            ..FileMentionState::default()
        };
        cx.emit(MentionMenuEvent::Changed);
        cx.notify();
    }

    pub(crate) fn refresh(
        &mut self,
        text: &str,
        cursor: usize,
        target: MentionTarget,
        cx: &mut Context<Self>,
    ) {
        let token = mention_token(text, cursor);
        self.attachments = target.attachments;
        if self.mention.context != target.context {
            self.reset(None, cx);
            self.mention.context = target.context;
        }
        let still_dismissed = token.as_ref().is_some_and(|token| {
            self.mention
                .dismissed
                .as_ref()
                .is_some_and(|(range, value)| {
                    token.range == *range && text.get(range.clone()) == Some(value.as_str())
                })
        });
        if still_dismissed {
            self.mention.token = None;
            self.search = None;
            self.changed(cx);
            return;
        }
        self.mention.dismissed = None;
        if token == self.mention.token {
            self.changed(cx);
            return;
        }
        self.mention.request = self.mention.request.wrapping_add(1);
        self.search = None;
        // Refining an open menu keeps the stale rows visible until the new
        // response lands — clearing here made the popup bounce through the
        // skeleton (and a different height) on every keystroke.
        let refining = self.mention.token.is_some() && token.is_some();
        self.mention.token = token.clone();
        if !refining {
            self.mention.results.clear();
            self.mention.active = (!self.attachment_matches().is_empty()).then_some(0);
            // Fresh open: the row stack restarts at the top.
            crate::popover::reset_menu_scroll(&self.scroll, &mut self.bar);
        }
        let task_query = token
            .as_ref()
            .and_then(|token| task_scope_query(&token.query));
        let scoped = task_query.is_some();
        self.mention.error = None;
        self.mention.loading = token.is_some() && !scoped;
        if scoped {
            self.mention.results.clear();
        }
        self.mention.tasks = match &token {
            Some(token) => task_matches(
                &self.state.read(cx).tasks,
                task_query.unwrap_or(&token.query),
                target.space.as_deref(),
                scoped,
                target.done_tasks,
            ),
            None => Vec::new(),
        };
        self.mention.threads = match (&token, &target.device) {
            (Some(_), _) if scoped => Vec::new(),
            (Some(token), Some((device, exclude))) => {
                let state = self.state.read(cx);
                thread_matches(
                    state.visible_chats(),
                    &token.query,
                    MentionScope {
                        device_id: device,
                        space_id: target.space.as_deref(),
                        exclude: exclude.as_deref(),
                    },
                    |id| state.is_draft(id),
                    |chat| match (state.space_for_chat(chat), chat.space_id.as_deref()) {
                        (Some(space), _) => space.display_name().to_string(),
                        (None, None) => "~".to_string(),
                        (None, Some(_)) => "?".to_string(),
                    },
                    |chat| crate::pickers::chat_marks(chat, cx),
                )
            }
            _ => Vec::new(),
        };
        // Threads are known before the file search answers, so the keyboard
        // cursor lands on the first thread now instead of on the first file
        // later (and stays within range when a refinement shrinks the list).
        let count = self.item_count();
        self.mention.active = match self.mention.active {
            Some(active) if active < count => Some(active),
            _ => (count > 0).then_some(0),
        };
        cx.emit(MentionMenuEvent::Changed);
        let Some(token) = token.filter(|_| !scoped) else {
            cx.notify();
            return;
        };
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.mention.loading = false;
            cx.notify();
            return;
        };
        let Some(mut params) = target.workspace else {
            self.mention.loading = false;
            cx.notify();
            return;
        };
        params["query"] = token.query.as_str().into();
        let request = self.mention.request;
        self.search = Some(cx.spawn(async move |this, cx| {
            // A short debounce prevents one full workspace walk per keystroke
            // during normal typing. The generation check below still guards
            // requests that were already in flight when the query changed.
            cx.background_executor()
                .timer(Duration::from_millis(80))
                .await;
            let mut result = engine
                .client()
                .call(methods::SEARCH_FILES, params.clone())
                .await;
            if matches!(result, Err(RpcError::Transport(_)) | Err(RpcError::Closed)) {
                // One retry rides out a cold relay dial to the host device
                // (the diffs pane retries forever; a keystroke-scoped search
                // gets a single second chance).
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                result = engine.client().call(methods::SEARCH_FILES, params).await;
            }
            this.update(cx, |menu, cx| {
                if !mention_response_is_current(&menu.mention, request) {
                    return;
                }
                menu.mention.loading = false;
                match result {
                    Ok(value) => match serde_json::from_value::<Vec<FileSearchMatch>>(value) {
                        Ok(mut results) => {
                            // Search results can include legal filesystem names
                            // that the canonical reference format cannot encode.
                            results.retain(|result| local_path_is_safe(&result.path));
                            menu.mention.error = None;
                            menu.mention.results = results;
                            let count = menu.item_count();
                            menu.mention.active = match menu.mention.active {
                                Some(active) if active < count => Some(active),
                                _ => (count > 0).then_some(0),
                            };
                            // New result set: the row stack restarts at the top.
                            crate::popover::reset_menu_scroll(&menu.scroll, &mut menu.bar);
                        }
                        Err(err) => tracing::warn!(%err, "file mention response decode failed"),
                    },
                    Err(err) => {
                        tracing::warn!(%err, "file mention search failed");
                        menu.mention.results.clear();
                        let count = menu.item_count();
                        menu.mention.active = menu
                            .mention
                            .active
                            .filter(|active| *active < count)
                            .or((count > 0).then_some(0));
                        menu.mention.error = Some(mention_error_message(&err));
                    }
                }
                menu.changed(cx);
            })
            .ok();
        }));
        cx.notify();
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(MentionMenuEvent::Changed);
        cx.notify();
    }

    pub(crate) fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.mention.active =
            crate::popover::menu_step(self.mention.active, self.item_count(), delta);
        if let Some(active) = self.mention.active {
            // Keep the keyboard cursor visible in the scrolled row stack.
            self.scroll.scroll_to_item(mention_list_ix(
                active,
                self.mention.threads.len(),
                self.mention.tasks.len(),
            ));
        }
        self.changed(cx);
    }

    pub(crate) fn dismiss(&mut self, cx: &mut Context<Self>) {
        let dismissed = self.mention.token.as_ref().and_then(|token| {
            self.input
                .read(cx)
                .text()
                .get(token.range.clone())
                .map(|text| (token.range.clone(), text.to_string()))
        });
        self.reset(dismissed, cx);
    }

    fn item_count(&self) -> usize {
        self.mention.threads.len()
            + self.mention.tasks.len()
            + self.attachment_matches().len()
            + self.mention.results.len()
    }

    /// Staged attachments that match the typed `@` query, pinned above file
    /// results. Matching ignores case and spaces, so `@im`, `@image2` and
    /// `@2` all find "Image 2", and `@note` finds "notes.md".
    pub(crate) fn attachment_matches(&self) -> Vec<AttachmentRow> {
        let Some(token) = self
            .mention
            .token
            .as_ref()
            .filter(|token| task_scope_query(&token.query).is_none())
        else {
            return Vec::new();
        };
        let normalize = |text: &str| -> String {
            text.chars()
                .filter(|c| !c.is_whitespace())
                .flat_map(char::to_lowercase)
                .collect()
        };
        let wanted = normalize(&token.query);
        let mut rows: Vec<_> = self
            .attachments
            .iter()
            .filter(|row| normalize(&row.label).contains(&wanted))
            .cloned()
            .collect();
        rows.sort_by_key(|row| row.index);
        rows
    }

    fn mentioned_task_ids(&self, cx: &App) -> Vec<String> {
        task_mention_links(self.input.read(cx).text())
            .into_iter()
            .map(|link| link.task_id)
            .collect()
    }

    pub(crate) fn accept(&mut self, cx: &mut Context<Self>) {
        let Some(token) = self.mention.token.clone() else {
            return;
        };
        let Some(active) = self.mention.active else {
            return;
        };
        let named = self.mention.threads.len() + self.mention.tasks.len();
        let attachments = self.attachment_matches();
        let link = if let Some(thread) = self.mention.threads.get(active) {
            thread_link(&thread.chat_id, &thread.title)
        } else if let Some(task) = self.mention.tasks.get(active - self.mention.threads.len()) {
            if self.mentioned_task_ids(cx).contains(&task.task_id) {
                return;
            }
            let Some(link) = task_mention_link(&task.task_id, &task.title) else {
                return;
            };
            link
        } else if let Some(row) = attachments.get(active - named) {
            zeron_proto::attachment_mentions::attachment_mention_link(
                row.index,
                row.image.is_none().then_some(row.label.as_str()),
            )
        } else if let Some(result) = self.mention.results.get(active - named - attachments.len()) {
            local_file_link(&result.path, result.is_dir)
        } else {
            return;
        };
        self.input.update(cx, |input, cx| {
            input.replace_mention_link(token.range, &link, cx)
        });
        self.reset(None, cx);
    }

    fn accept_at(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.mention.active = Some(ix);
        self.accept(cx);
    }

    fn on_list_hover(&mut self, hovered: &bool, _window: &mut Window, cx: &mut Context<Self>) {
        if self.bar.set_list_hovered(*hovered) {
            cx.notify();
        }
    }
}

impl crate::popover::ScrollRailHost for MentionMenu {
    fn rail_bar(&mut self) -> &mut crate::popover::MenuScrollbarState {
        &mut self.bar
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.mention.token.is_some().then(|| self.scroll.clone())
    }
}

impl Render for MentionMenu {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = &Theme::of(cx).for_popup();
        let Some(token) = self.mention.token.as_ref() else {
            return div().into_any_element();
        };
        let mut card = crate::popover::completion_card(theme)
            // Completion choices belong to the input. Keep it focused until
            // mouse-up can accept a choice (or while dragging the scrollbar).
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    window.focus(&this.input.focus_handle(cx), cx);
                }),
            )
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.dismiss(cx)));
        let thread_count = self.mention.threads.len();
        let task_count = self.mention.tasks.len();
        let named_count = thread_count + task_count;
        let scoped = task_scope_query(&token.query).is_some();
        let query_empty = token.query.is_empty();
        let attachment_rows = self.attachment_matches();
        let mut rows: Vec<gpui::AnyElement> = Vec::with_capacity(
            named_count + attachment_rows.len() + self.mention.results.len() + 3,
        );
        if thread_count > 0 {
            rows.push(crate::popover::menu_heading(theme, "Threads").into_any_element());
        }
        let statuses: Vec<(zeron_proto::ChatIndicator, Option<usize>)> = {
            let now = chrono::Utc::now();
            let state = self.state.read(cx);
            self.mention
                .threads
                .iter()
                .map(|thread| {
                    let status = state
                        .chats
                        .iter()
                        .find(|chat| chat.id == thread.chat_id)
                        .map_or(zeron_proto::ChatIndicator::Idle, |chat| {
                            state.display_status_for(chat, now)
                        });
                    let ring = zeron_proto::background::sidebar_ring(
                        status,
                        state.session_for(&thread.chat_id),
                    );
                    (status, ring)
                })
                .collect()
        };
        for (ix, thread) in self.mention.threads.iter().enumerate() {
            let selected = self.mention.active == Some(ix);
            let (status, ring) = statuses[ix];
            let status_color = crate::shell::status_dot_color(status, theme);
            let status_label = zeron_proto::chat_mentions::thread_status_label(status);
            let status_badge = match (status_label, ring) {
                (None, Some(count)) => Some(background::sidebar_ring_mark(count, theme)),
                (None, None) => None,
                (Some(label), _) => Some({
                    let glyph: gpui::AnyElement = match (ring, status) {
                        (Some(count), _) => {
                            background::sidebar_ring_mark(count, theme).into_any_element()
                        }
                        (None, zeron_proto::ChatIndicator::Working) => {
                            crate::loaders::mini_glyph_spinner(
                                format!("thread-mention-working-{}", thread.chat_id),
                                2.0,
                                theme.glyph,
                                cx.entity_id(),
                                cx,
                            )
                            .into_any_element()
                        }
                        (None, zeron_proto::ChatIndicator::Completed) => {
                            crate::icons::icon(crate::icons::CHECK)
                                .size(px(11.0))
                                .flex_none()
                                .text_color(status_color)
                                .into_any_element()
                        }
                        _ => div()
                            .size(px(6.0))
                            .flex_none()
                            .rounded_full()
                            .bg(status_color)
                            .into_any_element(),
                    };
                    div()
                        .flex_none()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(4.0))
                        .child(glyph)
                        .child(
                            div()
                                .text_size(crate::typography::ui_rems(10.0))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(status_color)
                                .child(SharedString::from(label)),
                        )
                }),
            };
            rows.push(
                crate::popover::menu_row(theme, selected, format!("thread-mention-result-{ix}"))
                    .id(("thread-mention-result", ix))
                    .debug_selector(|| format!("thread-mention-result-{ix}"))
                    .on_click(cx.listener(move |this, _, _, cx| this.accept_at(ix, cx)))
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.0))
                            .child(crate::pickers::harness_mark_stack(
                                &thread.marks,
                                14.0,
                                theme.text_muted,
                                1.0,
                            ))
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .overflow_hidden()
                                    .truncate()
                                    .text_size(px(13.0))
                                    .text_color(theme.text)
                                    .child(SharedString::from(thread.title.clone())),
                            )
                            .children(status_badge)
                            .child(
                                div()
                                    .flex_none()
                                    .max_w(px(160.0))
                                    .overflow_hidden()
                                    .truncate()
                                    .text_size(px(12.5))
                                    .text_color(theme.text_muted)
                                    .child(SharedString::from(thread.project.clone())),
                            ),
                    )
                    .into_any_element(),
            );
        }
        if task_count > 0 {
            rows.push(crate::popover::menu_heading(theme, "Tasks").into_any_element());
        }
        let task_rows: Vec<(&'static str, Hsla, Option<SharedString>, SharedString)> = {
            let now = chrono::Utc::now();
            let state = self.state.read(cx);
            self.mention
                .tasks
                .iter()
                .map(|task| {
                    let status = state
                        .tasks
                        .iter()
                        .find(|row| row.id == task.task_id)
                        .map_or(zeron_proto::TaskStatus::ToDo, |row| {
                            crate::tasks::rollup(state, row, now).status
                        });
                    let (_, icon, color) = crate::tasks::column_tone(status, theme);
                    let label = (status != zeron_proto::TaskStatus::ToDo)
                        .then(|| SharedString::from(zeron_proto::mission::column_title(status)));
                    let project = task
                        .space_id
                        .as_deref()
                        .and_then(|id| state.spaces.iter().find(|space| space.id == id))
                        .map_or_else(
                            || SharedString::from("No project"),
                            |space| SharedString::from(space.display_name().to_string()),
                        );
                    (icon, color, label, project)
                })
                .collect()
        };
        let mentioned = self.mentioned_task_ids(cx);
        for (task_ix, (task, (icon, color, label, project))) in
            self.mention.tasks.iter().zip(task_rows).enumerate()
        {
            let ix = thread_count + task_ix;
            let added = mentioned.contains(&task.task_id);
            rows.push(
                crate::popover::menu_row(
                    theme,
                    self.mention.active == Some(ix),
                    format!("task-mention-result-{ix}"),
                )
                .id(("task-mention-result", ix))
                .debug_selector(|| format!("task-mention-result-{ix}"))
                .on_click(cx.listener(move |this, _, _, cx| this.accept_at(ix, cx)))
                .child(
                    div()
                        .w_full()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            crate::icons::icon(icon)
                                .size(px(14.0))
                                .flex_none()
                                .text_color(color),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .overflow_hidden()
                                .truncate()
                                .text_size(px(13.0))
                                .text_color(if added { theme.text_muted } else { theme.text })
                                .child(SharedString::from(task.title.clone())),
                        )
                        .child(
                            div()
                                .flex_none()
                                .max_w(px(180.0))
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(4.0))
                                .overflow_hidden()
                                .text_size(px(12.5))
                                .text_color(theme.text_muted)
                                .when(added, |meta| meta.child("added"))
                                .when(!added, |meta| {
                                    meta.child(div().min_w_0().truncate().child(project))
                                        .children(label.map(|label| {
                                            div()
                                                .flex_none()
                                                .text_color(color)
                                                .child(SharedString::from(format!("· {label}")))
                                        }))
                                }),
                        ),
                )
                .into_any_element(),
            );
        }
        // The file half keeps its own states under the thread and task rows:
        // a skeleton while the search is out, the error, or the empty note
        // (only when no thread or task matched either).
        let mut status: Option<gpui::AnyElement> = None;
        if self.mention.loading && self.mention.results.is_empty() && attachment_rows.is_empty() {
            status = Some(crate::popover::skeleton_rows(
                "file-mention-loading",
                theme,
                if named_count > 0 { 1 } else { 3 },
                cx.entity_id(),
                cx,
            ));
        } else if let Some(error) = self
            .mention
            .error
            .clone()
            .filter(|_| attachment_rows.is_empty())
        {
            status = Some(
                div()
                    .px(px(12.0))
                    .py(px(10.0))
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.danger_muted)
                    .child(error)
                    .into_any_element(),
            );
        } else if self.mention.results.is_empty() && attachment_rows.is_empty() {
            if named_count == 0 {
                status = Some(
                    div()
                        .px(px(12.0))
                        .py(px(10.0))
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.text_muted)
                        .child(if scoped {
                            "No matching tasks"
                        } else if query_empty {
                            "No files available"
                        } else {
                            "No matching files, threads or tasks"
                        })
                        .into_any_element(),
                );
            }
        } else {
            if named_count > 0 {
                rows.push(crate::popover::menu_heading(theme, "Files").into_any_element());
            }
            // Staged attachments stay pinned above every file result.
            for (attachment_ix, row) in attachment_rows.iter().enumerate() {
                let ix = named_count + attachment_ix;
                let selected = self.mention.active == Some(ix);
                let (icon, detail) = match &row.image {
                    Some(image) => (
                        img(image.clone())
                            .size(px(16.0))
                            .rounded(px(3.0))
                            .object_fit(ObjectFit::Cover)
                            .into_any_element(),
                        "Attached image",
                    ),
                    None => (
                        crate::file_icons::icon(
                            crate::file_icons::FileIconIdentity::file(&row.label),
                            theme.appearance,
                        )
                        .size(px(16.0))
                        .into_any_element(),
                        "Attached file",
                    ),
                };
                rows.push(
                    crate::popover::menu_row(
                        theme,
                        selected,
                        format!("attachment-mention-result-{ix}"),
                    )
                    .id(("attachment-mention-result", ix))
                    .on_click(cx.listener(move |this, _, _, cx| this.accept_at(ix, cx)))
                    .child(crate::popover::completion_row_content(
                        theme,
                        icon,
                        row.label.clone().into(),
                        detail.into(),
                    ))
                    .into_any_element(),
                );
            }
            let attachment_count = attachment_rows.len();
            for (file_ix, result) in self.mention.results.iter().enumerate() {
                let ix = named_count + attachment_count + file_ix;
                let selected = self.mention.active == Some(ix);
                let (directory, name) = match result.path.rsplit_once('/') {
                    Some((directory, name)) => (directory.to_string(), name.to_string()),
                    None => (String::new(), result.path.clone()),
                };
                rows.push(
                    crate::popover::menu_row(theme, selected, format!("file-mention-result-{ix}"))
                        .id(("file-mention-result", ix))
                        .on_click(cx.listener(move |this, _, _, cx| this.accept_at(ix, cx)))
                        .child(crate::popover::completion_row_content(
                            theme,
                            crate::file_icons::icon(
                                if result.is_dir {
                                    crate::file_icons::FileIconIdentity::directory(
                                        &result.path,
                                        false,
                                    )
                                } else {
                                    crate::file_icons::FileIconIdentity::file(&result.path)
                                },
                                theme.appearance,
                            )
                            .size(px(16.0))
                            .into_any_element(),
                            name.into(),
                            directory.into(),
                        ))
                        .into_any_element(),
                );
            }
        }
        if scoped {
            card = card.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(8.0))
                    .pt(px(4.0))
                    .pb(px(6.0))
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .child(task_scope_pill(theme))
                    .child("Tasks only")
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(crate::typography::ui_rems(11.0))
                            .text_color(theme.text_faint)
                            .child("Delete \"task \" to search everything"),
                    ),
            );
        }
        if !rows.is_empty() {
            // Overflowing rows wheel-scroll inside a bounded viewport; the
            // floating rail mirrors the model-list scrollbar treatment.
            card = card.child(
                crate::popover::menu_scroll_host("mention-scroll-host")
                    .on_hover(cx.listener(Self::on_list_hover))
                    .child(crate::popover::completion_list(
                        "mention-list",
                        &self.scroll,
                        rows,
                    ))
                    .children(crate::popover::rail(self, "mention-scrollbar", theme, cx)),
            );
        }
        card = card.children(status);
        if !scoped && task_count > 0 {
            card = card.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .mt(px(4.0))
                    .px(px(8.0))
                    .pt(px(6.0))
                    .pb(px(2.0))
                    .border_t_1()
                    .border_color(theme.border)
                    .text_size(crate::typography::ui_rems(11.0))
                    .text_color(theme.text_faint)
                    .child(task_scope_pill(theme))
                    .child("searches only tasks"),
            );
        }
        card.into_any_element()
    }
}
