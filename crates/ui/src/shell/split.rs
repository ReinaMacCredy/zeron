//! Split view: a SECOND session beside the selected one. The selected chat
//! stays the primary pane (keyboard, right pane, tab-strip selection all
//! keep following `selected_chat`); the secondary is a live transcript +
//! composer over another chat, built the way a sidechat surface is. A tab
//! or sidebar row dragged onto the conversation column reveals two drop
//! zones; dropping on a side puts that session there. Selecting the
//! secondary's chat (tab click, palette, jump key) swaps the two panes
//! instead of showing one session twice. Device-local and session-transient.

use super::*;

/// Which side of the conversation column the secondary pane occupies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SplitSide {
    Left,
    Right,
}

/// The secondary pane: its chat, side, and the transcript + composer bound
/// to that chat (the same wiring as [`SidechatTab`]).
pub(super) struct SplitPane {
    pub(super) chat_id: String,
    pub(super) side: SplitSide,
    transcript: Entity<Transcript>,
    composer: Entity<Composer>,
    _events: Subscription,
    _composer_events: Subscription,
}

/// A sidebar row being dragged. Distinct from the strip's
/// [`tabs::TabDragPayload`] on purpose: the strip's drag-move/drop
/// listeners read a tab index and would reorder on a row drag.
pub(super) struct SessionRowDrag {
    pub(super) chat_id: String,
    pub(super) title: SharedString,
    pub(super) brand: Option<(&'static str, Option<gpui::Hsla>)>,
}

/// What a drop on `side` does, given the current selection and split. Pure.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum SplitDrop {
    /// Open `chat` as the secondary on that side (replacing any secondary).
    Open,
    /// The chat is the primary and lands on the secondary's side: swap.
    Swap,
    /// Route through normal selection (the primary's side).
    Select,
    Nothing,
}

pub(super) fn split_drop(
    chat: &str,
    side: SplitSide,
    selected: Option<&str>,
    split: Option<(&str, SplitSide)>,
) -> SplitDrop {
    match split {
        None => match selected {
            // Nothing to sit beside: the drop just opens the session.
            None => SplitDrop::Select,
            Some(sel) if sel == chat => SplitDrop::Nothing,
            Some(_) => SplitDrop::Open,
        },
        Some((split_chat, split_side)) if side == split_side => {
            if split_chat == chat {
                SplitDrop::Nothing
            } else if selected == Some(chat) {
                SplitDrop::Swap
            } else {
                SplitDrop::Open
            }
        }
        Some(_) => {
            if selected == Some(chat) {
                SplitDrop::Nothing
            } else {
                SplitDrop::Select
            }
        }
    }
}

/// The split after a selection change. `previous` is the chat the primary
/// pane was showing; selecting the secondary's own chat swaps the panes
/// (the secondary takes `previous`) or closes the split when there was no
/// previous chat. Landing on the canvas (`None`) closes it too: the caller
/// promotes the secondary instead of leaving it beside the hero. Pure.
pub(super) fn split_after_selection(
    split_chat: &str,
    selected: Option<&str>,
    previous: Option<&str>,
) -> Option<String> {
    match selected {
        None => None,
        Some(sel) if sel != split_chat => Some(split_chat.to_string()),
        Some(_) => previous
            .filter(|prev| !prev.is_empty() && *prev != split_chat)
            .map(str::to_string),
    }
}

impl Shell {
    pub(super) fn split_chat_id(&self) -> Option<&str> {
        self.split.as_ref().map(|s| s.chat_id.as_str())
    }

