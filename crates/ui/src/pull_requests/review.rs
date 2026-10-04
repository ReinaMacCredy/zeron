use std::collections::{HashMap, HashSet};
use std::ops::Range;

use chrono::Utc;
use gpui::{AnyElement, Context, Entity, SharedString, Subscription, Window, div, prelude::*, px};
use zeron_markdown::parser::BlockTree;
use zeron_proto::{
    ChangeRequestActivityComment, ChangeRequestReviewComment, ChangeRequestReviewEvent,
    ReplyToChangeRequestThreadRequest, SetChangeRequestThreadResolvedRequest,
    SubmitChangeRequestReviewRequest,
};
use zeron_rpc::methods;

use super::actions::{Look, button, icon_button, readable};
use super::code::{CodeEvent, PullRequestCode};
use super::{avatar, relative_time};
use crate::changes::{DiffRow, FileDiff, LineKind};
use crate::composer::{ComposerInput, ComposerInputEvent};
use crate::icons::{self, icon};
use crate::settings::widgets::text_tooltip;
use crate::theme::Theme;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Thread {
    pub(crate) id: String,
    pub(crate) path: String,
    pub(crate) line: Option<u64>,
    pub(crate) left: bool,
    pub(crate) resolved: bool,
    pub(crate) outdated: bool,
    pub(crate) comments: Vec<ChangeRequestActivityComment>,
}

/// What the page knows that the Code tab needs to place and gate conversations.
#[derive(Clone, Default)]
pub(crate) struct ReviewContext {
    pub(crate) threads: Vec<Thread>,
    pub(crate) can_comment: bool,
    pub(crate) can_resolve: bool,
    pub(crate) verdicts: Vec<ChangeRequestReviewEvent>,
}

