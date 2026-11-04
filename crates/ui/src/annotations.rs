//! Text annotations over a rendered text surface (a transcript or a Markdown
//! file view): the selection pill, the comment input, the blue highlight, and
//! the numbered margin badges.
//!
//! An annotation IS a staged [`Quote`] in `AppState`; this layer only draws it.
//! The layer sits after the surface's list in the tree, so it reads the text's
//! geometry during its own prepaint, after the list's: everything stays glued
//! to the text while scrolling and never reads list state from a row.

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnimationExt as _, AnyElement, App, AvailableSpace, Bounds, ContentMask, Context, Element,
    ElementId, Entity, EventEmitter, Focusable as _, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, MouseButton, Pixels, Render, SharedString, Size, Style, Subscription,
    WeakEntity, Window, div, point, prelude::*, px, size,
};

use crate::composer::{ComposerInput, ComposerInputEvent};
use crate::markdown::render::{self, AnnotationMark};
use crate::markdown::selection;
use crate::quotes::Quote;
use crate::state::AppState;
use crate::theme::Theme;

const PILL_HEIGHT: f32 = 30.0;
const PILL_RADIUS: f32 = 14.0;
const PILL_SIDE_PAD: f32 = 8.0;
const PILL_GAP: f32 = 8.0;
const EDITOR_WIDTH: f32 = 296.0;
const EDITOR_HEIGHT: f32 = 45.0;
const EDITOR_EDIT_RADIUS: f32 = 17.0;
const EDITOR_GAP: f32 = 7.0;
const ACTION_SIZE: f32 = 27.0;
const BADGE_SIZE: f32 = 30.0;
const BADGE_TAIL_RADIUS: f32 = 4.0;
const BADGE_OVER_LINE: f32 = 28.5;
const TIP_HEIGHT: f32 = 32.0;
const TIP_RADIUS: f32 = 11.0;
const TIP_GAP: f32 = 11.5;
const TIP_MAX_WIDTH: f32 = 280.0;
const TIP_CHARS_PER_LINE: usize = 36;
const EDGE_MARGIN: f32 = 8.0;
const COMMENT_TEXT_SIZE: f32 = 13.0;
const COMMENT_LINE_HEIGHT: f32 = 20.0;

pub(crate) const COMMENT_PLACEHOLDER: &str = "Add an optional comment...";

/// One selected span, with the surface's own key prefix stripped so a reopened
/// file view finds its annotations again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AnchorSpan {
    pub key: String,
    pub range: Range<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Column {
    pub pad_left: f32,
    pub pad_right: f32,
    pub max_width: f32,
}

impl Column {
    /// The right edge of the centered text column inside `bounds`.
    fn right(&self, bounds: Bounds<Pixels>) -> f32 {
        let left = f32::from(bounds.left()) + self.pad_left;
        let right = f32::from(bounds.right()) - self.pad_right;
        let width = (right - left).min(self.max_width);
        left + ((right - left) - width) / 2.0 + width
    }
}

/// Where the host's text may be seen, in the layer's own bounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Host {
    pub column: Column,
    pub top_inset: f32,
    pub bottom_inset: f32,
}

#[derive(Clone, Debug)]
pub(crate) enum AnnotationEvent {
    /// The anchor's row is not rendered: bring it into view.
    RevealRow(Arc<str>),
    /// Move the viewport by this much so the anchor and its input both show.
    ScrollBy(f32),
}

#[derive(Clone, Copy, PartialEq)]
enum Reveal {
    Wanted,
    RowAsked,
    Done,
}

struct Pending {
    text: String,
    spans: Vec<AnchorSpan>,
}

struct Editor {
    quote_id: String,
    input: Entity<ComposerInput>,
    editing: bool,
    generation: u64,
    reveal: Rc<Cell<Reveal>>,
    _events: Subscription,
    _changes: Subscription,
}

pub(crate) struct AnnotationLayer {
    state: Entity<AppState>,
    surface: u64,
    key_prefix: String,
    source: Option<String>,
    chat: Option<String>,
    host: Host,
    pending: Option<Pending>,
    editor: Option<Editor>,
    generation: u64,
    focus_pending: bool,
    hovered_badge: Option<String>,
    pinned_tip: Option<String>,
    _state_changes: Subscription,
}

impl EventEmitter<AnnotationEvent> for AnnotationLayer {}