    /// Build the secondary pane over `chat_id` on `side`, replacing any
    /// current secondary.
    pub(super) fn open_split(&mut self, chat_id: String, side: SplitSide, cx: &mut Context<Self>) {
        if self.split_chat_id() == Some(chat_id.as_str()) {
            if let Some(split) = self.split.as_mut() {
                split.side = side;
            }
            cx.notify();
            return;
        }
        self.close_split(cx);
        let transcript =
            cx.new(|cx| Transcript::for_doc(self.state.clone(), chat_id.clone(), true, cx));
        let links = Self::session_links(Some(chat_id.clone()), cx);
        transcript.update(cx, |transcript, _| {
            transcript.set_workspace_link_handler(links)
        });
        let events = cx.subscribe(&transcript, Self::on_transcript_event);
        self.state
            .update(cx, |s, cx| s.watch_subagent_doc(chat_id.clone(), cx));
        let composer = cx.new(|cx| Composer::for_chat(self.state.clone(), chat_id.clone(), cx));
        let composer_events = cx.subscribe(&composer, {
            let transcript = transcript.clone();
            move |_this: &mut Shell, _, event: &ComposerEvent, cx| match event {
                ComposerEvent::Sent {
                    chat_id,
                    message_id,
                } => transcript.update(cx, |t, cx| {
                    t.on_own_send(chat_id.clone(), message_id.clone(), cx)
                }),
                ComposerEvent::Queued {
                    chat_id,
                    message_id,
                } => transcript.update(cx, |t, cx| {
                    t.on_own_queued_send(chat_id.clone(), message_id.clone(), cx)
                }),
                ComposerEvent::OpenSidechat { .. }
                | ComposerEvent::NewThreadTransitionStarted
                | ComposerEvent::BackgroundStarted => {}
                ComposerEvent::UnsentChanged => cx.notify(),
            }
        });
        self.split = Some(SplitPane {
            chat_id,
            side,
            transcript,
            composer,
            _events: events,
            _composer_events: composer_events,
        });
        cx.notify();
    }

    /// Drop the secondary pane. The doc watch is shared with sidechat and
    /// subagent tabs (single-flight per doc id), so it is only released when
    /// no other surface still shows that doc.
    pub(super) fn close_split(&mut self, cx: &mut Context<Self>) {
        let Some(split) = self.split.take() else {
            return;
        };
        let held_elsewhere = self
            .sidechat_tabs
            .values()
            .any(|tab| tab.chat_id == split.chat_id)
            || self
                .subagent_tabs
                .values()
                .any(|tab| tab.doc_id == split.chat_id);
        if !held_elsewhere {
            self.state
                .update(cx, |s, _| s.unwatch_subagent_doc(&split.chat_id));
        }
        cx.notify();
    }

    /// Selection changed (every path lands in `on_state_changed`): keep the
    /// invariant that one session shows once. `previous` is the chat the
    /// primary was showing before the change.
    pub(super) fn reconcile_split_with_selection(
        &mut self,
        selected: Option<&str>,
        previous: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let Some(split_chat) = self.split_chat_id().map(str::to_string) else {
            return;
        };
        match split_after_selection(&split_chat, selected, previous) {
            Some(next) if next == split_chat => {}
            Some(next) => {
                let side = self
                    .split
                    .as_ref()
                    .map(|s| s.side)
                    .unwrap_or(SplitSide::Right);
                self.open_split(next, side, cx);
            }
            None => {
                self.close_split(cx);
                // The primary went away (last tab closed, chat deleted): the
                // secondary takes over as the selected session.
                if selected.is_none() {
                    self.select_existing_chat(split_chat, cx);
                }
            }
        }
    }

    /// The secondary's chat vanished (archived, deleted): the split goes.
    pub(super) fn prune_split(&mut self, cx: &mut Context<Self>) {
        let Some(split_chat) = self.split_chat_id() else {
            return;
        };
        let live = {
            let state = self.state.read(cx);
            state.chats_synced
                && state
                    .chats
                    .iter()
                    .any(|c| c.id == split_chat && !c.archived)
        };
        if !live {
            self.close_split(cx);
        }
    }

    /// A session dropped on one side of the conversation column.
    pub(super) fn drop_session_on_side(
        &mut self,
        chat_id: String,
        side: SplitSide,
        cx: &mut Context<Self>,
    ) {
        // A draft has no doc to show beside another session.
        if self.state.read(cx).is_draft(&chat_id) {
            return;
        }
        let selected = self.state.read(cx).selected_chat.clone();
        let split = self.split.as_ref().map(|s| (s.chat_id.clone(), s.side));
        match split_drop(
            &chat_id,
            side,
            selected.as_deref(),
            split.as_ref().map(|(id, side)| (id.as_str(), *side)),
        ) {
            SplitDrop::Open => self.open_split(chat_id, side, cx),
            SplitDrop::Swap => {
                // Selecting the secondary's chat swaps through the selection
                // invariant; the secondary then sits where the drop landed.
                let Some((split_chat, _)) = split else {
                    return;
                };
                if let Some(s) = self.split.as_mut() {
                    s.side = side;
                }
                self.open_chat(split_chat, cx);
            }
            SplitDrop::Select => self.open_chat(chat_id, cx),
            SplitDrop::Nothing => {}
        }
    }