/// Review-thread comments grouped into their threads, in the order the host listed them.
pub(crate) fn threads_of(comments: &[ChangeRequestActivityComment]) -> Vec<Thread> {
    let mut threads: Vec<Thread> = Vec::new();
    for comment in comments {
        let (Some(id), Some(path)) = (comment.thread_id.as_ref(), comment.path.as_ref()) else {
            continue;
        };
        match threads.iter_mut().find(|thread| &thread.id == id) {
            Some(thread) => thread.comments.push(comment.clone()),
            None => threads.push(Thread {
                id: id.clone(),
                path: path.clone(),
                line: comment.line.filter(|line| *line > 0),
                left: comment.left_side,
                resolved: comment.thread_resolved,
                outdated: comment.outdated,
                comments: vec![comment.clone()],
            }),
        }
    }
    threads
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pending {
    id: u64,
    path: String,
    left: bool,
    line: u64,
    body: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Annotation {
    file: usize,
    left: bool,
    line: u64,
    threads: Vec<usize>,
    pending: Vec<usize>,
    draft: bool,
}

/// The anchors a diff line can carry a note on: an added line on the right, a deleted one on
/// the left, a context line on either.
pub(crate) fn line_anchors(kind: LineKind, old: Option<u32>, new: Option<u32>) -> Vec<(bool, u64)> {
    match kind {
        LineKind::Add => new
            .map(|line| vec![(false, line as u64)])
            .unwrap_or_default(),
        LineKind::Del => old
            .map(|line| vec![(true, line as u64)])
            .unwrap_or_default(),
        LineKind::Context => [
            new.map(|line| (false, line as u64)),
            old.map(|line| (true, line as u64)),
        ]
        .into_iter()
        .flatten()
        .collect(),
        LineKind::Meta => Vec::new(),
    }
}

fn file_has_line(file: &FileDiff, left: bool, line: u64) -> bool {
    file.hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .any(|diff_line| {
            let number = if left {
                diff_line.old_no
            } else {
                diff_line.new_no
            };
            number.map(u64::from) == Some(line)
        })
}

/// Annotations per (file, side, line) and the threads no loaded line can hold. Nothing is placed
/// under a commit scope: comments anchor to the whole change (t3code CT:424-444).
pub(crate) fn place(
    files: &[FileDiff],
    threads: &[Thread],
    pending: &[Pending],
    draft: Option<(usize, bool, u64)>,
    commit_scoped: bool,
) -> (Vec<Annotation>, Vec<usize>) {
    let mut annotations: Vec<Annotation> = Vec::new();
    let slot = |annotations: &mut Vec<Annotation>, file: usize, left: bool, line: u64| -> usize {
        match annotations
            .iter()
            .position(|note| note.file == file && note.left == left && note.line == line)
        {
            Some(at) => at,
            None => {
                annotations.push(Annotation {
                    file,
                    left,
                    line,
                    ..Default::default()
                });
                annotations.len() - 1
            }
        }
    };
    let mut orphans = Vec::new();
    for (index, thread) in threads.iter().enumerate() {
        let placed = (!commit_scoped)
            .then_some(())
            .and(thread.line)
            .and_then(|line| {
                let file = files.iter().position(|file| {
                    file.path == thread.path && file_has_line(file, thread.left, line)
                })?;
                Some((file, line))
            });
        match placed {
            Some((file, line)) => {
                let at = slot(&mut annotations, file, thread.left, line);
                annotations[at].threads.push(index);
            }
            None => orphans.push(index),
        }
    }
    if !commit_scoped {
        for (index, note) in pending.iter().enumerate() {
            if let Some(file) = files.iter().position(|file| file.path == note.path) {
                let at = slot(&mut annotations, file, note.left, note.line);
                annotations[at].pending.push(index);
            }
        }
        if let Some((file, left, line)) = draft {
            let at = slot(&mut annotations, file, left, line);
            annotations[at].draft = true;
        }
    }
    (annotations, orphans)
}

/// Inserts one card row after the first line row carrying each annotation's anchor.
pub(crate) fn insert_cards(
    files: &[FileDiff],
    rows: Vec<DiffRow>,
    annotations: &[Annotation],
) -> (Vec<DiffRow>, Vec<Range<usize>>) {
    let mut out = Vec::with_capacity(rows.len());
    let mut ranges: Vec<Range<usize>> = Vec::new();
    let mut emitted: HashSet<usize> = HashSet::new();
    for row in rows {
        if let DiffRow::FileHeader { .. } = row {
            if let Some(last) = ranges.last_mut() {
                last.end = out.len();
            }
            ranges.push(out.len()..out.len());
        }
        out.push(row);
        let anchors: Vec<(usize, bool, u64)> = match row {
            DiffRow::Line {
                file, hunk, line, ..
            } => files
                .get(file as usize)
                .and_then(|f| f.hunks.get(hunk as usize))
                .and_then(|h| h.lines.get(line as usize))
                .map(|l| {
                    line_anchors(l.kind, l.old_no, l.new_no)
                        .into_iter()
                        .map(|(left, number)| (file as usize, left, number))
                        .collect()
                })
                .unwrap_or_default(),
            DiffRow::SplitLine {
                file,
                hunk,
                left,
                right,
            } => {
                let lines = files
                    .get(file as usize)
                    .and_then(|f| f.hunks.get(hunk as usize))
                    .map(|h| &h.lines);
                [left, right]
                    .into_iter()
                    .flatten()
                    .filter_map(|slot| lines.and_then(|lines| lines.get(slot as usize)))
                    .flat_map(|l| line_anchors(l.kind, l.old_no, l.new_no))
                    .map(|(left, number)| (file as usize, left, number))
                    .collect()
            }
            _ => Vec::new(),
        };
        for (file, left, line) in anchors {
            if let Some(at) = annotations
                .iter()
                .position(|note| note.file == file && note.left == left && note.line == line)
                && emitted.insert(at)
            {
                out.push(DiffRow::CommentCard {
                    file: file as u32,
                    card: at as u32,
                });
            }
        }
    }
    if let Some(last) = ranges.last_mut() {
        last.end = out.len();
    }
    (out, ranges)
}

struct Editor {
    input: Entity<ComposerInput>,
    _sub: Subscription,
}

struct Draft {
    file: usize,
    path: String,
    left: bool,
    line: u64,
    editor: Editor,
}

#[derive(Default)]
pub(crate) struct ReviewState {
    context: ReviewContext,
    bodies: HashMap<String, BlockTree>,
    pending: Vec<Pending>,
    next_id: u64,
    draft: Option<Draft>,
    pub(crate) annotations: Vec<Annotation>,
    orphans: Vec<usize>,
    hover: Option<(usize, bool, u64)>,
    open: bool,
    summary: Option<Editor>,
    submitting: bool,
    toggled_threads: HashSet<String>,
    reply: Option<(String, Editor)>,
    busy: bool,
    offdiff_open: bool,
}

impl PullRequestCode {
    pub(crate) fn set_review(&mut self, context: ReviewContext, cx: &mut Context<Self>) {
        self.review.bodies = context
            .threads
            .iter()
            .flat_map(|thread| &thread.comments)
            .map(|comment| {
                (
                    comment.id.clone(),
                    crate::markdown::parse_full(&super::detail::strip_html(&comment.body)),
                )
            })
            .collect();
        self.review.context = context;
        self.reflatten_all(cx);
    }

    pub(crate) fn place_review(&mut self, rows: Vec<DiffRow>) -> (Vec<DiffRow>, Vec<Range<usize>>) {
        let draft = self
            .review
            .draft
            .as_ref()
            .map(|draft| (draft.file, draft.left, draft.line));
        let (annotations, orphans) = place(
            &self.files,
            &self.review.context.threads,
            &self.review.pending,
            draft,
            self.commit.is_some(),
        );
        let placed = insert_cards(&self.files, rows, &annotations);
        self.review.annotations = annotations;
        self.review.orphans = orphans;
        placed
    }

    fn can_comment_lines(&self) -> bool {
        self.review.context.can_comment && self.commit.is_none() && self.review.draft.is_none()
    }

    pub(crate) fn review_dismiss(&mut self, cx: &mut Context<Self>) -> bool {
        if self.review.reply.take().is_some() {
            cx.notify();
            return true;
        }
        if self.review.draft.take().is_some() {
            self.reflatten_all(cx);
            return true;
        }
        if self.review.open {
            self.review.open = false;
            cx.notify();
            return true;
        }
        false
    }

    fn begin_draft(
        &mut self,
        file: usize,
        left: bool,
        line: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self.files.get(file).map(|file| file.path.clone()) else {
            return;
        };
        let input = cx.new(|cx| ComposerInput::new("Add a comment…", cx));
        let sub = cx.subscribe(&input, |code, _, event, cx| match event {
            ComposerInputEvent::ModifiedSubmitted => code.add_to_review(cx),
            ComposerInputEvent::Edited => cx.notify(),
            _ => {}
        });
        let handle = gpui::Focusable::focus_handle(input.read(cx), cx);
        window.focus(&handle, cx);
        self.review.draft = Some(Draft {
            file,
            path,
            left,
            line,
            editor: Editor { input, _sub: sub },
        });
        self.review.hover = None;
        self.reflatten_all(cx);
    }

    fn add_to_review(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = &self.review.draft else {
            return;
        };
        let body = draft.editor.input.read(cx).text().trim().to_string();
        if body.is_empty() {
            return;
        }
        self.review.next_id += 1;
        let note = Pending {
            id: self.review.next_id,
            path: draft.path.clone(),
            left: draft.left,
            line: draft.line,
            body,
        };
        self.review.pending.push(note);
        self.review.draft = None;
        self.reflatten_all(cx);
    }

    fn discard_pending(&mut self, id: u64, cx: &mut Context<Self>) {
        self.review.pending.retain(|note| note.id != id);
        self.reflatten_all(cx);
    }

    fn summary_editor(&mut self, cx: &mut Context<Self>) -> Entity<ComposerInput> {
        if let Some(editor) = &self.review.summary {
            return editor.input.clone();
        }
        let input = cx.new(|cx| ComposerInput::new("Summarize your review (optional)", cx));
        let sub = cx.subscribe(&input, |_, _, event, cx| {
            if matches!(event, ComposerInputEvent::Edited) {
                cx.notify();
            }
        });
        self.review.summary = Some(Editor {
            input: input.clone(),
            _sub: sub,
        });
        input
    }

    fn submit_review(&mut self, event: ChangeRequestReviewEvent, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        if self.review.submitting {
            return;
        }
        let body = self
            .review
            .summary
            .as_ref()
            .map(|editor| editor.input.read(cx).text().to_string())
            .unwrap_or_default();
        let submitted: Vec<Pending> = self.review.pending.clone();
        self.review.submitting = true;
        let params = serde_json::to_value(SubmitChangeRequestReviewRequest {
            host: self.key.host.clone(),
            repository: self.key.repository.clone(),
            number: self.key.number,
            event,
            body: body.clone(),
            comments: submitted
                .iter()
                .map(|note| ChangeRequestReviewComment {
                    path: note.path.clone(),
                    line: note.line,
                    left_side: note.left,
                    body: note.body.clone(),
                })
                .collect(),
        })
        .unwrap_or(serde_json::Value::Null);
        cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::SUBMIT_CHANGE_REQUEST_REVIEW, params)
                .await;
            this.update(cx, |code, cx| {
                code.review.submitting = false;
                match result {
                    Ok(_) => {
                        let sent: HashSet<u64> = submitted.iter().map(|note| note.id).collect();
                        code.review.pending.retain(|note| !sent.contains(&note.id));
                        if let Some(editor) = &code.review.summary
                            && editor.input.read(cx).text() == body
                        {
                            let input = editor.input.clone();
                            input.update(cx, |input, cx| input.set_text("", cx));
                        }
                        code.review.open = false;
                        cx.emit(CodeEvent::Toast(
                            match event {
                                ChangeRequestReviewEvent::Comment => "Review submitted",
                                ChangeRequestReviewEvent::Approve => "Pull request approved",
                                ChangeRequestReviewEvent::RequestChanges => "Changes requested",
                            }
                            .into(),
                            None,
                        ));
                        cx.emit(CodeEvent::Refresh);
                        code.reflatten_all(cx);
                    }
                    Err(error) => cx.emit(CodeEvent::Toast(
                        "The review could not be submitted".into(),
                        Some(readable(&error.to_string(), "The host refused it.")),
                    )),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn start_reply(&mut self, thread: String, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| ComposerInput::new("Reply", cx));
        let sub = cx.subscribe(&input, |code, _, event, cx| match event {
            ComposerInputEvent::ModifiedSubmitted => code.send_reply(cx),
            ComposerInputEvent::Edited => cx.notify(),
            _ => {}
        });
        let handle = gpui::Focusable::focus_handle(input.read(cx), cx);
        window.focus(&handle, cx);
        self.review.reply = Some((thread, Editor { input, _sub: sub }));
        cx.notify();
    }

    fn send_reply(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let Some((thread, editor)) = &self.review.reply else {
            return;
        };
        let body = editor.input.read(cx).text().to_string();
        if body.trim().is_empty() || self.review.busy {
            return;
        }
        self.review.busy = true;
        let params = serde_json::to_value(ReplyToChangeRequestThreadRequest {
            host: self.key.host.clone(),
            repository: self.key.repository.clone(),
            number: self.key.number,
            thread_id: thread.clone(),
            body,
        })
        .unwrap_or(serde_json::Value::Null);
        cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::REPLY_TO_CHANGE_REQUEST_THREAD, params)
                .await;
            this.update(cx, |code, cx| {
                code.review.busy = false;
                match result {
                    Ok(_) => {
                        code.review.reply = None;
                        cx.emit(CodeEvent::Refresh);
                    }
                    Err(error) => cx.emit(CodeEvent::Toast(
                        "Reply could not be posted".into(),
                        Some(readable(&error.to_string(), "The host refused it.")),
                    )),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn set_resolved(&mut self, thread: String, resolved: bool, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        if self.review.busy {
            return;
        }
        self.review.busy = true;
        let params = serde_json::to_value(SetChangeRequestThreadResolvedRequest {
            host: self.key.host.clone(),
            repository: self.key.repository.clone(),
            number: self.key.number,
            thread_id: thread,
            resolved,
        })
        .unwrap_or(serde_json::Value::Null);
        cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::SET_CHANGE_REQUEST_THREAD_RESOLVED, params)
                .await;
            this.update(cx, |code, cx| {
                code.review.busy = false;
                match result {
                    Ok(_) => cx.emit(CodeEvent::Refresh),
                    Err(error) => cx.emit(CodeEvent::Toast(
                        "The conversation could not be updated".into(),
                        Some(readable(&error.to_string(), "The host refused it.")),
                    )),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// A line row with the `+` adder on hover, for lines the reader can comment on.
    pub(crate) fn with_adder(
        &self,
        row: AnyElement,
        ix: usize,
        file: usize,
        anchor: Option<(bool, u64)>,
        gutter_left: f32,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some((left, line)) = anchor.filter(|_| self.can_comment_lines()) else {
            return row;
        };
        let hovered = self.review.hover == Some((file, left, line));
        div()
            .id(("pr-review-line", ix))
            .relative()
            .w_full()
            .child(row)
            .on_hover(cx.listener(move |code, hovered: &bool, _, cx| {
                let next = hovered.then_some((file, left, line));
                if *hovered || code.review.hover == Some((file, left, line)) {
                    code.review.hover = next;
                    cx.notify();
                }
            }))
            .when(hovered, |el| {
                el.child(
                    div()
                        .id(("pr-review-adder", ix))
                        .role(gpui::Role::Button)
                        .aria_label("Comment on this line")
                        .absolute()
                        .top(px(2.0))
                        .left(px(gutter_left))
                        .size(px(16.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.0))
                        .bg(theme.accent)
                        .cursor_pointer()
                        .on_click(cx.listener(move |code, _, window, cx| {
                            code.begin_draft(file, left, line, window, cx)
                        }))
                        .child(
                            icon(icons::LUCIDE_SQUARE_PLUS)
                                .size(px(12.0))
                                .text_color(theme.bg),
                        ),
                )
            })
            .into_any_element()
    }

    pub(crate) fn render_annotation(
        &self,
        card: usize,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(note) = self.review.annotations.get(card) else {
            return gpui::Empty.into_any_element();
        };
        let threads: Vec<AnyElement> = note
            .threads
            .iter()
            .filter_map(|ix| self.review.context.threads.get(*ix))
            .map(|thread| self.render_thread(thread, theme, window, cx))
            .collect();
        let pending: Vec<AnyElement> = note
            .pending
            .iter()
            .filter_map(|ix| self.review.pending.get(*ix))
            .map(|note| self.render_pending(note, theme, cx))
            .collect();
        let draft = note.draft.then(|| self.render_draft(theme, cx)).flatten();
        div()
            .px(px(12.0))
            .py(px(4.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .font_family(theme.font_sans.clone())
            .text_color(theme.text)
            .children(threads)
            .children(pending)
            .children(draft)
            .into_any_element()
    }

    fn render_thread(
        &self,
        thread: &Thread,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = thread.resolved == self.review.toggled_threads.contains(&thread.id);
        let count = thread.comments.len();
        let status = format!(
            "{} · {count} {}",
            if thread.resolved { "Resolved" } else { "Open" },
            if count == 1 { "comment" } else { "comments" }
        );
        let toggle_id = thread.id.clone();
        let resolve_id = thread.id.clone();
        let resolved = thread.resolved;
        let now = Utc::now();
        let comments: Vec<AnyElement> = if open {
            thread
                .comments
                .iter()
                .map(|comment| {
                    let author = comment
                        .author
                        .as_ref()
                        .map(|author| author.login.clone())
                        .unwrap_or_else(|| "ghost".into());
                    let body = self.review.bodies.get(&comment.id).map(|tree| {
                        let options = crate::markdown::render::RenderOptions::settled(
                            SharedString::from(format!("pr-thread-comment-{}", comment.id)),
                        );
                        crate::markdown::render::render_tree(tree, &options, theme, window, &|_| {
                            None
                        })
                    });
                    div()
                        .min_w_0()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .text_size(px(12.0))
                                .text_color(theme.text_muted)
                                .child(avatar(&author, theme))
                                .child(div().text_color(theme.text).child(author))
                                .child(relative_time(now, comment.created_at)),
                        )
                        .children(
                            body.map(|body| div().mt(px(4.0)).text_size(px(13.0)).child(body)),
                        )
                        .into_any_element()
                })
                .collect()
        } else {
            Vec::new()
        };
        let reply: Option<AnyElement> = open.then(|| match &self.review.reply {
            Some((id, editor)) if *id == thread.id => {
                let empty = editor.input.read(cx).text().trim().is_empty();
                let busy = self.review.busy;
                div()
                    .mt(px(8.0))
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        div()
                            .id("pr-thread-reply-input")
                            .role(gpui::Role::TextInput)
                            .aria_label("Reply to this conversation")
                            .min_h(px(56.0))
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(theme.hairline(0.14))
                            .px(px(10.0))
                            .py(px(6.0))
                            .text_size(px(13.0))
                            .child(editor.input.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.0))
                            .child(
                                button("pr-thread-reply-cancel", "Cancel", Look::Ghost, theme)
                                    .on_click(cx.listener(|code, _, _, cx| {
                                        code.review.reply = None;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                button("pr-thread-reply-send", "Reply", Look::Primary, theme)
                                    .when(empty || busy, |el| el.opacity(0.64))
                                    .when(!empty && !busy, |el| {
                                        el.on_click(
                                            cx.listener(|code, _, _, cx| code.send_reply(cx)),
                                        )
                                    }),
                            ),
                    )
                    .into_any_element()
            }
            _ => {
                let reply_id = thread.id.clone();
                div()
                    .mt(px(8.0))
                    .child(
                        button(
                            SharedString::from(format!("pr-thread-reply-{}", thread.id)),
                            "Reply",
                            Look::Ghost,
                            theme,
                        )
                        .on_click(cx.listener(
                            move |code, _, window, cx| {
                                code.start_reply(reply_id.clone(), window, cx)
                            },
                        )),
                    )
                    .into_any_element()
            }
        });
        div()
            .rounded(px(12.0))
            .border_1()
            .border_color(theme.hairline(0.1))
            .bg(theme.bg)
            .p(px(12.0))
            .text_size(px(13.0))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .text_size(px(12.0))
                    .text_color(theme.text_muted)
                    .child(if thread.resolved {
                        icon(icons::LUCIDE_CIRCLE_CHECK)
                            .size(px(14.0))
                            .text_color(super::Tones::of(theme).open)
                    } else {
                        icon(icons::LUCIDE_CIRCLE_DOT)
                            .size(px(14.0))
                            .text_color(theme.text_muted)
                    })
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "pr-thread-toggle-{}",
                                thread.id
                            )))
                            .role(gpui::Role::Button)
                            .aria_label(SharedString::from(status.clone()))
                            .cursor_pointer()
                            .hover(|style| style.text_color(theme.text))
                            .on_click(cx.listener(move |code, _, _, cx| {
                                if !code.review.toggled_threads.remove(&toggle_id) {
                                    code.review.toggled_threads.insert(toggle_id.clone());
                                }
                                code.reflatten_all(cx);
                            }))
                            .child(status),
                    )
                    .when(thread.outdated, |el| el.child("outdated"))
                    .child(div().flex_1())
                    .when(self.review.context.can_resolve, |el| {
                        el.child(
                            button(
                                SharedString::from(format!("pr-thread-resolve-{}", thread.id)),
                                if resolved { "Unresolve" } else { "Resolve" },
                                Look::Ghost,
                                theme,
                            )
                            .when(self.review.busy, |el| el.opacity(0.64))
                            .on_click(cx.listener(
                                move |code, _, _, cx| {
                                    code.set_resolved(resolve_id.clone(), !resolved, cx)
                                },
                            )),
                        )
                    }),
            )
            .when(open, |el| {
                el.child(
                    div()
                        .mt(px(8.0))
                        .flex()
                        .flex_col()
                        .gap(px(12.0))
                        .children(comments),
                )
                .children(reply)
            })
            .into_any_element()
    }

    fn render_pending(&self, note: &Pending, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let id = note.id;
        div()
            .rounded(px(12.0))
            .border_1()
            .border_dashed()
            .border_color(theme.hairline(0.16))
            .bg(theme.bg)
            .p(px(12.0))
            .text_size(px(13.0))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .text_size(px(12.0))
                    .text_color(theme.text_muted)
                    .child(
                        icon(icons::LUCIDE_MESSAGE_SQUARE)
                            .size(px(14.0))
                            .text_color(theme.text_muted),
                    )
                    .child("Pending — sent when you submit the review")
                    .child(div().flex_1())
                    .child(
                        div()
                            .id(("pr-pending-discard", id as usize))
                            .role(gpui::Role::Button)
                            .aria_label("Discard this comment")
                            .size(px(22.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.0))
                            .cursor_pointer()
                            .hover(|style| style.bg(theme.ink(0.06)))
                            .on_click(
                                cx.listener(move |code, _, _, cx| code.discard_pending(id, cx)),
                            )
                            .child(
                                icon(icons::LUCIDE_X)
                                    .size(px(14.0))
                                    .text_color(theme.text_muted),
                            ),
                    ),
            )
            .child(div().mt(px(8.0)).child(note.body.clone()))
            .into_any_element()
    }

    fn render_draft(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let draft = self.review.draft.as_ref()?;
        let empty = draft.editor.input.read(cx).text().trim().is_empty();
        Some(
            div()
                .px(px(12.0))
                .py(px(8.0))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .id("pr-review-draft")
                        .role(gpui::Role::TextInput)
                        .aria_label(SharedString::from(format!(
                            "Comment on lines {}:{}",
                            draft.path, draft.line
                        )))
                        .min_h(px(48.0))
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(theme.hairline(0.14))
                        .px(px(10.0))
                        .py(px(6.0))
                        .text_size(px(12.0))
                        .child(draft.editor.input.clone()),
                )
                .child(
                    div()
                        .mt(px(6.0))
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .child(
                            div()
                                .mr_auto()
                                .text_size(px(10.0))
                                .text_color(theme.text_muted.opacity(0.7))
                                .child("⌘ Enter to send"),
                        )
                        .child(
                            button("pr-review-draft-cancel", "Cancel", Look::Ghost, theme)
                                .on_click(cx.listener(|code, _, _, cx| {
                                    code.review.draft = None;
                                    code.reflatten_all(cx);
                                })),
                        )
                        .child(
                            button("pr-review-draft-add", "Add to review", Look::Primary, theme)
                                .when(empty, |el| el.opacity(0.64))
                                .when(!empty, |el| {
                                    el.on_click(
                                        cx.listener(|code, _, _, cx| code.add_to_review(cx)),
                                    )
                                }),
                        ),
                )
                .into_any_element(),
        )
    }

    /// The floating Review pill, or the review bar card when open; absent without verdicts.
    pub(crate) fn render_review_overlay(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let verdicts = self.review.context.verdicts.clone();
        if verdicts.is_empty() {
            return None;
        }
        let count = self.review.pending.len();
        if !self.review.open {
            return Some(
                div()
                    .absolute()
                    .right(px(16.0))
                    .bottom(px(12.0))
                    .child(
                        div()
                            .id("pr-review-pill")
                            .role(gpui::Role::Button)
                            .aria_label("Review")
                            .h(px(28.0))
                            .px(px(10.0))
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .rounded_full()
                            .border_1()
                            .border_color(theme.hairline(0.12))
                            .bg(theme.for_popup().surface_dialog)
                            .shadow_lg()
                            .text_size(px(12.0))
                            .text_color(theme.text)
                            .cursor_pointer()
                            .on_click(cx.listener(|code, _, _, cx| {
                                code.review.open = true;
                                cx.notify();
                            }))
                            .child(
                                icon(icons::LUCIDE_MESSAGE_SQUARE)
                                    .size(px(14.0))
                                    .text_color(theme.text),
                            )
                            .child("Review")
                            .when(count > 0, |el| {
                                el.child(
                                    div()
                                        .size(px(16.0))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded_full()
                                        .bg(theme.accent)
                                        .text_size(px(10.0))
                                        .text_color(theme.bg)
                                        .child(count.to_string()),
                                )
                            }),
                    )
                    .into_any_element(),
            );
        }
        let summary = self.summary_editor(cx);
        let has_summary = !summary.read(cx).text().trim().is_empty();
        let submitting = self.review.submitting;
        let status = if count == 0 {
            "No line comments yet".to_string()
        } else {
            format!(
                "{count} {} pending",
                if count == 1 { "comment" } else { "comments" }
            )
        };
        let verdict_buttons = [
            (
                ChangeRequestReviewEvent::Comment,
                "Comment",
                icons::LUCIDE_MESSAGE_SQUARE,
                Look::Outline,
                "pr-review-comment",
            ),
            (
                ChangeRequestReviewEvent::Approve,
                "Approve",
                icons::LUCIDE_CHECK,
                Look::Primary,
                "pr-review-approve",
            ),
            (
                ChangeRequestReviewEvent::RequestChanges,
                "Request changes",
                icons::LUCIDE_CIRCLE_X,
                Look::Primary,
                "pr-review-request-changes",
            ),
        ]
        .into_iter()
        .filter(|(event, ..)| verdicts.contains(event))
        .map(|(event, label, glyph, look, id)| {
            let enabled = !submitting
                && (event == ChangeRequestReviewEvent::Approve || has_summary || count > 0);
            let tone = if look == Look::Primary {
                theme.bg
            } else {
                theme.text_muted
            };
            icon_button(id, label, look, theme, glyph, tone, 12.0)
                .when(!enabled, |el| el.opacity(0.64))
                .when(enabled, |el| {
                    el.on_click(cx.listener(move |code, _, _, cx| code.submit_review(event, cx)))
                })
        });
        let popup = theme.for_popup();
        Some(
            div()
                .absolute()
                .left(px(12.0))
                .right(px(12.0))
                .bottom(px(12.0))
                .child(
                    div()
                        .id("pr-review-card")
                        .relative()
                        .rounded(px(12.0))
                        .border_1()
                        .border_color(popup.hairline(0.12))
                        .bg(popup.surface_dialog)
                        .shadow_lg()
                        .px(px(16.0))
                        .py(px(12.0))
                        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .id("pr-review-close")
                                .role(gpui::Role::Button)
                                .aria_label("Close review")
                                .absolute()
                                .top(px(8.0))
                                .right(px(8.0))
                                .size(px(24.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(6.0))
                                .cursor_pointer()
                                .hover(|style| style.bg(popup.ink(0.06)))
                                .on_click(cx.listener(|code, _, _, cx| {
                                    code.review.open = false;
                                    cx.notify();
                                }))
                                .child(
                                    icon(icons::LUCIDE_X)
                                        .size(px(14.0))
                                        .text_color(popup.text_muted),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .text_size(px(12.0))
                                .text_color(popup.text_muted)
                                .child(status)
                                .when(count > 0, |el| {
                                    el.child(
                                        button("pr-review-discard", "Discard", Look::Ghost, &popup)
                                            .when(submitting, |el| el.opacity(0.64))
                                            .when(!submitting, |el| {
                                                el.on_click(cx.listener(|code, _, _, cx| {
                                                    code.review.pending.clear();
                                                    code.reflatten_all(cx);
                                                }))
                                            }),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .id("pr-review-summary")
                                .role(gpui::Role::TextInput)
                                .aria_label("Review summary")
                                .mt(px(8.0))
                                .min_h(px(66.0))
                                .rounded(px(8.0))
                                .border_1()
                                .border_color(popup.hairline(0.14))
                                .bg(popup.bg)
                                .px(px(10.0))
                                .py(px(4.0))
                                .text_size(px(13.0))
                                .child(summary),
                        )
                        .child(
                            div()
                                .mt(px(8.0))
                                .flex()
                                .flex_wrap()
                                .justify_end()
                                .gap(px(8.0))
                                .children(verdict_buttons),
                        ),
                )
                .into_any_element(),
        )
    }

    /// Threads no rendered line holds (all of them under a commit scope), grouped by path.
    pub(crate) fn render_offdiff(
        &self,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.review.orphans.is_empty() {
            return None;
        }
        let open = self.review.offdiff_open;
        let count = self.review.orphans.len();
        let label = if self.next_cursor.is_some() {
            "Conversations not on the diff loaded so far"
        } else {
            "Conversations not on the current diff"
        };
        let mut paths: Vec<&str> = Vec::new();
        for ix in &self.review.orphans {
            let path = self.review.context.threads[*ix].path.as_str();
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
        let groups: Vec<AnyElement> = if open {
            paths
                .into_iter()
                .map(|path| {
                    let cards: Vec<AnyElement> = self
                        .review
                        .orphans
                        .iter()
                        .map(|ix| &self.review.context.threads[*ix])
                        .filter(|thread| thread.path == path)
                        .map(|thread| {
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(4.0))
                                .children(thread.line.map(|line| {
                                    div()
                                        .px(px(12.0))
                                        .text_size(px(12.0))
                                        .text_color(theme.text_muted)
                                        .child(format!("Line {line}"))
                                }))
                                .child(self.render_thread(thread, theme, window, cx))
                                .into_any_element()
                        })
                        .collect();
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .child(
                            div()
                                .id(SharedString::from(format!("pr-offdiff-path-{path}")))
                                .px(px(12.0))
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .text_size(px(12.0))
                                .text_color(theme.text_muted)
                                .tooltip(text_tooltip(path.to_owned()))
                                .child(path.to_owned()),
                        )
                        .children(cards)
                        .into_any_element()
                })
                .collect()
        } else {
            Vec::new()
        };
        Some(
            div()
                .flex_none()
                .border_b_1()
                .border_color(theme.hairline(0.06))
                .child(
                    div()
                        .id("pr-offdiff-toggle")
                        .role(gpui::Role::Button)
                        .aria_expanded(open)
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .px(px(16.0))
                        .py(px(8.0))
                        .text_size(px(12.0))
                        .text_color(theme.text_muted)
                        .cursor_pointer()
                        .on_click(cx.listener(|code, _, _, cx| {
                            code.review.offdiff_open = !code.review.offdiff_open;
                            cx.notify();
                        }))
                        .child(label)
                        .child(
                            icon(if open {
                                icons::LUCIDE_CHEVRON_DOWN
                            } else {
                                icons::LUCIDE_CHEVRON_RIGHT
                            })
                            .size(px(14.0))
                            .text_color(theme.text_muted),
                        )
                        .child(count.to_string()),
                )
                .when(open, |el| {
                    el.child(
                        div()
                            .id("pr-offdiff-list")
                            .max_h(px(256.0))
                            .overflow_y_scroll()
                            .px(px(16.0))
                            .pb(px(12.0))
                            .flex()
                            .flex_col()
                            .gap(px(12.0))
                            .children(groups),
                    )
                })
                .into_any_element(),
        )
    }

    pub(crate) fn comment_lock(&self, theme: &Theme) -> Option<AnyElement> {
        (self.commit.is_some() && self.review.context.can_comment).then(|| {
            div()
                .id("pr-code-comment-lock")
                .aria_label("Line comments are written from the whole change")
                .tooltip(text_tooltip(
                    "A comment is anchored to the whole change, so switch to All commits to write one.",
                ))
                .child(
                    icon(icons::LUCIDE_MESSAGE_SQUARE_OFF)
                        .size(px(14.0))
                        .text_color(theme.text_muted),
                )
                .into_any_element()
        })
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn file(path: &str) -> FileDiff {
        crate::changes::parse_patch(&format!(
            "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1,3 +1,3 @@\n a\n-b\n+c\n d\n"
        ))
        .remove(0)
    }

    fn comment(
        thread: &str,
        path: &str,
        line: u64,
        left: bool,
        minute: u32,
    ) -> ChangeRequestActivityComment {
        ChangeRequestActivityComment {
            id: format!("{thread}-{minute}"),
            kind: zeron_proto::ChangeRequestCommentKind::ReviewComment,
            author: None,
            body: "note".into(),
            created_at: Utc.with_ymd_and_hms(2026, 10, 1, 0, minute, 0).unwrap(),
            url: None,
            review_state: None,
            path: Some(path.into()),
            line: Some(line),
            thread_id: Some(thread.into()),
            thread_resolved: false,
            outdated: false,
            left_side: left,
            reactions: Vec::new(),
        }
    }

    #[test]
    fn threads_group_and_land_on_their_side_or_off_the_diff() {
        let threads = threads_of(&[
            comment("T1", "a.rs", 2, false, 1),
            comment("T2", "a.rs", 2, true, 2),
            comment("T1", "a.rs", 2, false, 3),
            comment("T3", "a.rs", 40, false, 4),
            comment("T4", "gone.rs", 1, false, 5),
        ]);
        assert_eq!(threads.len(), 4);
        assert_eq!(threads[0].comments.len(), 2);
        let files = vec![file("a.rs")];
        let (notes, orphans) = place(&files, &threads, &[], None, false);
        assert_eq!(orphans, [2, 3]);
        assert_eq!(
            notes
                .iter()
                .map(|note| (note.left, note.line, note.threads.clone()))
                .collect::<Vec<_>>(),
            [(false, 2, vec![0]), (true, 2, vec![1])]
        );
        let (scoped, scoped_orphans) = place(&files, &threads, &[], None, true);
        assert!(scoped.is_empty());
        assert_eq!(scoped_orphans, [0, 1, 2, 3]);
        let rows =
            crate::changes::body_rows(0, &files[0], &[], None, crate::changes::DiffMode::Unified);
        let mut all = vec![DiffRow::FileHeader { file: 0 }];
        all.extend(rows);
        let (placed, ranges) = insert_cards(&files, all, &notes);
        let cards: Vec<usize> = placed
            .iter()
            .enumerate()
            .filter(|(_, row)| matches!(row, DiffRow::CommentCard { .. }))
            .map(|(ix, _)| ix)
            .collect();
        assert_eq!(cards.len(), 2);
        assert_eq!(ranges, [0..placed.len()]);
        assert!(matches!(placed[cards[0] - 1], DiffRow::Line { .. }));
    }

    #[test]
    fn context_lines_take_either_side_and_meta_lines_none() {
        assert_eq!(line_anchors(LineKind::Add, None, Some(3)), [(false, 3)]);
        assert_eq!(line_anchors(LineKind::Del, Some(2), None), [(true, 2)]);
        assert_eq!(
            line_anchors(LineKind::Context, Some(1), Some(4)),
            [(false, 4), (true, 1)]
        );
        assert!(line_anchors(LineKind::Meta, None, None).is_empty());
    }
}