/// The accent, deep enough to carry white as the recording's badge does; a
/// preset's `accent_strong` can be light enough that it wants dark text.
fn deep_accent(theme: &Theme) -> gpui::Hsla {
    let mut color = theme.accent_strong;
    color.l = color.l.min(0.56);
    color.s = color.s.min(0.55);
    color
}

/// One step off the page and opaque, as the recording's floating surfaces are.
/// Frost's translucent popover tint lands almost on the page color here.
pub(crate) fn surface_fill(theme: &Theme) -> gpui::Hsla {
    crate::theme::flatten(theme.ink(0.08), theme.bg)
}

pub(crate) fn wash_color(theme: &Theme) -> gpui::Hsla {
    deep_accent(theme).opacity(if theme.appearance.is_dark() {
        0.5
    } else {
        0.28
    })
}

impl AnnotationLayer {
    pub(crate) fn new(
        state: Entity<AppState>,
        surface: u64,
        key_prefix: String,
        source: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let state_changes = cx.observe(&state, |this: &mut Self, _, cx| {
            this.take_edit_request(cx);
            cx.notify();
        });
        cx.on_release(move |_, _| render::publish_annotation_marks(surface, Vec::new()))
            .detach();
        Self {
            state,
            surface,
            key_prefix,
            source,
            chat: None,
            host: Host {
                column: Column {
                    pad_left: 0.0,
                    pad_right: 0.0,
                    max_width: f32::MAX,
                },
                top_inset: 0.0,
                bottom_inset: 0.0,
            },
            pending: None,
            editor: None,
            generation: 0,
            focus_pending: false,
            hovered_badge: None,
            pinned_tip: None,
            _state_changes: state_changes,
        }
    }

    pub(crate) fn set_host(&mut self, chat: Option<String>, host: Host, cx: &mut Context<Self>) {
        if self.chat != chat {
            self.commit_editor(false, cx);
            self.pending = None;
            self.hovered_badge = None;
            self.pinned_tip = None;
            self.chat = chat;
            self.take_edit_request(cx);
            cx.notify();
        }
        if self.host != host {
            self.host = host;
            cx.notify();
        }
    }

    /// A selection just settled on this surface: offer the pill.
    pub(crate) fn settle_selection(&mut self, cx: &mut Context<Self>) {
        self.pending = None;
        self.pinned_tip = None;
        if self.chat.is_none() {
            cx.notify();
            return;
        }
        let spans: Vec<AnchorSpan> = selection::selected_spans()
            .into_iter()
            .filter_map(|span| {
                Some(AnchorSpan {
                    key: span.key.strip_prefix(&self.key_prefix)?.to_string(),
                    range: span.range,
                })
            })
            .collect();
        let owned = self.key_prefix.is_empty() && selection::anchor_surface() == self.surface
            || !self.key_prefix.is_empty();
        if owned
            && !spans.is_empty()
            && let Some(text) = selection::selected_text().filter(|text| !text.trim().is_empty())
        {
            self.pending = Some(Pending { text, spans });
        }
        cx.notify();
    }

    fn clear_selection() {
        if let Some(key) = selection::anchor_key() {
            selection::clear_if_owner(&key);
        }
    }

    fn add_to_chat(&mut self, cx: &mut Context<Self>) {
        let (Some(pending), Some(chat)) = (self.pending.take(), self.chat.clone()) else {
            return;
        };
        Self::clear_selection();
        let quote = Quote::annotation(pending.text, self.source.clone());
        let id = quote.id.clone();
        self.state.update(cx, |state, cx| {
            state.add_annotation(&chat, quote, pending.spans);
            cx.notify();
        });
        self.open_editor(id, false, String::new(), cx);
    }

    fn ask_in_side_chat(&mut self, cx: &mut Context<Self>) {
        let (Some(pending), Some(chat)) = (self.pending.take(), self.chat.clone()) else {
            return;
        };
        Self::clear_selection();
        let quote = Quote::annotation(pending.text, self.source.clone());
        self.state.update(cx, |state, cx| {
            state.annotation_requests.side_chat = Some((chat, quote));
            cx.notify();
        });
        cx.notify();
    }

