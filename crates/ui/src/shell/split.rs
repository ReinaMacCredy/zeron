//! Split view: two sessions PAIRED side by side. The pair is a device-local
//! fact (`Shell::split_pair`: left chat, right chat) that outlives what is
//! on screen: while the selected chat is one of the pair, the other renders
//! as the secondary pane (a live transcript + composer over that chat,
//! built the way a sidechat surface is) on its side; selecting any other
//! session shows that session full-width and the pair waits on the tab
//! strip as one double chip. A tab or sidebar row dragged onto the
//! conversation column reveals two drop zones; dropping on a side puts
//! that session there. The selected chat stays the primary pane (keyboard,
//! right pane, tab-strip selection all keep following `selected_chat`).

use super::*;

/// Which side of the conversation column a pane occupies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SplitSide {
    Left,
    Right,
}

/// The two paired chats in visual order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SplitPair {
    pub(super) left: String,
    pub(super) right: String,
}

impl SplitPair {
    pub(super) fn contains(&self, chat: &str) -> bool {
        self.left == chat || self.right == chat
    }

    fn member(&self, side: SplitSide) -> &str {
        match side {
            SplitSide::Left => &self.left,
            SplitSide::Right => &self.right,
        }
    }

    pub(super) fn side_of(&self, chat: &str) -> Option<SplitSide> {
        if self.left == chat {
            Some(SplitSide::Left)
        } else if self.right == chat {
            Some(SplitSide::Right)
        } else {
            None
        }
    }

    /// Sidebar order while the split is open: the later member moves up
    /// beside the earlier one (the strip's double-chip rule), so the two
    /// rows can share one card wherever the earlier one sits.
    pub(super) fn adjoin(&self, order: &mut Vec<String>) {
        let (Some(a), Some(b)) = (
            order.iter().position(|id| *id == self.left),
            order.iter().position(|id| *id == self.right),
        ) else {
            return;
        };
        let (first, later) = if a < b { (a, b) } else { (b, a) };
        let moved = order.remove(later);
        order.insert(first + 1, moved);
    }

    /// [`Self::adjoin`] over grouped rows: the later member joins the
    /// earlier one's group right after it.
    pub(super) fn adjoin_grouped<G, T>(&self, groups: &mut [(G, Vec<T>)], id: impl Fn(&T) -> &str) {
        let find = |chat: &str| {
            groups.iter().enumerate().find_map(|(g, (_, rows))| {
                rows.iter().position(|row| id(row) == chat).map(|r| (g, r))
            })
        };
        let (Some(a), Some(b)) = (find(&self.left), find(&self.right)) else {
            return;
        };
        let (first, later) = if a < b { (a, b) } else { (b, a) };
        let moved = groups[later.0].1.remove(later.1);
        groups[first.0].1.insert(first.1 + 1, moved);
    }

    /// The other member and the side it sits on, for a member `chat`.
    pub(super) fn partner(&self, chat: &str) -> Option<(&str, SplitSide)> {
        if self.left == chat {
            Some((&self.right, SplitSide::Right))
        } else if self.right == chat {
            Some((&self.left, SplitSide::Left))
        } else {
            None
        }
    }

    fn with(&self, side: SplitSide, chat: String) -> Self {
        match side {
            SplitSide::Left => Self {
                left: chat,
                right: self.right.clone(),
            },
            SplitSide::Right => Self {
                left: self.left.clone(),
                right: chat,
            },
        }
    }

    fn of(anchor: String, chat: String, chat_side: SplitSide) -> Self {
        match chat_side {
            SplitSide::Left => Self {
                left: chat,
                right: anchor,
            },
            SplitSide::Right => Self {
                left: anchor,
                right: chat,
            },
        }
    }
}

/// The secondary pane: its chat, side, and the transcript + composer bound
/// to that chat (the same wiring as [`SidechatTab`]).
pub(super) struct SplitPane {
    pub(crate) chat_id: String,
    pub(super) side: SplitSide,
    transcript: Entity<Transcript>,
    composer: Entity<Composer>,
    _events: Subscription,
    _composer_events: Subscription,
}

impl SplitPane {
    pub(super) fn composer_for(&self, transcript: &Entity<Transcript>) -> Option<Entity<Composer>> {
        (&self.transcript == transcript).then(|| self.composer.clone())
    }
}

/// Which drop zone the pointer is over during a drag; `epoch` keys the
/// grow/shrink animation restarts (the tab strip's slide idiom).
#[derive(Debug, Default)]
pub(super) struct SplitDropHot {
    pub(super) hot: Option<SplitSide>,
    pub(super) prev: Option<SplitSide>,
    pub(super) epoch: usize,
}

/// Slack above the composer pill where the split halves already yield to
/// it, so a drop aimed at the pill's top edge still lands as a mention.
const COMPOSER_DROP_MARGIN: f32 = 8.0;

/// What a drop on `side` does: the pair afterwards and the chat to select
/// (when the drop replaced or created the primary's slot). Pure.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct SplitDrop {
    pub(super) pair: Option<SplitPair>,
    pub(super) select: Option<String>,
}

pub(super) fn split_drop(
    chat: &str,
    side: SplitSide,
    selected: Option<&str>,
    pair: Option<&SplitPair>,
) -> SplitDrop {
    let keep = SplitDrop {
        pair: pair.cloned(),
        select: None,
    };
    let Some(selected) = selected else {
        // Nothing to sit beside: the drop just opens the session.
        return SplitDrop {
            pair: pair.cloned(),
            select: Some(chat.to_string()),
        };
    };
    match pair {
        Some(pair) if pair.contains(selected) => {
            let occupant = pair.member(side);
            if occupant == chat {
                keep
            } else if pair.contains(chat) {
                // A member dropped on the other's side: the pair flips.
                SplitDrop {
                    pair: Some(SplitPair {
                        left: pair.right.clone(),
                        right: pair.left.clone(),
                    }),
                    select: None,
                }
            } else {
                // Replace that side's member; a replaced primary hands the
                // selection to the newcomer.
                SplitDrop {
                    pair: Some(pair.with(side, chat.to_string())),
                    select: (occupant == selected).then(|| chat.to_string()),
                }
            }
        }
        // The selected session is shown full: it pairs with the drop (any
        // older pair gives way — one pair at a time).
        _ if selected == chat => keep,
        _ => SplitDrop {
            pair: Some(SplitPair::of(selected.to_string(), chat.to_string(), side)),
            select: None,
        },
    }
}

