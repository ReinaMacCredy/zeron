use super::*;
use crate::pickers::HandoffTarget;
use zeron_doc::{MessageRole, MessageStatus};
use zeron_proto::handoff;

impl Shell {
    fn handoff_blocked(&self, cx: &App) -> bool {
        let state = self.state.read(cx);
        let Some(chat) = state.selected_chat_row() else {
            return true;
        };
        let finished = state.transcript_replayed
            && state.transcript.iter().any(|entry| {
                matches!(entry.role, MessageRole::User | MessageRole::Assistant)
                    && entry.status != Some(MessageStatus::Streaming)
            });
        handoff::handoff_blocked(state.display_status_for(chat, Utc::now()), finished)
    }

    fn in_place_handoff_targets(&self, cx: &App) -> Vec<HandoffTarget> {
        let current = self
            .state
            .read(cx)
            .selected_chat_row()
            .and_then(|chat| chat.config.as_ref())
            .map(|config| config.harness);
        let mut targets = self.composer.read(cx).pickers().read(cx).handoff_targets();
        targets.retain(|target| Some(target.harness) != current);
        targets
    }

    pub(super) fn handoff_disabled(&self, cx: &App) -> bool {
        self.handoff_blocked(cx)
            || self
                .composer
                .read(cx)
                .pickers()
                .read(cx)
                .handoff_targets()
                .is_empty()
    }

    pub(super) fn open_handoff_menu(&mut self, cx: &mut Context<Self>) {
        if !self.handoff_disabled(cx) {
            self.handoff_menu.open(());
            cx.notify();
        }
    }

    fn close_handoff_menu(&mut self, cx: &mut Context<Self>) {
        if self.handoff_menu.begin_close() {
            popover::reap_popup(cx, |shell: &mut Self| &mut shell.handoff_menu);
        }
        cx.notify();
    }

    pub(super) fn render_handoff_button(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let disabled = self.handoff_disabled(cx);
        if disabled && self.handoff_menu.is_open() {
            self.close_handoff_menu(cx);
        }
        let button = header_icon_button(
            "titlebar-handoff",
            icons::LUCIDE_ARROW_LEFT_RIGHT,
            handoff::HAND_OFF_THREAD,
            theme,
            cx.listener(|this, _, _, cx| {
                if this.handoff_menu.take_press_was_open() {
                    this.close_handoff_menu(cx);
                } else {
                    this.open_handoff_menu(cx);
                }
            }),
        )
        .role(gpui::Role::Button)
        .aria_label(handoff::HAND_OFF_THREAD)
        .aria_expanded(self.handoff_menu.is_open())
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, _| this.handoff_menu.note_trigger_press()),
        )
        .when(disabled, |button| button.opacity(0.4).cursor_default());
        let menu = self
            .handoff_menu
            .get()
            .is_some()
            .then(|| self.render_handoff_menu(theme, cx));
        div()
            .relative()
            .flex_none()
            .child(button)
            .children(menu)
            .into_any_element()
    }

    fn render_handoff_menu(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let popup = theme.for_popup();
        let in_place: Vec<AnyElement> = self
            .in_place_handoff_targets(cx)
            .into_iter()
            .map(|target| {
                handoff_row(&popup, "this", target, cx, |this, harness, model, cx| {
                    let pickers = this.composer.read(cx).pickers().clone();
                    pickers.update(cx, |pickers, cx| {
                        pickers.hand_off_in_place(harness, model, cx)
                    });
                })
            })
            .collect();
        let new_thread: Vec<AnyElement> = self
            .composer
            .read(cx)
            .pickers()
            .read(cx)
            .handoff_targets()
            .into_iter()
            .map(|target| {
                handoff_row(&popup, "new", target, cx, |this, harness, model, cx| {
                    if let Some(chat_id) = this.state.read(cx).selected_chat.clone() {
                        let picks = this
                            .composer
                            .read(cx)
                            .pickers()
                            .read(cx)
                            .new_thread_picks(harness, &model);
                        let target = Some((harness, model, picks));
                        this.continue_in_new_thread(chat_id, target, None, cx);
                    }
                })
            })
            .collect();
        let state = self.state.read(cx);
        let device = state
            .selected_chat_row()
            .filter(|chat| chat.space_id.is_none())
            .map(|chat| chat.device_id.as_str());
        let projects: Vec<(String, String)> = state
            .spaces
            .iter()
            .filter(|space| Some(space.device_id.as_str()) == device)
            .map(|space| (space.id.clone(), space.display_name().to_string()))
            .collect();
        let projects: Vec<AnyElement> = projects
            .into_iter()
            .map(|(space_id, name)| handoff_project_row(&popup, space_id, name, cx))
            .collect();
        let has_in_place = !in_place.is_empty();
        let has_new_thread = !new_thread.is_empty();
        let card = popover::popover_card(&popup)
            .id("handoff-menu-card")
            .w(px(224.0))
            .p(px(4.0))
            .flex()
            .flex_col()
            .gap(px(popover::MENU_GAP))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_handoff_menu(cx)))
            .when(!in_place.is_empty(), |card| {
                card.child(popover::menu_heading(
                    &popup,
                    handoff::CONTINUE_IN_THIS_THREAD,
                ))
                .children(in_place)
            })
            .when(!new_thread.is_empty(), |card| {
                card.when(has_in_place, |card| card.child(popover::menu_separator()))
                    .child(popover::menu_heading(
                        &popup,
                        handoff::CONTINUE_IN_A_NEW_THREAD,
                    ))
                    .children(new_thread)
            })
            .when(!projects.is_empty(), |card| {
                card.when(has_in_place || has_new_thread, |card| {
                    card.child(popover::menu_separator())
                })
                .child(popover::menu_heading(&popup, handoff::HAND_OFF_TO_PROJECT))
                .children(projects)
            })
            .into_any_element();
        popover::anchored_menu_below_end("handoff-menu", card, self.handoff_menu.closing_since())
    }
}