    fn open_editor(
        &mut self,
        quote_id: String,
        editing: bool,
        comment: String,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| {
            let mut input = ComposerInput::new(COMMENT_PLACEHOLDER, cx)
                .with_text_metrics(COMMENT_TEXT_SIZE, COMMENT_LINE_HEIGHT)
                .with_wrapping();
            input.set_text(comment, cx);
            input
        });
        let events = cx.subscribe(&input, |this, _, event: &ComposerInputEvent, cx| {
            if matches!(event, ComposerInputEvent::Submitted) {
                this.commit_editor(true, cx);
            }
        });
        let changes = cx.observe(&input, |_, _, cx| cx.notify());
        self.generation = self.generation.wrapping_add(1);
        self.pending = None;
        self.pinned_tip = None;
        self.hovered_badge = None;
        self.focus_pending = true;
        self.editor = Some(Editor {
            quote_id,
            input,
            editing,
            generation: self.generation,
            reveal: Rc::new(Cell::new(if editing {
                Reveal::Wanted
            } else {
                Reveal::Done
            })),
            _events: events,
            _changes: changes,
        });
        cx.notify();
    }

    /// Keep what was typed: the check, Save, Enter, or a press elsewhere.
    fn commit_editor(&mut self, focus_composer: bool, cx: &mut Context<Self>) {
        let (Some(editor), Some(chat)) = (self.editor.take(), self.chat.clone()) else {
            return;
        };
        let comment = editor.input.read(cx).text().trim().to_string();
        self.state.update(cx, |state, cx| {
            state.set_quote_comment(&chat, &editor.quote_id, comment);
            if focus_composer {
                state.annotation_requests.focus_composer = Some(chat.clone());
            }
            cx.notify();
        });
        cx.notify();
    }

    /// Cancel leaves an edit's comment as it was, its tooltip up as in the
    /// recording. Escape on a new annotation drops it: nothing was saved yet.
    fn cancel_editor(&mut self, cx: &mut Context<Self>) {
        let (Some(editor), Some(chat)) = (self.editor.take(), self.chat.clone()) else {
            return;
        };
        if editor.editing {
            self.pinned_tip = Some(editor.quote_id);
        } else {
            self.state.update(cx, |state, cx| {
                state.remove_quote(&chat, &editor.quote_id);
                cx.notify();
            });
        }
        cx.notify();
    }

    fn delete_editor(&mut self, cx: &mut Context<Self>) {
        let (Some(editor), Some(chat)) = (self.editor.take(), self.chat.clone()) else {
            return;
        };
        self.state.update(cx, |state, cx| {
            state.remove_quote(&chat, &editor.quote_id);
            cx.notify();
        });
        cx.notify();
    }

    /// The card's pencil asks through `AppState`: the composer does not know
    /// which surface drew its annotation.
    fn take_edit_request(&mut self, cx: &mut Context<Self>) {
        let Some(chat) = self.chat.clone() else {
            return;
        };
        let state = self.state.read(cx);
        let Some((request_chat, id)) = state.annotation_requests.edit.clone() else {
            return;
        };
        if request_chat != chat {
            return;
        }
        let Some(quote) = state.quotes(&chat).iter().find(|quote| quote.id == id) else {
            return;
        };
        if quote.source != self.source || state.annotation_anchor(&id).is_none() {
            return;
        }
        let comment = quote.comment.clone().unwrap_or_default();
        self.state.update(cx, |state, _| {
            state.annotation_requests.edit = None;
        });
        self.commit_editor(false, cx);
        self.open_editor(id, true, comment, cx);
    }

    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| selection::selected_text().as_deref() != Some(&pending.text))
        {
            self.pending = None;
        }
        let live = |id: &str| {
            self.chat.as_deref().is_some_and(|chat| {
                self.state
                    .read(cx)
                    .quotes(chat)
                    .iter()
                    .any(|quote| quote.id == id)
            })
        };
        if self
            .editor
            .as_ref()
            .is_some_and(|editor| !live(&editor.quote_id))
        {
            self.editor = None;
        }
        if self.focus_pending {
            self.focus_pending = false;
            if let Some(editor) = &self.editor {
                window.focus(&editor.input.focus_handle(cx), cx);
            }
        }
    }

    fn annotations(&self, cx: &App) -> Vec<(usize, Quote, Vec<AnchorSpan>)> {
        let Some(chat) = self.chat.as_deref() else {
            return Vec::new();
        };
        let state = self.state.read(cx);
        state
            .quotes(chat)
            .iter()
            .filter(|quote| quote.is_annotation())
            .enumerate()
            .filter(|(_, quote)| quote.source == self.source)
            .filter_map(|(ix, quote)| {
                let spans = state.annotation_anchor(&quote.id)?.to_vec();
                Some((ix + 1, quote.clone(), spans))
            })
            .collect()
    }

    fn full_key(&self, span: &AnchorSpan) -> Arc<str> {
        format!("{}{}", self.key_prefix, span.key).into()
    }

    fn publish_marks(&self, theme: &Theme, cx: &App) {
        let lit = wash_color(theme);
        let open = self.editor.as_ref().map(|editor| editor.quote_id.as_str());
        let mut marks = Vec::new();
        for (_, quote, spans) in self.annotations(cx) {
            let lit = (open == Some(quote.id.as_str())).then_some(lit);
            marks.extend(spans.iter().map(|span| AnnotationMark {
                key: self.full_key(span),
                range: span.range.clone(),
                lit,
            }));
        }
        if let Some(first) = self.pending.as_ref().and_then(|p| p.spans.first()) {
            marks.push(AnnotationMark {
                key: self.full_key(first),
                range: first.range.clone(),
                lit: None,
            });
        }
        render::publish_annotation_marks(self.surface, marks);
    }

    fn pill_segment(
        &self,
        id: &'static str,
        label: &'static str,
        popup: &Theme,
        cx: &mut Context<Self>,
        act: fn(&mut Self, &mut Context<Self>),
    ) -> impl IntoElement + use<> {
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .role(gpui::Role::Button)
            .aria_label(label)
            .h_full()
            .flex()
            .items_center()
            .px(px(PILL_SIDE_PAD))
            .cursor_pointer()
            .hover(|style| style.bg(popup.glass_hover()))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                act(this, cx);
            }))
            .child(label)
    }

    fn render_pill(&self, popup: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let card = div()
            .h(px(PILL_HEIGHT))
            .flex()
            .flex_row()
            .items_center()
            .rounded(px(PILL_RADIUS))
            .border_1()
            .border_color(popup.border)
            .bg(surface_fill(popup))
            .overflow_hidden()
            .text_size(crate::typography::ui_rems(COMMENT_TEXT_SIZE))
            .text_color(popup.text)
            .child(self.pill_segment(
                "annotation-add-to-chat",
                "Add to chat",
                popup,
                cx,
                Self::add_to_chat,
            ))
            .child(div().w(px(1.0)).h_full().bg(popup.border))
            .child(self.pill_segment(
                "annotation-ask-side-chat",
                "Ask in side chat",
                popup,
                cx,
                Self::ask_in_side_chat,
            ));
        div()
            .id("annotation-pill")
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(crate::frost::layered(card))
            .into_any_element()
    }

    fn icon_button(
        id: &'static str,
        label: &'static str,
        icon: &'static str,
        color: gpui::Hsla,
        popup: &Theme,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .role(gpui::Role::Button)
            .aria_label(label)
            .flex_none()
            .size(px(ACTION_SIZE))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .cursor_pointer()
            .hover(|style| style.bg(popup.glass_hover()))
            .child(crate::icons::icon(icon).size(px(16.0)).text_color(color))
    }

    fn render_mic(
        &self,
        input: &Entity<ComposerInput>,
        popup: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let active = input.read(cx).dictation_active();
        if !crate::dictation::enabled(cx) && !active {
            return None;
        }
        let input = input.clone();
        let color = if active {
            popup.accent
        } else {
            popup.text_muted
        };
        Some(
            Self::icon_button(
                "annotation-mic",
                "Dictate",
                crate::icons::MICROPHONE,
                color,
                popup,
            )
            .on_click(cx.listener(move |_, _, _, cx| {
                cx.stop_propagation();
                input.update(cx, |input, cx| input.toggle_dictation(cx));
            }))
            .into_any_element(),
        )
    }

    fn render_editor(&self, editor: &Editor, popup: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let field = div()
            .flex_1()
            .min_w_0()
            .text_size(crate::typography::ui_rems(COMMENT_TEXT_SIZE))
            .child(editor.input.clone());
        let mic = self.render_mic(&editor.input, popup, cx);
        let radius = if editor.editing {
            EDITOR_EDIT_RADIUS
        } else {
            EDITOR_HEIGHT / 2.0
        };
        let card = div()
            .w(px(EDITOR_WIDTH))
            .rounded(px(radius))
            .bg(surface_fill(popup))
            .text_color(popup.text);
        let card = if editor.editing {
            card.flex()
                .flex_col()
                .pt(px(13.0))
                .pb(px(11.0))
                .pl(px(17.0))
                .pr(px(13.0))
                .gap(px(12.0))
                .child(field)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .ml(px(-7.5))
                        .child(
                            Self::icon_button(
                                "annotation-delete",
                                "Delete annotation",
                                crate::icons::TRASH_BIN_MINIMALISTIC,
                                popup.text_muted,
                                popup,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.delete_editor(cx);
                            })),
                        )
                        .child(div().flex_1())
                        .children(mic)
                        .child(
                            div()
                                .id("annotation-cancel")
                                .debug_selector(|| "annotation-cancel".to_string())
                                .role(gpui::Role::Button)
                                .aria_label("Cancel")
                                .ml(px(6.0))
                                .h(px(ACTION_SIZE))
                                .px(px(8.0))
                                .flex()
                                .items_center()
                                .rounded_full()
                                .border_1()
                                .border_color(popup.border)
                                .cursor_pointer()
                                .text_size(crate::typography::ui_rems(COMMENT_TEXT_SIZE))
                                .hover(|style| style.bg(popup.glass_hover()))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.cancel_editor(cx);
                                }))
                                .child("Cancel"),
                        )
                        .child(
                            div()
                                .id("annotation-save")
                                .debug_selector(|| "annotation-save".to_string())
                                .role(gpui::Role::Button)
                                .aria_label("Save")
                                .ml(px(6.0))
                                .h(px(ACTION_SIZE))
                                .px(px(8.5))
                                .flex()
                                .items_center()
                                .rounded_full()
                                .bg(popup.solid)
                                .text_color(popup.on_solid)
                                .cursor_pointer()
                                .text_size(crate::typography::ui_rems(COMMENT_TEXT_SIZE))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.commit_editor(true, cx);
                                }))
                                .child("Save"),
                        ),
                )
        } else {
            card.min_h(px(EDITOR_HEIGHT))
                .flex()
                .flex_row()
                .items_center()
                .pl(px(15.0))
                .pr(px(9.0))
                .gap(px(8.5))
                .child(field)
                .children(mic)
                .child(
                    div()
                        .id("annotation-check")
                        .debug_selector(|| "annotation-check".to_string())
                        .role(gpui::Role::Button)
                        .aria_label("Add annotation")
                        .flex_none()
                        .size(px(ACTION_SIZE))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(popup.solid)
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.commit_editor(true, cx);
                        }))
                        .child(
                            crate::icons::icon(crate::icons::CHECK)
                                .size(px(14.0))
                                .text_color(popup.on_solid),
                        ),
                )
        };
        let animated = card.with_animation(
            ElementId::NamedInteger("annotation-editor-in".into(), editor.generation),
            crate::motion::ANNOTATION_EDITOR_IN.animation(),
            |card, t| card.opacity(t),
        );
        div()
            .id("annotation-editor")
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.commit_editor(false, cx)))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    this.cancel_editor(cx);
                }
            }))
            .child(crate::frost::layered(animated))
            .into_any_element()
    }

    fn render_badge(
        &self,
        number: usize,
        quote: &Quote,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = quote.id.clone();
        let selector = format!("annotation-badge-{number}");
        crate::frost::layered(
            div()
                .id(ElementId::NamedInteger(
                    "annotation-badge".into(),
                    number as u64,
                ))
                .debug_selector(move || selector.clone())
                .role(gpui::Role::Image)
                .aria_label(format!("Annotation {number}"))
                .occlude()
                .size(px(BADGE_SIZE))
                .flex()
                .items_center()
                .justify_center()
                .rounded_tl(px(BADGE_SIZE / 2.0))
                .rounded_tr(px(BADGE_SIZE / 2.0))
                .rounded_br(px(BADGE_SIZE / 2.0))
                .rounded_bl(px(BADGE_TAIL_RADIUS))
                .border_1()
                .border_color(gpui::white())
                .bg(deep_accent(theme))
                .text_size(crate::typography::ui_rems(COMMENT_TEXT_SIZE))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(gpui::white())
                .on_hover(cx.listener(move |this, hovered, _, cx| {
                    if *hovered {
                        this.hovered_badge = Some(id.clone());
                    } else if this.hovered_badge.as_deref() == Some(id.as_str()) {
                        this.hovered_badge = None;
                    }
                    cx.notify();
                }))
                .child(number.to_string()),
        )
        .into_any_element()
    }

    fn render_tip(&self, comment: &str, popup: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let wide = comment.chars().count() > TIP_CHARS_PER_LINE;
        let card = div()
            .when(wide, |el| el.w(px(TIP_MAX_WIDTH)))
            .min_h(px(TIP_HEIGHT))
            .flex()
            .items_center()
            .px(px(9.0))
            .py(px(6.0))
            .rounded(px(TIP_RADIUS))
            .border_1()
            .border_color(popup.border)
            .bg(surface_fill(popup))
            .text_size(crate::typography::ui_rems(COMMENT_TEXT_SIZE))
            .line_height(px(COMMENT_LINE_HEIGHT))
            .text_color(popup.text)
            .child(SharedString::from(comment.to_string()));
        let dismiss = cx.weak_entity();
        div()
            .id("annotation-tip")
            .debug_selector(|| "annotation-tip".to_string())
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.pinned_tip = None;
                cx.notify();
            }))
            .child(crate::frost::layered(card))
            .child(
                gpui::canvas(
                    |_, _, _| (),
                    move |_, _, window, _| {
                        let dismiss = dismiss.clone();
                        window.on_key_event(move |_: &gpui::KeyDownEvent, _, _, cx| {
                            dismiss
                                .update(cx, |this: &mut Self, cx| {
                                    if this.pinned_tip.take().is_some() {
                                        cx.notify();
                                    }
                                })
                                .ok();
                        });
                    },
                )
                .absolute()
                .size_0(),
            )
            .into_any_element()
    }
}

