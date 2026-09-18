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

/// Which drop zone the pointer is over during a drag; `epoch` keys the
/// grow/shrink animation restarts (the tab strip's slide idiom).
#[derive(Debug, Default)]
pub(super) struct SplitDropHot {
    pub(super) hot: Option<SplitSide>,
    pub(super) prev: Option<SplitSide>,
    pub(super) epoch: usize,
}

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

impl Shell {
    /// The chat shown in the secondary pane right now.
    pub(super) fn split_chat_id(&self) -> Option<&str> {
        self.split.as_ref().map(|s| s.chat_id.as_str())
    }

    pub(super) fn split_pair_contains(&self, chat: &str) -> bool {
        self.split_pair.as_ref().is_some_and(|p| p.contains(chat))
    }

    pub(super) fn split_composer_for(&self, chat: &str) -> Option<Entity<Composer>> {
        self.split
            .as_ref()
            .filter(|s| s.chat_id == chat)
            .map(|s| s.composer.clone())
    }

    /// Build the secondary pane over `chat_id` on `side`, replacing any
    /// current secondary.
    fn open_split(&mut self, chat_id: String, side: SplitSide, cx: &mut Context<Self>) {
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
                .any(|tab| tab.doc_id == split.chat_id);
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

    /// Selection changed (every path lands in `on_state_changed`): a
    /// selected pair member shows its partner beside it; anything else shows
    /// full width.
    pub(super) fn reconcile_split_with_selection(
        &mut self,
        selected: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let partner = selected.and_then(|selected| {
            self.split_pair
                .as_ref()
                .and_then(|pair| pair.partner(selected))
                .map(|(chat, side)| (chat.to_string(), side))
        });
        match partner {
            Some((chat, side)) => self.open_split(chat, side, cx),
            None => self.close_split(cx),
        }
    }

    /// A pair member vanished (archived, deleted): the pair goes.
    pub(super) fn prune_split(&mut self, cx: &mut Context<Self>) {
        let Some(pair) = self.split_pair.clone() else {
            return;
        };
        let live = {
            let state = self.state.read(cx);
            !state.chats_synced
                || [&pair.left, &pair.right]
                    .iter()
                    .all(|id| state.chats.iter().any(|c| c.id == **id && !c.archived))
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
        if self.state.read(cx).is_draft(&chat_id) {
            return;
        }
        let selected = self.state.read(cx).selected_chat.clone();
        let verdict = split_drop(
            &chat_id,
            side,
            selected.as_deref(),
            self.split_pair.as_ref(),
        );
        tracing::info!(chat = %chat_id, ?side, ?selected, pair = ?self.split_pair, ?verdict, "split: drop verdict");
        let SplitDrop { pair, select } = verdict;
        self.split_pair = pair;
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
    /// layer: no zone is hot).
    fn split_zone_at(bounds: gpui::Bounds<Pixels>, position: Point<Pixels>) -> Option<SplitSide> {
        if !bounds.contains(&position) {
            return None;
        }
        Some(if position.x < bounds.center().x {
            SplitSide::Left
        } else {
            SplitSide::Right
        })
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
                        this.dissolve_split_pair(cx);
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
                .debug_selector(|| "split-pane".into())
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
            state.read(cx).is_draft(chat_id) || selected.as_deref() == Some(chat_id)
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
                .on_drop::<SessionRowDrag>(cx.listener(
                    move |this, payload: &SessionRowDrag, _, cx| {
                        tracing::info!(chat = %payload.chat_id, ?side, "split: row dropped");
                        this.drop_session_on_side(payload.chat_id.clone(), side, cx);
                    },
                ))
                .child(card)
        };
        let left = zone(SplitSide::Left, cx);
        let right = zone(SplitSide::Right, cx);
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
                .flex_row()
                .opacity(0.0)
                .bg(theme.scrim().opacity(0.35))
                .on_drag_move::<tabs::TabDragPayload>(cx.listener(
                    |this, event: &gpui::DragMoveEvent<tabs::TabDragPayload>, _, cx| {
                        let side = Self::split_zone_at(event.bounds, event.event.position);
                        this.set_split_drop_hot(side, cx);
                    },
                ))
                .on_drag_move::<SessionRowDrag>(cx.listener(
                    |this, event: &gpui::DragMoveEvent<SessionRowDrag>, _, cx| {
                        let side = Self::split_zone_at(event.bounds, event.event.position);
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
                .drag_over::<SessionRowDrag>(move |style, payload, _, cx| {
                    style.opacity(if inert(&payload.chat_id, cx) {
                        0.3
                    } else {
                        1.0
                    })
                })
                .child(left)
                .child(right)
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
                    None,
                    zeron_proto::ChatIndicator::Idle,
                    false,
                    false,
                    None,
                    None,
                    &theme,
                    cx,
                );
                let zones = shell.render_split_drop_zones(cx);
                div()
                    .flex()
                    .flex_row()
                    .w(px(1000.))
                    .h(px(500.))
                    .child(div().w(px(240.)).child(row))
                    .child(div().relative().flex_1().h_full().children(zones))
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
                    chat_id: "b".into(),
                    device_id: "dev".into(),
                    status: zeron_proto::SessionStatus::Working,
                    started_at: None,
                    updated_at: chrono::Utc::now(),
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