fn handoff_row(
    popup: &Theme,
    group: &str,
    target: HandoffTarget,
    cx: &mut Context<Shell>,
    pick: fn(&mut Shell, HarnessId, String, &mut Context<Shell>),
) -> AnyElement {
    let (mark, tint) = crate::pickers::harness_brand_icon(target.harness);
    let id = SharedString::from(format!("handoff-{group}-{:?}", target.harness));
    let selector = id.clone();
    let HandoffTarget {
        harness,
        name,
        model,
        model_label,
    } = target;
    popover::menu_row(popup, false, id.clone())
        .id(id)
        .debug_selector(move || selector.to_string())
        .role(gpui::Role::MenuItem)
        .gap(px(8.0))
        .child(
            icon(mark)
                .size(px(14.0))
                .flex_none()
                .text_color(tint.unwrap_or(popup.text_muted)),
        )
        .child(
            div()
                .min_w_0()
                .truncate()
                .child(format!("{name} · {model_label}")),
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            this.close_handoff_menu(cx);
            pick(this, harness, model.clone(), cx);
        }))
        .into_any_element()
}

fn handoff_project_row(
    popup: &Theme,
    space_id: String,
    name: String,
    cx: &mut Context<Shell>,
) -> AnyElement {
    let id = SharedString::from(format!("handoff-project-{space_id}"));
    let selector = id.clone();
    popover::menu_row(popup, false, id.clone())
        .id(id)
        .debug_selector(move || selector.to_string())
        .role(gpui::Role::MenuItem)
        .gap(px(8.0))
        .child(
            icon(icons::LUCIDE_FOLDER)
                .size(px(14.0))
                .flex_none()
                .text_color(popup.text_muted),
        )
        .child(div().min_w_0().truncate().child(name))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.close_handoff_menu(cx);
            if let Some(chat_id) = this.state.read(cx).selected_chat.clone() {
                this.continue_in_new_thread(chat_id, None, Some(space_id.clone()), cx);
            }
        }))
        .into_any_element()
}