impl Render for AnnotationLayer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync(window, cx);
        let theme = Theme::of(cx).clone();
        let popup = theme.for_popup();
        self.publish_marks(&theme, cx);
        let mut items = Vec::new();
        let editing = self.editor.as_ref().map(|editor| editor.quote_id.clone());
        for (number, quote, spans) in self.annotations(cx) {
            let Some(first) = spans.first() else {
                continue;
            };
            let key = self.full_key(first);
            let range = first.range.clone();
            items.push(Item {
                key: key.clone(),
                range: range.clone(),
                place: Place::Badge,
                reveal: None,
                element: self.render_badge(number, &quote, &theme, cx),
            });
            let tip_shown = editing.as_deref() != Some(quote.id.as_str())
                && (self.hovered_badge.as_deref() == Some(quote.id.as_str())
                    || self.pinned_tip.as_deref() == Some(quote.id.as_str()));
            let comment = quote.comment.clone().unwrap_or_default();
            if tip_shown && !comment.is_empty() {
                items.push(Item {
                    key: key.clone(),
                    range: range.clone(),
                    place: Place::OverBadge,
                    reveal: None,
                    element: self.render_tip(&comment, &popup, cx),
                });
            }
            if let Some(editor) = self
                .editor
                .as_ref()
                .filter(|editor| editor.quote_id == quote.id)
            {
                items.push(Item {
                    key,
                    range,
                    place: Place::OverLine(EDITOR_GAP),
                    reveal: Some(editor.reveal.clone()),
                    element: self.render_editor(editor, &popup, cx),
                });
            }
        }
        if let Some(first) = self.pending.as_ref().and_then(|p| p.spans.first()) {
            items.push(Item {
                key: self.full_key(first),
                range: first.range.clone(),
                place: Place::OverLine(PILL_GAP),
                reveal: None,
                element: self.render_pill(&popup, cx),
            });
        }
        Overlay {
            surface: self.surface,
            host: self.host,
            layer: cx.weak_entity(),
            items,
        }
    }
}