/// The sidebar pair card's caption strip and its inset around the rows.
const SPLIT_CARD_CAPTION: f32 = 20.0;
const SPLIT_CARD_INSET: f32 = 2.0;
pub(super) const SPLIT_HEADER_HEIGHT: f32 = 32.0;

impl Shell {
    /// The chat shown in the secondary pane right now.
    pub(super) fn split_chat_id(&self) -> Option<&str> {
        self.split.as_ref().map(|s| s.chat_id.as_str())
    }

    /// The pair, open or waiting, once [`SplitPair::adjoin`] has put its rows
    /// side by side in the sidebar; a pinned member keeps its pin slot, so
    /// the rows then stay apart and wear only their side glyphs.
    pub(super) fn sidebar_split_card(&self, pinned: &[String]) -> Option<SplitPair> {
        self.split_pair
            .clone()
            .filter(|pair| !pinned.contains(&pair.left) && !pinned.contains(&pair.right))
    }

    /// Fold the pair's two adjacent keyed rows into one card: a caption
    /// with the split's close above the rows. The card's height adds up
    /// exactly so the section's disclosure math is unchanged.
    pub(super) fn join_split_rows(
        &self,
        rows: &mut Vec<(String, f32, AnyElement)>,
        pair: &SplitPair,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) {
        let keys = [format!("c:{}", pair.left), format!("c:{}", pair.right)];
        let Some(ix) = rows
            .windows(2)
            .position(|w| keys.contains(&w[0].0) && keys.contains(&w[1].0) && w[0].0 != w[1].0)
        else {
            return;
        };
        let (_, second_height, second) = rows.remove(ix + 1);
        let (key, first_height, first) = rows.remove(ix);
        let height = SPLIT_CARD_INSET * 2.0
            + SPLIT_CARD_CAPTION
            + SIDEBAR_LIST_GAP * 2.0
            + first_height
            + second_height;
        let caption = div()
            .h(px(SPLIT_CARD_CAPTION))
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(5.0))
            .px(px(Theme::SPACE_SM))
            .text_size(crate::typography::ui_rems(10.0))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(theme.text_muted)
            .child(
                icon(icons::SPLIT_COLUMNS)
                    .size(px(11.0))
                    .flex_none()
                    .text_color(theme.text_muted),
            )
            .child(SharedString::from("Split"))
            .child(div().flex_1())
            .child(
                div()
                    .id("sidebar-split-close")
                    .debug_selector(|| "sidebar-split-close".into())
                    .size(px(16.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.0))
                    .opacity(0.0)
                    .group_hover("sidebar-split-card", |s| s.opacity(1.0))
                    .hover(|s| s.bg(crate::theme::wash(0.14)))
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.dissolve_split_pair(cx);
                    }))
                    .child(
                        icon(icons::CLOSE)
                            .size(px(11.0))
                            .text_color(theme.text_muted),
                    ),
            );
        let card = div()
            .id("sidebar-split-card")
            .debug_selector(|| "sidebar-split-card".into())
            .group("sidebar-split-card")
            .w_full()
            .flex()
            .flex_col()
            .gap(px(SIDEBAR_LIST_GAP))
            .p(px(SPLIT_CARD_INSET))
            .rounded(px(10.0))
            .bg(crate::theme::wash(0.03))
            .shadow(crate::theme::card_selected_shadows())
            .child(caption)
            .child(first)
            .child(second)
            .into_any_element();
        rows.insert(ix, (key, height, card));
    }

    pub(super) fn split_pair_contains(&self, chat: &str) -> bool {
        self.split_pair.as_ref().is_some_and(|p| p.contains(chat))
    }

    /// Build the secondary pane over `chat_id` on `side`, replacing any
    /// current secondary.
    fn open_split(&mut self, chat_id: String, side: SplitSide, cx: &mut Context<Self>) {
        if self.split_chat_id() == Some(chat_id.as_str()) {
            if let Some(split) = self.split.as_mut()
                && split.side != side
            {
                split.side = side;
                cx.notify();
            }
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
            move |this: &mut Shell, _, event: &ComposerEvent, cx| {
                this.on_pane_composer_event(&transcript, event, cx)
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

    /// Sidechats and new threads have no surface to open on from the split or the task center.
    pub(super) fn on_pane_composer_event(
        &mut self,
        transcript: &Entity<Transcript>,
        event: &ComposerEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
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
            | ComposerEvent::BackgroundStarted
            | ComposerEvent::WorkspaceCommand(_)
            | ComposerEvent::WorktreeSetup { .. }
            | ComposerEvent::OpenChat(_)
            | ComposerEvent::OpenChanges => {}
            ComposerEvent::OpenSubagent {
                chat_id,
                doc_id,
                title,
                frozen,
            } => self.add_subagent_surface(
                chat_id.clone(),
                doc_id.clone(),
                title.clone(),
                *frozen,
                cx,
            ),
            ComposerEvent::UnsentChanged => cx.notify(),
            ComposerEvent::ThreadMentionDropped => self.accept_command_palette_drop(),
            ComposerEvent::OpenComputerSetup { missing } => {
                self.open_computer_permission_guide(missing.clone(), cx)
            }
        }
    }

    /// Drop the secondary pane (the pair stays). The doc watch is shared
    /// with sidechat and subagent tabs (single-flight per doc id), so it is
    /// only released when no other surface still shows that doc.
    fn close_split(&mut self, cx: &mut Context<Self>) {
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
                .any(|tab| tab.doc_id == split.chat_id)
            || self.task_center_holds(&split.chat_id);
        if !held_elsewhere {
            self.state
                .update(cx, |s, _| s.unwatch_subagent_doc(&split.chat_id));
        }
        cx.notify();
    }

    /// Forget the pair: the pane closes and the double chip splits back
    /// into ordinary tabs.
    pub(super) fn dissolve_split_pair(&mut self, cx: &mut Context<Self>) {
        self.split_pair = None;
        self.close_split(cx);
    }

    /// Focus changed (every path lands in `on_state_changed`): a focused
    /// pair member shows its partner beside it; anything else shows full
    /// width. The canvas draft is a member like any other (its id becomes
    /// the chat's on send), but a draft partner has no doc to show, so the
    /// pair is one-way until it promotes.
    pub(super) fn reconcile_split_with_selection(
        &mut self,
        selected: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let partner = selected.and_then(|selected| {
            self.split_pair
                .as_ref()
                .and_then(|pair| pair.partner(selected))
                .filter(|(chat, _)| !self.state.read(cx).is_draft(chat))
                .map(|(chat, side)| (chat.to_string(), side))
        });
        match partner {
            Some((chat, side)) => self.open_split(chat, side, cx),
            None => self.close_split(cx),
        }
    }

    /// A pair member vanished (archived, deleted): the pair goes. A draft
    /// counts as live, and so does a selected chat with no row yet: a
    /// just-promoted draft is neither a draft nor a synced row until its
    /// doc frame lands. A selected chat whose row IS archived is gone.
    pub(super) fn prune_split(&mut self, cx: &mut Context<Self>) {
        let Some(pair) = self.split_pair.clone() else {
            return;
        };
        let live = {
            let state = self.state.read(cx);
            !state.chats_synced
                || [&pair.left, &pair.right].iter().all(|id| {
                    state.is_draft(id)
                        || match state.chats.iter().find(|c| c.id == **id) {
                            Some(chat) => !chat.archived,
                            None => state.selected_chat.as_deref() == Some(id.as_str()),
                        }
                })
        };
        if !live {
            self.dissolve_split_pair(cx);
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
        if self.state.read(cx).is_draft(&chat_id) || terminal_tabs::is_terminal_tab(&chat_id) {
            return;
        }
        let selected = self.state.read(cx).focused_id();
        let verdict = split_drop(
            &chat_id,
            side,
            selected.as_deref(),
            self.split_pair.as_ref(),
        );
        tracing::info!(chat = %chat_id, ?side, ?selected, pair = ?self.split_pair, ?verdict, "split: drop verdict");
        let SplitDrop { pair, select } = verdict;
        self.split_pair = pair;
        self.accept_command_palette_drop();
        match select {
            Some(chat) => self.open_chat(chat, cx),
            None => self.reconcile_split_with_selection(selected.as_deref(), cx),
        }
    }

    /// Pointer moved over a zone while dragging: restart the grow/shrink
    /// animation only on a change of side.
    fn set_split_drop_hot(&mut self, side: Option<SplitSide>, cx: &mut Context<Self>) {
        let state = self.split_drop_hot.get_or_insert_with(Default::default);
        if state.hot != side {
            state.prev = state.hot;
            state.hot = side;
            state.epoch += 1;
            cx.notify();
        }
    }

    /// GPUI fires `on_drag_move` on EVERY listener, hovered or not, so the
    /// side comes from the pointer against the layer's bounds (outside the
    /// layer, or over the composer's own drop target: no zone is hot).
    fn split_zone_at(
        bounds: gpui::Bounds<Pixels>,
        position: Point<Pixels>,
        composer_top: Option<Pixels>,
    ) -> Option<SplitSide> {
        if !bounds.contains(&position) {
            return None;
        }
        if composer_top.is_some_and(|top| position.y >= top - px(COMPOSER_DROP_MARGIN)) {
            return None;
        }
        Some(if position.x < bounds.center().x {
            SplitSide::Left
        } else {
            SplitSide::Right
        })
    }

    /// Window-space top of the composer pill, measured last frame. The split
    /// halves stop above it so the pill's own drop listeners (thread
    /// mention) receive a drop instead of the half behind it.
    fn composer_drop_top(&self, cx: &App) -> Option<Pixels> {
        self.composer
            .read(cx)
            .surface_bounds()
            .get()
            .map(|bounds| bounds.top())
    }

    /// While the seam is held, each pane dims under a centred `W × H`
    /// badge (Ghostty's resize overlay) so the resulting size reads at a
    /// glance. `width` is the pane's laid-out width; the veil is inset 1px
    /// on the seam-facing edge so the seam highlight stays on top.
    pub(super) fn render_split_size_veil(
        &self,
        width: f32,
        seam_edge: SplitSide,
        cx: &App,
    ) -> Option<AnyElement> {
        if self.pane_resize_dragging != Some(PaneResizeKind::Split) {
            return None;
        }
        let theme = Theme::of(cx);
        let height = (self.viewport_height - Theme::TITLEBAR_HEIGHT).max(0.0);
        let (inset_l, inset_r) = match seam_edge {
            SplitSide::Left => (1.0, 0.0),
            SplitSide::Right => (0.0, 1.0),
        };
        let badge = div()
            .px(px(9.0))
            .py(px(5.0))
            .rounded(px(6.0))
            .bg(theme.surface_overlay)
            .border_1()
            .border_color(theme.border_strong)
            .shadow_md()
            .flex()
            .items_center()
            .gap(px(4.0))
            .font_family(theme.font_mono.clone())
            .text_size(crate::typography::ui_rems(12.0))
            .text_color(theme.text)
            .child(SharedString::from(format!("{}", width.round() as i32)))
            .child(div().text_color(theme.text_muted).child("×"))
            .child(SharedString::from(format!("{}", height.round() as i32)));
        let selector = match seam_edge {
            SplitSide::Left => "split-size-veil-right",
            SplitSide::Right => "split-size-veil-left",
        };
        Some(
            div()
                .debug_selector(|| selector.into())
                .absolute()
                .top(px(Theme::TITLEBAR_HEIGHT))
                .bottom_0()
                .left(px(inset_l))
                .right(px(inset_r))
                .bg(theme.scrim().opacity(0.35))
                .flex()
                .items_center()
                .justify_center()
                .child(badge)
                .into_any_element(),
        )
    }

    /// Close the selected pane of an open split: the pair goes and the
    /// partner takes the whole column.
    pub(super) fn close_primary_split(&mut self, cx: &mut Context<Self>) {
        let Some(partner) = self.split_chat_id().map(str::to_string) else {
            return;
        };
        self.dissolve_split_pair(cx);
        self.open_chat(partner, cx);
    }

    /// A pane's header, the same on both sides of an open split: brand,
    /// title, the split glyph (on the secondary, a click focuses it) and a
    /// close that removes that pane.
    pub(super) fn render_split_header(
        &self,
        chat_id: &str,
        primary: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let theme = Theme::of(cx).clone();
        let (title, brand) = {
            let state = self.state.read(cx);
            let chat = state.chats.iter().find(|c| c.id == chat_id);
            (
                SharedString::from(transcript::single_line(
                    chat.and_then(|c| c.title.as_deref())
                        .unwrap_or("New session"),
                )),
                chat.and_then(|c| crate::pickers::chat_mark(c, cx)),
            )
        };
        let (focus_id, close_id) = if primary {
            ("split-primary-focus", "split-primary-close")
        } else {
            ("split-pane-focus", "split-pane-close")
        };
        let focus_chat = chat_id.to_string();
        div()
            .flex_none()
            .h(px(SPLIT_HEADER_HEIGHT))
            .px(px(12.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .border_b_1()
            .border_color(theme.border)
            .text_size(crate::typography::ui_rems(12.0))
            .text_color(theme.text_muted)
            .when_some(brand, |el, brand| {
                el.child(brand.svg(theme.text_muted).size(px(14.0)).flex_none())
            })
            .child(div().flex_1().min_w_0().truncate().child(title))
            .child(
                div()
                    .id(focus_id)
                    .debug_selector(move || focus_id.into())
                    .flex_none()
                    .size(px(20.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .when(!primary, |el| {
                        el.cursor_pointer()
                            .hover(|s| s.bg(crate::theme::wash(0.14)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.open_chat(focus_chat.clone(), cx);
                            }))
                    })
                    .child(
                        icon(icons::SPLIT_COLUMNS)
                            .size(px(12.0))
                            .text_color(if primary {
                                theme.accent
                            } else {
                                theme.text_muted
                            }),
                    ),
            )
            .child(
                div()
                    .id(close_id)
                    .debug_selector(move || close_id.into())
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
                        if primary {
                            this.close_primary_split(cx);
                        } else {
                            this.dissolve_split_pair(cx);
                        }
                    }))
                    .child(
                        icon(icons::CLOSE)
                            .size(px(12.0))
                            .text_color(theme.text_muted),
                    ),
            )
    }

    /// The secondary pane: a header (brand, title, close) under the
    /// titlebar band, the live transcript, its own composer. `width` is the
    /// pane's laid-out width, for the resize badge.
    pub(super) fn render_split_pane(
        &mut self,
        width: f32,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let split = self.split.as_ref()?;
        let theme = Theme::of(cx).clone();
        let (chat_id, side, transcript, composer) = (
            split.chat_id.clone(),
            split.side,
            split.transcript.clone(),
            split.composer.clone(),
        );
        let seam_edge = match side {
            SplitSide::Left => SplitSide::Right,
            SplitSide::Right => SplitSide::Left,
        };
        let veil = self.render_split_size_veil(width, seam_edge, cx);
        let header = self.render_split_header(&chat_id, false, cx);
        Some(
            div()
                .id("split-pane")
                .debug_selector(|| "split-pane".into())
                .h_full()
                .flex_1()
                .min_w_0()
                .relative()
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
                .children(veil)
                .into_any_element(),
        )
    }

    /// The two drop zones over the conversation column. Mounted only while
    /// a drag is active; GPUI reveals them by the dragged payload's concrete
    /// type (`drag_over`), so a pane-resize drag never shows them. The zone
    /// under the pointer grows and fills (Atlas-style) on a 150ms curve; a
    /// drag that cannot split (the lone selected session, a draft) dims the
    /// whole layer.
    pub(super) fn render_split_drop_zones(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !cx.has_active_drag() {
            self.split_drop_hot = None;
            return None;
        }
        let theme = Theme::of(cx).clone();
        let selected = self.state.read(cx).selected_chat.clone();
        let state = self.state.clone();
        let inert = std::rc::Rc::new(move |chat_id: &str, cx: &App| {
            state.read(cx).is_draft(chat_id)
                || terminal_tabs::is_terminal_tab(chat_id)
                || selected.as_deref() == Some(chat_id)
        });
        let (hot, prev, epoch) = self
            .split_drop_hot
            .as_ref()
            .map(|h| (h.hot, h.prev, h.epoch))
            .unwrap_or((None, None, 0));
        let zone = |side: SplitSide, cx: &mut Context<Self>| {
            let (id, label) = match side {
                SplitSide::Left => ("split-drop-left", "Split left"),
                SplitSide::Right => ("split-drop-right", "Split right"),
            };
            let is_hot = hot == Some(side);
            let was_hot = prev == Some(side);
            let size = |hot: bool| if hot { (184.0, 520.0) } else { (148.0, 420.0) };
            let (from_w, from_h) = size(was_hot);
            let (to_w, to_h) = size(is_hot);
            let (border, bg, text) = if is_hot {
                (theme.accent, theme.accent.opacity(0.12), theme.text)
            } else {
                (
                    theme.border_strong,
                    crate::theme::wash(0.0),
                    theme.text_muted,
                )
            };
            let card = div()
                .max_h(px(to_h))
                .h_full()
                .rounded(px(14.0))
                .border_1()
                .border_dashed()
                .border_color(border)
                .bg(bg)
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(10.0))
                .text_size(crate::typography::ui_rems(12.0))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(text)
                .child(icon(icons::SPLIT_COLUMNS).size(px(18.0)).text_color(text))
                .child(SharedString::from(label))
                .with_animation(
                    SharedString::from(format!("{id}-grow-{epoch}")),
                    TAB_SLIDE.animation(),
                    move |el, t| {
                        el.w(px(motion::lerp(from_w, to_w, t)))
                            .max_h(px(motion::lerp(from_h, to_h, t)))
                    },
                );
            div()
                .id(id)
                .debug_selector(|| id.to_string())
                .flex_1()
                .min_w_0()
                .h_full()
                .px(px(32.0))
                .py(px(24.0))
                .flex()
                .items_center()
                // The boxes hug the column's outer edges (Atlas): the middle
                // stays clear so the drag ghost reads over the transcript.
                .when(side == SplitSide::Left, |el| el.justify_start())
                .when(side == SplitSide::Right, |el| el.justify_end())
                .on_drop::<tabs::TabDragPayload>(cx.listener(
                    move |this, payload: &tabs::TabDragPayload, _, cx| {
                        tracing::info!(chat = %payload.chat_id, ?side, "split: tab dropped");
                        this.drop_session_on_side(payload.chat_id.clone(), side, cx);
                    },
                ))
                .on_drop::<SidebarSessionDrag>(cx.listener(
                    move |this, payload: &SidebarSessionDrag, _, cx| {
                        tracing::info!(chat = %payload.chat_id, ?side, "split: row dropped");
                        this.drop_session_on_side(payload.chat_id.clone(), side, cx);
                        this.cancel_sidebar_session_transfer(cx);
                        cx.stop_propagation();
                    },
                ))
                .child(card)
        };
        let left = zone(SplitSide::Left, cx);
        let right = zone(SplitSide::Right, cx);
        // A listener-free spacer keeps the halves off the composer pill, so
        // a drop there reaches the pill's own thread-mention target.
        let composer_top = self.composer_drop_top(cx);
        let spacer_h = composer_top
            .map(|top| (self.viewport_height - f32::from(top) + COMPOSER_DROP_MARGIN).max(0.0))
            .unwrap_or(0.0);
        let inert_tab = inert.clone();
        Some(
            div()
                .id("split-drop-zones")
                .absolute()
                .top(px(Theme::TITLEBAR_HEIGHT))
                .left_0()
                .right_0()
                .bottom_0()
                .flex()
                .flex_col()
                .opacity(0.0)
                .bg(theme.scrim().opacity(0.35))
                .on_drag_move::<tabs::TabDragPayload>(cx.listener(
                    |this, event: &gpui::DragMoveEvent<tabs::TabDragPayload>, _, cx| {
                        let top = this.composer_drop_top(cx);
                        let side = Self::split_zone_at(event.bounds, event.event.position, top);
                        this.set_split_drop_hot(side, cx);
                    },
                ))
                .on_drag_move::<SidebarSessionDrag>(cx.listener(
                    |this, event: &gpui::DragMoveEvent<SidebarSessionDrag>, _, cx| {
                        let top = this.composer_drop_top(cx);
                        let side = Self::split_zone_at(event.bounds, event.event.position, top);
                        this.set_split_drop_hot(side, cx);
                    },
                ))
                .drag_over::<tabs::TabDragPayload>(move |style, payload, _, cx| {
                    style.opacity(if inert_tab(&payload.chat_id, cx) {
                        0.3
                    } else {
                        1.0
                    })
                })
                .drag_over::<SidebarSessionDrag>(move |style, payload, _, cx| {
                    style.opacity(if inert(&payload.chat_id, cx) {
                        0.3
                    } else {
                        1.0
                    })
                })
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_row()
                        .child(left)
                        .child(right),
                )
                .child(div().flex_none().h(px(spacer_h)))
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod drag_regressions {
    use super::*;
    use gpui::{AppContext, TestAppContext, VisualTestContext};

    fn chat(id: &str) -> zeron_proto::Chat {
        zeron_proto::Chat {
            id: id.into(),
            device_id: "dev".into(),
            title: Some(id.into()),
            archived: false,
            cwd: None,
            branch: None,
            checkout_id: None,
            source_context: None,
            config: None,
            last_message_preview: None,
            last_message_at: None,
            created_at: chrono::Utc::now(),
            harness_session_id: None,
            harness_session_cwd: None,
            space_id: None,
            last_seen_at: None,
            room_gen: None,
            sidechat_source_chat_id: None,
            created_by_chat_id: None,
            previous_harnesses: Vec::new(),
            delegation: None,
            settle: None,
            unsettled_at: None,
        }
    }

    fn boot_config(dir: &tempfile::TempDir) -> EngineBootConfig {
        EngineBootConfig {
            data_dir: dir.path().into(),
            ipc_port: 0,
            edge_url: "http://127.0.0.1:1".into(),
            edge_token: None,
            org_id: None,
            workos_client_id: None,
            default_harness: zeron_proto::HarnessId::Mock,
        }
    }

    fn seed(shell: &Shell, cx: &mut Context<Shell>) {
        shell.state.update(cx, |state, _| {
            state.workspace_scope = Some(zeron_proto::WorkspaceScope::Local);
            state.chats = vec![chat("a"), chat("b")];
            state.chats_synced = true;
            state.spaces_synced = true;
            state.selected_chat = Some("a".into());
        });
    }

    fn init(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            crate::app_menus::init(cx);
        });
    }

    /// A sidebar row beside a conversation column with the split drop zones:
    /// the production row and zone elements, driven by the real Shell. (The
    /// full page caches the sidebar view, which hides row bounds from
    /// `debug_bounds`; the tab test below drives the full page instead.)
    struct RowHost {
        shell: Entity<Shell>,
        _data_dir: tempfile::TempDir,
    }

    impl Render for RowHost {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.shell.update(cx, |shell, cx| {
                let theme = Theme::of(cx).clone();
                let row = shell.render_chat_row(
                    "b".into(),
                    "Second".into(),
                    "1m".into(),
                    "proj".into(),
                    None,
                    None,
                    Vec::new(),
                    None,
                    zeron_proto::ChatIndicator::Idle,
                    false,
                    None,
                    false,
                    false,
                    Some(SidebarSessionDrag {
                        chat_id: "b".into(),
                        visible_ids: Default::default(),
                        filter: None,
                        profile_key: "local".into(),
                    }),
                    None,
                    true,
                    false,
                    false,
                    None,
                    &theme,
                    cx,
                );
                let zones = shell.render_split_drop_zones(cx);
                // The column carries the real composer at its foot, as the
                // shell lays it out, so the halves' spacer has a pill to
                // stop above.
                let composer = shell.composer.clone();
                shell.viewport_height = 500.0;
                div()
                    .flex()
                    .flex_row()
                    .w(px(1000.))
                    .h(px(500.))
                    .child(div().w(px(240.)).child(row))
                    .child(
                        shell
                            .chat_dropzone("chat-dropzone", composer.clone(), cx)
                            .relative()
                            .flex_1()
                            .h_full()
                            .flex()
                            .flex_col()
                            .child(div().flex_1())
                            .child(composer)
                            .children(zones),
                    )
            })
        }
    }

    #[gpui::test]
    fn dragging_a_sidebar_row_onto_the_right_zone_opens_the_split(cx: &mut TestAppContext) {
        init(cx);
        let dir = tempfile::tempdir().unwrap();
        let (host, cx) = cx.add_window_view(|_, cx| {
            let shell = cx.new(|cx| {
                let state = cx.new(|_| AppState::new());
                let shell = Shell::new(state, boot_config(&dir), cx);
                seed(&shell, cx);
                shell
            });
            RowHost {
                shell,
                _data_dir: dir,
            }
        });
        let shell = host.read_with(cx, |host, _| host.shell.clone());
        cx.update(|window, cx| window.draw(cx).clear());
        let start = cx.debug_bounds("chat-b").unwrap().center();
        cx.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_move(
            start + gpui::point(px(12.), px(12.)),
            Some(MouseButton::Left),
            gpui::Modifiers::default(),
        );
        cx.update(|_, cx| assert!(cx.has_active_drag(), "row drag did not start"));
        cx.update(|window, cx| window.draw(cx).clear());
        let zone = cx
            .debug_bounds("split-drop-right")
            .expect("drop zones mount while the row is dragged")
            .center();
        cx.simulate_mouse_move(zone, Some(MouseButton::Left), gpui::Modifiers::default());
        cx.simulate_mouse_up(zone, MouseButton::Left, gpui::Modifiers::default());
        shell.read_with(cx, |shell, _| {
            assert_eq!(shell.split_chat_id(), Some("b"));
            assert_eq!(shell.split.as_ref().map(|s| s.side), Some(SplitSide::Right));
        });
    }

    /// The same row dragged onto the composer pill does not split: the
    /// halves yield above the pill and the drop lands in the composer as a
    /// thread mention link (the strict Markdown the chip projects from).
    #[gpui::test]
    fn dragging_a_sidebar_row_onto_the_composer_inserts_a_thread_mention(cx: &mut TestAppContext) {
        init(cx);
        let dir = tempfile::tempdir().unwrap();
        let (host, cx) = cx.add_window_view(|_, cx| {
            let shell = cx.new(|cx| {
                let state = cx.new(|_| AppState::new());
                let shell = Shell::new(state, boot_config(&dir), cx);
                seed(&shell, cx);
                shell
            });
            RowHost {
                shell,
                _data_dir: dir,
            }
        });
        let shell = host.read_with(cx, |host, _| host.shell.clone());
        cx.update(|window, cx| window.draw(cx).clear());
        let start = cx.debug_bounds("chat-b").unwrap().center();
        cx.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_move(
            start + gpui::point(px(12.), px(12.)),
            Some(MouseButton::Left),
            gpui::Modifiers::default(),
        );
        cx.update(|window, cx| window.draw(cx).clear());
        let pill = cx
            .debug_bounds("composer-surface")
            .expect("composer pill rendered")
            .center();
        cx.simulate_mouse_move(pill, Some(MouseButton::Left), gpui::Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear());
        shell.read_with(cx, |shell, _| {
            assert_eq!(
                shell.split_drop_hot.as_ref().and_then(|h| h.hot),
                None,
                "no split half is hot over the pill"
            );
        });
        cx.simulate_mouse_up(pill, MouseButton::Left, gpui::Modifiers::default());
        shell.read_with(cx, |shell, cx| {
            assert_eq!(
                shell.split_chat_id(),
                None,
                "a drop on the pill must not split"
            );
            let text = shell.composer.read(cx).input.read(cx).text().to_string();
            assert_eq!(text, "[b](zeron-thread:b) ");
        });
    }

    /// The real sidebar beside the drop zones, so one row drag can reach a
    /// custom section and a split half.
    struct SidebarHost {
        shell: Entity<Shell>,
        _data_dir: tempfile::TempDir,
    }

    impl Render for SidebarHost {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.shell.update(cx, |shell, cx| {
                let theme = Theme::of(cx).clone();
                let sidebar = shell.render_chat_sidebar(&theme, cx);
                let zones = shell.render_split_drop_zones(cx);
                shell.viewport_height = 800.0;
                div()
                    .flex()
                    .flex_row()
                    .w(px(1000.))
                    .h(px(800.))
                    .on_drag_move::<SidebarSessionDrag>(
                        cx.listener(Shell::contain_pinned_session_drag),
                    )
                    .on_drop::<SidebarSessionDrag>(
                        cx.listener(|shell, _, _, cx| shell.cancel_sidebar_session_transfer(cx)),
                    )
                    .child(div().w(px(280.)).h_full().child(sidebar))
                    .child(div().relative().flex_1().h_full().children(zones))
            })
        }
    }

    #[gpui::test]
    fn one_row_drag_lands_in_a_custom_section_and_a_split_zone(cx: &mut TestAppContext) {
        init(cx);
        cx.update(|cx| {
            crate::history::init(
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                cx,
            )
        });
        let dir = tempfile::tempdir().unwrap();
        let (host, cx) = cx.add_window_view(|_, cx| {
            let shell = cx.new(|cx| {
                let state = cx.new(|_| AppState::new());
                let mut shell = Shell::new(state, boot_config(&dir), cx);
                seed(&shell, cx);
                shell.state.update(cx, |state, _| {
                    state.local_device_id = Some("dev".into());
                });
                shell.settings.sidebar_width = 280.0;
                shell.settings.space_filter = None;
                shell.settings.sidebar_organization = SidebarOrganization::InOneList;
                shell.settings.sidebar_sections_by_profile.insert(
                    "local".into(),
                    vec![crate::settings::SidebarSection {
                        id: "focus".into(),
                        name: "Focus".into(),
                        session_ids: vec![],
                        collapsed: false,
                    }],
                );
                shell
            });
            SidebarHost {
                shell,
                _data_dir: dir,
            }
        });
        let shell = host.read_with(cx, |host, _| host.shell.clone());
        cx.update(|window, cx| window.draw(cx).clear());

        let start = cx.debug_bounds("chat-b").unwrap().center();
        cx.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_move(
            start + gpui::point(px(8.), px(0.)),
            Some(MouseButton::Left),
            gpui::Modifiers::default(),
        );
        let section = cx.debug_bounds("custom-section-focus").unwrap().center();
        cx.simulate_mouse_move(section, Some(MouseButton::Left), gpui::Modifiers::default());
        cx.simulate_mouse_up(section, MouseButton::Left, gpui::Modifiers::default());
        shell.update(cx, |shell, cx| {
            let sections = shell.active_sidebar_sections(cx);
            assert_eq!(sections[0].session_ids, ["b"]);
            assert!(shell.split_chat_id().is_none());
            shell.sidebar_session_return = None;
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear());

        let start = cx.debug_bounds("chat-b").unwrap().center();
        cx.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_move(
            start + gpui::point(px(8.), px(0.)),
            Some(MouseButton::Left),
            gpui::Modifiers::default(),
        );
        cx.update(|window, cx| window.draw(cx).clear());
        let zone = cx
            .debug_bounds("split-drop-right")
            .expect("drop zones mount while the row is dragged")
            .center();
        cx.simulate_mouse_move(zone, Some(MouseButton::Left), gpui::Modifiers::default());
        cx.simulate_mouse_up(zone, MouseButton::Left, gpui::Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear());
        shell.read_with(cx, |shell, cx| {
            assert_eq!(shell.split_chat_id(), Some("b"));
            assert_eq!(shell.split.as_ref().map(|s| s.side), Some(SplitSide::Right));
            assert_eq!(shell.active_sidebar_sections(cx)[0].session_ids, ["b"]);
            assert!(shell.sidebar_session_transfer.is_none());
        });
    }

    /// The real Shell as the window root (gate forced Ready): a strip tab
    /// dragged onto the left zone opens the split and the pane renders.
    #[gpui::test]
    fn dragging_a_strip_tab_onto_the_left_zone_opens_the_split(cx: &mut TestAppContext) {
        init(cx);
        let dir = tempfile::tempdir().unwrap();
        let (shell, cx) = cx.add_window_view(|_, cx| {
            let state = cx.new(|_| AppState::new());
            let mut shell = Shell::new(state, boot_config(&dir), cx);
            shell.debug_gate = Some(GatePhase::Ready);
            seed(&shell, cx);
            shell.settings.open_tabs = Some(vec!["a".into(), "b".into()]);
            shell.state.update(cx, |state, _| {
                state.sessions.push(zeron_proto::Session {
                    last_completed_turn: None,
                    held_by_subagents: false,
                    compacting: false,
                    background_tasks: Vec::new(),
                    chat_id: "b".into(),
                    device_id: "dev".into(),
                    status: zeron_proto::SessionStatus::Working,
                    started_at: None,
                    updated_at: chrono::Utc::now(),
                    running_subagents: 0,
                    compact_plan: None,
                    classifying: None,
                });
            });
            std::mem::forget(dir);
            shell
        });
        cx.update(|window, cx| window.draw(cx).clear());
        let start = cx
            .debug_bounds("session-tab-b")
            .expect("tab for b on the strip")
            .center();
        cx.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_move(
            start + gpui::point(px(0.), px(60.)),
            Some(MouseButton::Left),
            gpui::Modifiers::default(),
        );
        cx.update(|_, cx| assert!(cx.has_active_drag(), "tab drag did not start"));
        cx.update(|window, cx| window.draw(cx).clear());
        let zone = cx
            .debug_bounds("split-drop-left")
            .expect("drop zones mount while the tab is dragged")
            .center();
        cx.simulate_mouse_move(zone, Some(MouseButton::Left), gpui::Modifiers::default());
        cx.simulate_mouse_up(zone, MouseButton::Left, gpui::Modifiers::default());
        shell.read_with(cx, |shell, _| {
            assert_eq!(shell.split_chat_id(), Some("b"));
            assert_eq!(shell.split.as_ref().map(|s| s.side), Some(SplitSide::Left));
        });
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(
            cx.debug_bounds("split-pane").is_some(),
            "secondary pane not rendered"
        );
    }

    /// Holding the seam mounts a size veil over each pane, meeting at the
    /// seam; releasing unmounts both.
    #[gpui::test]
    fn holding_the_seam_shows_a_size_veil_over_each_pane(cx: &mut TestAppContext) {
        init(cx);
        let dir = tempfile::tempdir().unwrap();
        let (_shell, cx) = cx.add_window_view(|_, cx| {
            let state = cx.new(|_| AppState::new());
            let mut shell = Shell::new(state, boot_config(&dir), cx);
            shell.debug_gate = Some(GatePhase::Ready);
            seed(&shell, cx);
            shell.open_split("b".into(), SplitSide::Right, cx);
            std::mem::forget(dir);
            shell
        });
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(cx.debug_bounds("split-size-veil-left").is_none());
        let seam = cx.debug_bounds("split-resize").expect("seam handle");
        cx.simulate_mouse_down(seam.center(), MouseButton::Left, gpui::Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear());
        let left = cx.debug_bounds("split-size-veil-left").expect("left veil");
        let right = cx
            .debug_bounds("split-size-veil-right")
            .expect("right veil");
        let seam_x = f32::from(seam.center().x);
        assert!(
            (f32::from(left.right()) + 1.0 - seam_x).abs() <= 1.0,
            "{left:?} vs {seam_x}"
        );
        assert!(
            (f32::from(right.left()) - 1.0 - seam_x).abs() <= 1.0,
            "{right:?} vs {seam_x}"
        );
        assert_eq!(left.top(), right.top());
        cx.simulate_mouse_up(seam.center(), MouseButton::Left, gpui::Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(cx.debug_bounds("split-size-veil-left").is_none());
        assert!(cx.debug_bounds("split-size-veil-right").is_none());
    }

    /// Both panes of an open split wear the same header row, each with its
    /// own close, level with each other under the titlebar.
    #[gpui::test]
    fn both_panes_wear_a_header_with_a_close(cx: &mut TestAppContext) {
        init(cx);
        let dir = tempfile::tempdir().unwrap();
        let (_shell, cx) = cx.add_window_view(|_, cx| {
            let state = cx.new(|_| AppState::new());
            let mut shell = Shell::new(state, boot_config(&dir), cx);
            shell.debug_gate = Some(GatePhase::Ready);
            seed(&shell, cx);
            shell.open_split("b".into(), SplitSide::Right, cx);
            std::mem::forget(dir);
            shell
        });
        cx.update(|window, cx| window.draw(cx).clear());
        let primary = cx
            .debug_bounds("split-primary-close")
            .expect("primary pane close");
        let secondary = cx
            .debug_bounds("split-pane-close")
            .expect("secondary pane close");
        assert_eq!(primary.top(), secondary.top());
        assert!(primary.right() < secondary.left());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(left: &str, right: &str) -> SplitPair {
        SplitPair {
            left: left.into(),
            right: right.into(),
        }
    }

    fn drop(pair: Option<SplitPair>, select: Option<&str>) -> SplitDrop {
        SplitDrop {
            pair,
            select: select.map(str::to_string),
        }
    }

    fn ids(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    #[test]
    fn adjoin_moves_the_later_member_up_beside_the_earlier_one() {
        let mut order = ids(&["a", "b", "c", "d"]);
        pair("a", "d").adjoin(&mut order);
        assert_eq!(order, ids(&["a", "d", "b", "c"]));
        let mut order = ids(&["a", "b", "c", "d"]);
        pair("d", "b").adjoin(&mut order);
        assert_eq!(order, ids(&["a", "b", "d", "c"]));
        let mut order = ids(&["a", "b", "c"]);
        pair("a", "b").adjoin(&mut order);
        assert_eq!(order, ids(&["a", "b", "c"]));
    }

    #[test]
    fn adjoin_leaves_the_order_alone_when_a_member_is_missing() {
        let mut order = ids(&["a", "b", "c"]);
        pair("a", "z").adjoin(&mut order);
        assert_eq!(order, ids(&["a", "b", "c"]));
    }

    #[test]
    fn adjoin_grouped_pulls_the_later_member_into_the_earlier_group() {
        let mut groups = vec![("one", ids(&["a", "b"])), ("two", ids(&["c", "d"]))];
        pair("d", "a").adjoin_grouped(&mut groups, String::as_str);
        assert_eq!(groups[0].1, ids(&["a", "d", "b"]));
        assert_eq!(groups[1].1, ids(&["c"]));
        let mut groups = vec![("one", ids(&["a", "b", "c"]))];
        pair("a", "c").adjoin_grouped(&mut groups, String::as_str);
        assert_eq!(groups[0].1, ids(&["a", "c", "b"]));
    }

    #[test]
    fn drop_beside_a_selected_chat_pairs_them() {
        assert_eq!(
            split_drop("b", SplitSide::Right, Some("a"), None),
            drop(Some(pair("a", "b")), None)
        );
        assert_eq!(
            split_drop("b", SplitSide::Left, Some("a"), None),
            drop(Some(pair("b", "a")), None)
        );
    }

    #[test]
    fn dropping_the_selected_chat_changes_nothing() {
        assert_eq!(
            split_drop("a", SplitSide::Left, Some("a"), None),
            drop(None, None)
        );
        assert_eq!(
            split_drop("a", SplitSide::Left, Some("a"), Some(&pair("a", "b"))),
            drop(Some(pair("a", "b")), None)
        );
    }

    #[test]
    fn drop_on_the_canvas_selects() {
        assert_eq!(
            split_drop("a", SplitSide::Left, None, None),
            drop(None, Some("a"))
        );
    }

    #[test]
    fn drop_on_the_partner_side_replaces_it() {
        assert_eq!(
            split_drop("c", SplitSide::Right, Some("a"), Some(&pair("a", "b"))),
            drop(Some(pair("a", "c")), None)
        );
    }

    #[test]
    fn drop_on_the_primary_side_replaces_and_selects() {
        assert_eq!(
            split_drop("c", SplitSide::Left, Some("a"), Some(&pair("a", "b"))),
            drop(Some(pair("c", "b")), Some("c"))
        );
    }

    #[test]
    fn dropping_a_member_on_the_other_side_flips_the_pair() {
        assert_eq!(
            split_drop("a", SplitSide::Right, Some("a"), Some(&pair("a", "b"))),
            drop(Some(pair("b", "a")), None)
        );
        assert_eq!(
            split_drop("b", SplitSide::Left, Some("a"), Some(&pair("a", "b"))),
            drop(Some(pair("b", "a")), None)
        );
    }

    #[test]
    fn a_full_width_session_pairs_afresh() {
        assert_eq!(
            split_drop("d", SplitSide::Right, Some("c"), Some(&pair("a", "b"))),
            drop(Some(pair("c", "d")), None)
        );
    }

    #[test]
    fn partner_lookup_follows_the_pair_order() {
        let p = pair("a", "b");
        assert_eq!(p.partner("a"), Some(("b", SplitSide::Right)));
        assert_eq!(p.partner("b"), Some(("a", SplitSide::Left)));
        assert_eq!(p.partner("c"), None);
    }
}
