//! The pinned summary: a titlebar card, left of the right-pane toggle, that
//! indexes what the session produced — output surfaces (previews, files,
//! diffs), the subagents it spawned, and the images the user attached. Each
//! row focuses the thing it names; the card itself is a transient popover.

use super::*;

const CARD_WIDTH: f32 = 300.0;
/// Rows shown per section before the tail folds into "+N more".
const SECTION_CAP: usize = 8;

impl Shell {
    pub(super) fn render_summary_trigger(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let open = self.summary_menu.get().is_some();
        let fade_key = "header-icon-pinned-summary";
        let mut trigger = div()
            .id("pinned-summary")
            .debug_selector(|| "pinned-summary".into())
            .size(px(28.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .bg(motion::hover_blend(
                fade_key,
                crate::theme::wash(0.0),
                crate::theme::wash(0.11),
            ))
            .on_hover(motion::hover_listener(fade_key))
            .tooltip(|_, cx| {
                cx.new(|_| SurfaceTabTooltip {
                    text: "Toggle pinned summary".into(),
                })
                .into()
            })
            .tooltip_show_delay(Duration::from_millis(350))
            // Sits inside the titlebar drag region like [`header_icon_button`].
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, _| {
                    window.prevent_default();
                    this.summary_menu.note_trigger_press();
                }),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                if this.summary_menu.take_press_was_open() {
                    this.close_summary_menu(cx);
                } else {
                    this.summary_menu.open(());
                    cx.notify();
                }
            }))
            .child(icon(icons::CHECKLIST).size(px(16.0)).text_color(if open {
                theme.text
            } else {
                theme.text_muted
            }));
        if open {
            let closing = self.summary_menu.closing_since();
            let card = self.render_summary_card(&theme, cx);
            trigger = trigger.relative().child(popover::anchored_menu_below_end(
                "pinned-summary-menu",
                card,
                closing,
            ));
        }
        trigger.into_any_element()
    }

    pub(super) fn close_summary_menu(&mut self, cx: &mut Context<Self>) {
        if self.summary_menu.begin_close() {
            popover::reap_popup(cx, |shell: &mut Self| &mut shell.summary_menu);
        }
        cx.notify();
    }

    /// Bring one right-pane surface on screen: open the pane if it is closed,
    /// then make that tab active.
    fn focus_right_surface(&mut self, surface: RightSurface, cx: &mut Context<Self>) {
        if !self.right_pane_open(cx) {
            self.toggle_right_pane(cx);
        }
        self.set_right_active(surface, cx);
    }

    fn render_summary_card(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let outputs: Vec<(RightSurface, SharedString, &'static str)> = self
            .right_surface_rows(cx)
            .into_iter()
            .filter_map(|(surface, title, _, _)| {
                let icon_path = match surface {
                    RightSurface::Browser(_) => icons::GLOBE,
                    RightSurface::File(_) => icons::DOCUMENT,
                    RightSurface::Diff(_) => icons::LIST,
                    _ => return None,
                };
                Some((surface, title, icon_path))
            })
            .collect();
        let subagents = self.transcript.read(cx).subagent_summaries();
        let sources = self.transcript.read(cx).user_attachments();
        let muted = theme.text_muted;

        let mut card = popover::popover_card(theme)
            .w(px(CARD_WIDTH))
            .flex()
            .flex_col()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_summary_menu(cx)));

        if outputs.is_empty() && subagents.is_empty() && sources.is_empty() {
            return card
                .child(
                    div()
                        .px(px(8.0))
                        .py(px(10.0))
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(muted)
                        .child(SharedString::from(
                            "Nothing to pin yet. Previews, files, subagents and attached images collect here as the session works.",
                        )),
                )
                .into_any_element();
        }

        let row = |theme: &Theme, key: &'static str, ix: usize, icon_path: &'static str| {
            popover::menu_row(theme, false, format!("{key}-{ix}"))
                .id((key, ix))
                .debug_selector(|| format!("{key}-{ix}"))
                .child(icon(icon_path).size(px(13.0)).text_color(muted))
        };
        let label = |text: SharedString| div().flex_1().min_w_0().truncate().child(text);
        let more = |theme: &Theme, hidden: usize| {
            div()
                .px(px(8.0))
                .py(px(4.0))
                .text_size(crate::typography::ui_rems(11.0))
                .text_color(theme.text_muted)
                .child(SharedString::from(format!("+{hidden} more")))
        };

        let has_outputs = !outputs.is_empty();
        if has_outputs {
            let hidden = outputs.len().saturating_sub(SECTION_CAP);
            card = card.child(popover::menu_heading(theme, "Outputs"));
            for (ix, (surface, title, icon_path)) in
                outputs.into_iter().take(SECTION_CAP).enumerate()
            {
                card = card.child(
                    row(theme, "pinned-summary-output", ix, icon_path)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.focus_right_surface(surface, cx);
                            this.close_summary_menu(cx);
                        }))
                        .child(label(title)),
                );
            }
            card = card.when(hidden > 0, |card| card.child(more(theme, hidden)));
        }

        let has_subagents = !subagents.is_empty();
        if has_subagents {
            let hidden = subagents.len().saturating_sub(SECTION_CAP);
            card = card
                .when(has_outputs, |card| card.child(popover::menu_separator()))
                .child(popover::menu_heading(theme, "Subagents"));
            for (ix, agent) in subagents.into_iter().take(SECTION_CAP).enumerate() {
                let (state, tone) = match agent.status {
                    Some(zeron_doc::SubagentStatus::Running) => ("working", theme.warning),
                    Some(zeron_doc::SubagentStatus::Failed) => ("failed", theme.danger),
                    Some(zeron_doc::SubagentStatus::Done) | None => ("done", muted),
                };
                let frozen = !matches!(agent.status, Some(zeron_doc::SubagentStatus::Running));
                let title = agent.title.clone();
                card = card.child(
                    row(theme, "pinned-summary-subagent", ix, icons::BOT)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.add_subagent_surface(
                                agent.chat_id.clone(),
                                agent.doc_id.clone(),
                                agent.title.to_string(),
                                frozen,
                                cx,
                            );
                            this.close_summary_menu(cx);
                        }))
                        .child(label(title))
                        .child(
                            div()
                                .flex_none()
                                .text_size(crate::typography::ui_rems(11.0))
                                .text_color(tone)
                                .child(SharedString::from(state)),
                        ),
                );
            }
            card = card.when(hidden > 0, |card| card.child(more(theme, hidden)));
        }

        if !sources.is_empty() {
            let hidden = sources.len().saturating_sub(SECTION_CAP);
            card = card
                .when(has_outputs || has_subagents, |card| {
                    card.child(popover::menu_separator())
                })
                .child(popover::menu_heading(theme, "Sources"));
            for (ix, attachment) in sources.into_iter().take(SECTION_CAP).enumerate() {
                let name = SharedString::from(attachment.name.clone());
                card = card.child(
                    row(theme, "pinned-summary-source", ix, icons::FILE_IMAGE)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.transcript.update(cx, |transcript, cx| {
                                transcript.preview_attachment(&attachment, window, cx);
                            });
                            this.close_summary_menu(cx);
                        }))
                        .child(label(name)),
                );
            }
            card = card.when(hidden > 0, |card| card.child(more(theme, hidden)));
        }

        card = card.child(popover::menu_separator()).child(
            popover::menu_row(theme, false, "pinned-summary-view-all")
                .id("pinned-summary-view-all")
                .on_click(cx.listener(|this, _, _, cx| {
                    if !this.right_pane_open(cx) {
                        this.toggle_right_pane(cx);
                    }
                    this.close_summary_menu(cx);
                }))
                .child(
                    icon(icons::SIDEBAR_MINIMALISTIC)
                        .size(px(13.0))
                        .text_color(muted),
                )
                .child(SharedString::from("View all")),
        );
        card.into_any_element()
    }
}