enum Place {
    /// Centered on the anchor's first line, this far above it.
    OverLine(f32),
    /// At the column's right edge, just over the anchor's first line.
    Badge,
    /// Centered above where the badge sits.
    OverBadge,
}

struct Item {
    key: Arc<str>,
    range: Range<usize>,
    place: Place,
    reveal: Option<Rc<Cell<Reveal>>>,
    element: AnyElement,
}

struct Overlay {
    surface: u64,
    host: Host,
    layer: WeakEntity<AnnotationLayer>,
    items: Vec<Item>,
}

impl Overlay {
    fn clamp_x(x: f32, width: f32, bounds: Bounds<Pixels>) -> f32 {
        let min = f32::from(bounds.left()) + EDGE_MARGIN;
        let max = (f32::from(bounds.right()) - width - EDGE_MARGIN).max(min);
        x.clamp(min, max)
    }

    fn origin(
        &self,
        place: &Place,
        line: Bounds<Pixels>,
        item: Size<Pixels>,
        bounds: Bounds<Pixels>,
    ) -> gpui::Point<Pixels> {
        let (width, height) = (f32::from(item.width), f32::from(item.height));
        let badge_left = self.host.column.right(bounds) - BADGE_SIZE;
        let badge_top = f32::from(line.top()) - BADGE_OVER_LINE;
        match place {
            Place::OverLine(gap) => {
                let x = f32::from(line.left()) + f32::from(line.size.width) / 2.0 - width / 2.0;
                let y = f32::from(line.top()) - gap - height;
                let top = f32::from(bounds.top()) + self.host.top_inset + EDGE_MARGIN;
                point(px(Self::clamp_x(x, width, bounds)), px(y.max(top)))
            }
            Place::Badge => point(px(badge_left), px(badge_top)),
            Place::OverBadge => point(
                px(Self::clamp_x(
                    badge_left + BADGE_SIZE / 2.0 - width / 2.0,
                    width,
                    bounds,
                )),
                px(badge_top - TIP_GAP - height),
            ),
        }
    }