    /// The secondary pane: a header (brand, title, close) under the
    /// titlebar band, the live transcript, its own composer.
    pub(super) fn render_split_pane(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let split = self.split.as_ref()?;
        let theme = Theme::of(cx).clone();
        let (chat_id, side, transcript, composer) = (
            split.chat_id.clone(),
            split.side,
            split.transcript.clone(),
            split.composer.clone(),
        );
        let (title, brand) = {
            let state = self.state.read(cx);
            let chat = state.chats.iter().find(|c| c.id == chat_id);
            (
                SharedString::from(transcript::single_line(
                    chat.and_then(|c| c.title.as_deref())
                        .unwrap_or("New session"),
                )),
                chat.and_then(|c| c.config.as_ref().map(|c| c.harness))
                    .map(crate::pickers::harness_brand_icon),
            )
        };
        let focus_id = chat_id.clone();
        let header = div()
            .flex_none()
            .h(px(32.0))
            .px(px(12.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .border_b_1()
            .border_color(theme.border)
            .text_size(crate::typography::ui_rems(12.0))
            .text_color(theme.text_muted)
            .when_some(brand, |el, (path, tint)| {
                el.child(
                    icon(path)
                        .size(px(14.0))
                        .flex_none()
                        .text_color(tint.unwrap_or(theme.text_muted)),
                )
            })
            .child(div().flex_1().min_w_0().truncate().child(title))
            .child(
                div()
                    .id("split-pane-focus")
                    .flex_none()
                    .size(px(20.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(crate::theme::wash(0.14)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.open_chat(focus_id.clone(), cx);
                    }))
                    .child(
                        icon(icons::SPLIT_COLUMNS)
                            .size(px(12.0))
                            .text_color(theme.text_muted),
                    ),
            )
            .child(
                div()
                    .id("split-pane-close")
                    .flex_none()
                    .size(px(20.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(crate::theme::wash(0.14)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.close_split(cx);
                    }))
                    .child(
                        icon(icons::CLOSE)
                            .size(px(12.0))
                            .text_color(theme.text_muted),
                    ),
            );
        Some(
            div()
                .id("split-pane")
                .h_full()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .pt(px(Theme::TITLEBAR_HEIGHT))
                .when(side == SplitSide::Right, |el| {
                    el.border_l_1().border_color(theme.border)
                })
                .when(side == SplitSide::Left, |el| {
                    el.border_r_1().border_color(theme.border)
                })
                .child(header)
                .child(div().flex_1().min_h_0().child(transcript))
                .child(div().flex_none().child(composer))
                .into_any_element(),
        )
    }

    /// The two drop zones over the conversation column. Mounted only while
    /// a drag is active; GPUI reveals them by the dragged payload's concrete
    /// type (`drag_over`), so a pane-resize drag never shows them.
    pub(super) fn render_split_drop_zones(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !cx.has_active_drag() {
            return None;
        }
        let theme = Theme::of(cx).clone();
        let zone = |side: SplitSide, cx: &mut Context<Self>| {
            let (id, label) = match side {
                SplitSide::Left => ("split-drop-left", "Split left"),
                SplitSide::Right => ("split-drop-right", "Split right"),
            };
            let accent = theme.accent;
            div()
                .id(id)
                .flex_1()
                .min_w_0()
                .h_full()
                .p(px(24.0))
                .flex()
                .items_center()
                .justify_center()
                .drag_over::<tabs::TabDragPayload>(move |style, _, _, _| {
                    style.bg(accent.opacity(0.10))
                })
                .drag_over::<SessionRowDrag>(move |style, _, _, _| style.bg(accent.opacity(0.10)))
                .on_drop::<tabs::TabDragPayload>(cx.listener(
                    move |this, payload: &tabs::TabDragPayload, _, cx| {
                        this.drop_session_on_side(payload.chat_id.clone(), side, cx);
                    },
                ))
                .on_drop::<SessionRowDrag>(cx.listener(
                    move |this, payload: &SessionRowDrag, _, cx| {
                        this.drop_session_on_side(payload.chat_id.clone(), side, cx);
                    },
                ))
                .child(
                    div()
                        .w(px(148.0))
                        .max_h(px(420.0))
                        .h_full()
                        .rounded(px(14.0))
                        .border_1()
                        .border_dashed()
                        .border_color(theme.border_strong)
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(10.0))
                        .text_size(crate::typography::ui_rems(12.0))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(theme.text_muted)
                        .child(
                            icon(icons::SPLIT_COLUMNS)
                                .size(px(18.0))
                                .text_color(theme.text_muted),
                        )
                        .child(SharedString::from(label)),
                )
        };
        let left = zone(SplitSide::Left, cx);
        let right = zone(SplitSide::Right, cx);
        Some(
            div()
                .id("split-drop-zones")
                .absolute()
                .top(px(Theme::TITLEBAR_HEIGHT))
                .left_0()
                .right_0()
                .bottom_0()
                .flex()
                .flex_row()
                .opacity(0.0)
                .bg(theme.scrim().opacity(0.35))
                .drag_over::<tabs::TabDragPayload>(|style, _, _, _| style.opacity(1.0))
                .drag_over::<SessionRowDrag>(|style, _, _, _| style.opacity(1.0))
                .child(left)
                .child(right)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_beside_a_selected_chat_opens_the_split() {
        assert_eq!(
            split_drop("b", SplitSide::Right, Some("a"), None),
            SplitDrop::Open
        );
    }

    #[test]
    fn dropping_the_selected_chat_with_no_split_does_nothing() {
        assert_eq!(
            split_drop("a", SplitSide::Left, Some("a"), None),
            SplitDrop::Nothing
        );
    }

    #[test]
    fn drop_on_the_canvas_selects() {
        assert_eq!(
            split_drop("a", SplitSide::Left, None, None),
            SplitDrop::Select
        );
    }

    #[test]
    fn drop_on_the_secondary_side_replaces_it() {
        assert_eq!(
            split_drop(
                "c",
                SplitSide::Right,
                Some("a"),
                Some(("b", SplitSide::Right))
            ),
            SplitDrop::Open
        );
        assert_eq!(
            split_drop(
                "b",
                SplitSide::Right,
                Some("a"),
                Some(("b", SplitSide::Right))
            ),
            SplitDrop::Nothing
        );
    }

    #[test]
    fn dropping_the_primary_on_the_secondary_side_swaps() {
        assert_eq!(
            split_drop(
                "a",
                SplitSide::Right,
                Some("a"),
                Some(("b", SplitSide::Right))
            ),
            SplitDrop::Swap
        );
    }

    #[test]
    fn drop_on_the_primary_side_selects() {
        assert_eq!(
            split_drop(
                "c",
                SplitSide::Left,
                Some("a"),
                Some(("b", SplitSide::Right))
            ),
            SplitDrop::Select
        );
        assert_eq!(
            split_drop(
                "b",
                SplitSide::Left,
                Some("a"),
                Some(("b", SplitSide::Right))
            ),
            SplitDrop::Select
        );
        assert_eq!(
            split_drop(
                "a",
                SplitSide::Left,
                Some("a"),
                Some(("b", SplitSide::Right))
            ),
            SplitDrop::Nothing
        );
    }

    #[test]
    fn selecting_the_secondary_swaps_the_panes() {
        assert_eq!(
            split_after_selection("b", Some("b"), Some("a")),
            Some("a".to_string())
        );
    }

    #[test]
    fn selecting_the_secondary_off_the_canvas_closes_the_split() {
        assert_eq!(split_after_selection("b", Some("b"), None), None);
        assert_eq!(split_after_selection("b", Some("b"), Some("")), None);
    }

    #[test]
    fn selecting_another_chat_keeps_the_split() {
        assert_eq!(
            split_after_selection("b", Some("c"), Some("a")),
            Some("b".to_string())
        );
    }

    #[test]
    fn landing_on_the_canvas_closes_the_split() {
        assert_eq!(split_after_selection("b", None, Some("a")), None);
    }
}