    /// How far to scroll so the line and what floats over it both show.
    fn reveal_delta(&self, line: Bounds<Pixels>, over: f32, bounds: Bounds<Pixels>) -> f32 {
        let top_limit = f32::from(bounds.top()) + self.host.top_inset + EDGE_MARGIN;
        let bottom_limit = f32::from(bounds.bottom()) - self.host.bottom_inset - EDGE_MARGIN;
        let top = f32::from(line.top()) - over;
        if top < top_limit {
            top - top_limit
        } else if f32::from(line.bottom()) > bottom_limit {
            f32::from(line.bottom()) - bottom_limit
        } else {
            0.0
        }
    }

    fn emit(&self, event: AnnotationEvent, cx: &mut App) {
        let layer = self.layer.clone();
        cx.defer(move |cx| {
            layer
                .update(cx, |_, cx| {
                    cx.emit(event);
                    cx.notify();
                })
                .ok();
        });
    }
}

impl IntoElement for Overlay {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Overlay {
    type RequestLayoutState = ();
    type PrepaintState = Vec<bool>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.position = gpui::Position::Absolute;
        style.inset.top = px(0.0).into();
        style.inset.left = px(0.0).into();
        style.size = size(gpui::relative(1.0).into(), gpui::relative(1.0).into());
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _state: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<bool> {
        let mut items = std::mem::take(&mut self.items);
        let shown = window.with_content_mask(Some(ContentMask { bounds }), |window| {
            items
                .iter_mut()
                .map(|item| {
                    let line = render::annotation_anchor_line(self.surface, &item.key, &item.range);
                    let Some(line) = line else {
                        if let Some(reveal) = item.reveal.as_ref()
                            && reveal.get() == Reveal::Wanted
                        {
                            reveal.set(Reveal::RowAsked);
                            self.emit(AnnotationEvent::RevealRow(item.key.clone()), cx);
                        }
                        return false;
                    };
                    let space = size(AvailableSpace::MinContent, AvailableSpace::MinContent);
                    let measured = item.element.layout_as_root(space, window, cx);
                    if let Some(reveal) = item.reveal.as_ref()
                        && reveal.get() != Reveal::Done
                    {
                        reveal.set(Reveal::Done);
                        let over = f32::from(measured.height) + EDITOR_GAP;
                        let delta = self.reveal_delta(line, over, bounds);
                        if delta != 0.0 {
                            self.emit(AnnotationEvent::ScrollBy(delta), cx);
                        }
                    }
                    let origin = self.origin(&item.place, line, measured, bounds);
                    item.element.prepaint_at(origin, window, cx);
                    true
                })
                .collect()
        });
        self.items = items;
        shown
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _state: &mut (),
        shown: &mut Vec<bool>,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for (item, shown) in self.items.iter_mut().zip(shown.iter()) {
                if *shown {
                    item.element.paint(window, cx);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(
        cx: &mut gpui::TestAppContext,
    ) -> (Entity<AppState>, Entity<AnnotationLayer>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
            crate::settings::init(crate::settings::UiSettings::default(), dir.path(), cx);
        });
        let state = cx.new(|_| AppState::new());
        let layer = cx.new(|cx| {
            let mut layer = AnnotationLayer::new(state.clone(), 1, String::new(), None, cx);
            layer.chat = Some("chat".into());
            layer
        });
        (state, layer, dir)
    }

    fn select(layer: &Entity<AnnotationLayer>, text: &str, cx: &mut gpui::TestAppContext) {
        layer.update(cx, |layer, cx| {
            layer.pending = Some(Pending {
                text: text.into(),
                spans: vec![AnchorSpan {
                    key: format!("{text}:0"),
                    range: 0..text.len(),
                }],
            });
            layer.add_to_chat(cx);
        });
    }

    fn type_comment(layer: &Entity<AnnotationLayer>, text: &str, cx: &mut gpui::TestAppContext) {
        layer.update(cx, |layer, cx| {
            let input = layer.editor.as_ref().expect("editor open").input.clone();
            input.update(cx, |input, cx| input.set_text(text, cx));
        });
    }

    fn staged(state: &Entity<AppState>, cx: &mut gpui::TestAppContext) -> Vec<(String, String)> {
        state.read_with(cx, |state, _| {
            state
                .quotes("chat")
                .iter()
                .map(|quote| {
                    (
                        quote.text.clone(),
                        quote.comment.clone().unwrap_or_default(),
                    )
                })
                .collect()
        })
    }

    #[gpui::test]
    fn the_input_commits_and_cancels_into_the_staged_quote(cx: &mut gpui::TestAppContext) {
        let (state, layer, _dir) = layer(cx);
        select(&layer, "first", cx);
        assert_eq!(staged(&state, cx), [("first".to_string(), String::new())]);
        type_comment(&layer, "  why?  ", cx);
        layer.update(cx, |layer, cx| layer.commit_editor(true, cx));
        assert_eq!(
            staged(&state, cx),
            [("first".to_string(), "why?".to_string())]
        );
        assert_eq!(
            state.read_with(cx, |state, _| state
                .annotation_requests
                .focus_composer
                .clone()),
            Some("chat".to_string())
        );

        let id = state.read_with(cx, |state, _| state.quotes("chat")[0].id.clone());
        layer.update(cx, |layer, cx| {
            layer.open_editor(id.clone(), true, "why?".into(), cx)
        });
        type_comment(&layer, "discarded", cx);
        layer.update(cx, |layer, cx| layer.cancel_editor(cx));
        assert_eq!(
            staged(&state, cx),
            [("first".to_string(), "why?".to_string())]
        );
        assert_eq!(
            layer.read_with(cx, |layer, _| layer.pinned_tip.clone()),
            Some(id)
        );

        select(&layer, "second", cx);
        layer.update(cx, |layer, cx| layer.cancel_editor(cx));
        assert_eq!(staged(&state, cx).len(), 1);
    }

    #[gpui::test]
    fn the_cards_pencil_reaches_only_the_surface_that_drew_it(cx: &mut gpui::TestAppContext) {
        let (state, layer, _dir) = layer(cx);
        select(&layer, "note", cx);
        layer.update(cx, |layer, cx| layer.commit_editor(false, cx));
        let id = state.read_with(cx, |state, _| state.quotes("chat")[0].id.clone());
        let other = cx.new(|cx| {
            let mut other = AnnotationLayer::new(
                state.clone(),
                2,
                "md-preview-1|".into(),
                Some("a.md".into()),
                cx,
            );
            other.chat = Some("chat".into());
            other
        });
        state.update(cx, |state, cx| {
            state.request_annotation_edit("chat", &id);
            cx.notify();
        });
        cx.run_until_parked();
        assert!(other.read_with(cx, |other, _| other.editor.is_none()));
        assert!(layer.read_with(cx, |layer, _| {
            layer.editor.as_ref().is_some_and(|editor| editor.editing)
        }));
        assert!(state.read_with(cx, |state, _| state.annotation_requests.edit.is_none()));
    }

    #[test]
    fn the_badge_edge_follows_the_centered_column() {
        let column = Column {
            pad_left: 48.0,
            pad_right: 48.0,
            max_width: 700.0,
        };
        let wide = Bounds::new(point(px(100.0), px(0.0)), size(px(1000.0), px(500.0)));
        assert_eq!(column.right(wide), 950.0);
        let narrow = Bounds::new(point(px(100.0), px(0.0)), size(px(600.0), px(500.0)));
        assert_eq!(column.right(narrow), 652.0);
    }
}
