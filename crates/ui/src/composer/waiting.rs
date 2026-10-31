use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use gpui::{
    AnyElement, App, Bounds, Context, FocusHandle, KeyDownEvent, MouseButton, Pixels, SharedString,
    Stateful, Window, div, prelude::*, px,
};

use super::Composer;
use super::ComposerEvent;
use super::background::{
    CardMotion, card_frame, card_motion, eased, entering, frosted_pill, pill_enter, pill_exit,
    pill_frame, pill_leading, presence,
};
use crate::icons::{self, icon};
use crate::motion;
use crate::plan_proposal::PlanTarget;
use crate::popover;
use crate::theme::Theme;

const CARD_WIDTH: f32 = 520.0;
const TAB_TITLE_WIDTH: f32 = 180.0;
const SWITCH_SHIFT: f32 = 8.0;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WaitItem {
    pub key: String,
    pub chat_id: String,
    pub title: String,
    pub from: String,
    pub kind: WaitKind,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum WaitKind {
    Ask,
    Plan {
        task_id: Option<String>,
        steps: Vec<String>,
    },
    Decision {
        item_id: String,
        label: &'static str,
        detail: String,
        options: Vec<String>,
    },
}

#[derive(Debug, Clone)]
pub enum WaitingAction {
    ApprovePlan {
        chat_id: String,
        target: PlanTarget,
        as_task: bool,
    },
    Decide {
        item_id: String,
        choice: String,
    },
    Open {
        chat_id: String,
        decision: bool,
    },
}

#[derive(Default)]
pub(super) struct WaitingUi {
    items: Vec<WaitItem>,
    card: popover::Popup<Instant>,
    current: Option<String>,
    current_ix: usize,
    switched_at: Option<Instant>,
    focus: Option<FocusHandle>,
    focus_pending: bool,
    pill_since: Option<Instant>,
    shown: usize,
    ghost: Option<(usize, Instant)>,
    pill_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    row_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
}

impl Composer {
    pub(crate) fn set_waiting(&mut self, items: Vec<WaitItem>, cx: &mut Context<Self>) {
        if self.waiting.items != items {
            self.waiting.items = items;
            cx.notify();
        }
    }

    fn visible_waiting(&self, cx: &App) -> Vec<WaitItem> {
        if !self.hosts_asks() {
            return Vec::new();
        }
        let bound = self.bound_chat_id(cx);
        let elsewhere = |chat_id: &str| bound.as_deref() != Some(chat_id);
        self.waiting
            .items
            .iter()
            .filter(|item| match &item.kind {
                WaitKind::Ask => elsewhere(&item.chat_id) && self.ask_info(&item.chat_id).is_some(),
                WaitKind::Plan { task_id: None, .. } => {
                    elsewhere(&item.chat_id) && self.plan_answer(&item.chat_id).is_some()
                }
                _ => true,
            })
            .cloned()
            .collect()
    }

    pub(crate) fn waiting_open(&self) -> bool {
        self.waiting.card.is_open()
    }

    pub(crate) fn open_waiting(&mut self, key: Option<String>, cx: &mut Context<Self>) {
        if !self.waiting.card.is_open() {
            self.waiting.card.open(motion::now());
            self.waiting.switched_at = None;
            self.waiting.current = None;
            self.waiting.current_ix = 0;
        }
        if let Some(key) = key {
            self.select_waiting(key, cx);
        }
        self.close_roster(cx);
        self.close_shells(cx);
        self.waiting.focus_pending = true;
        cx.notify();
    }

    /// Defer, never cancel: an open question keeps its page, picks and typed text.
    pub(crate) fn close_waiting(&mut self, cx: &mut Context<Self>) {
        if self.waiting.card.begin_close() {
            popover::reap_popup(cx, |this: &mut Composer| &mut this.waiting.card);
            self.focus_pending = true;
        }
        cx.notify();
    }

    fn select_waiting(&mut self, key: String, cx: &mut Context<Self>) {
        if self.waiting.current.as_deref() == Some(key.as_str()) {
            return;
        }
        if self.waiting.current.is_some() {
            self.waiting.switched_at = Some(motion::now());
        }
        self.waiting.current = Some(key);
        cx.notify();
    }

    fn act_waiting(&mut self, action: WaitingAction, cx: &mut Context<Self>) {
        if matches!(action, WaitingAction::Open { .. }) {
            self.close_waiting(cx);
        }
        cx.emit(ComposerEvent::Waiting(action));
    }

    pub(super) fn waiting_row_probe(&self) -> AnyElement {
        let cell = self.waiting.row_bounds.clone();
        gpui::canvas(move |bounds, _, _| cell.set(Some(bounds)), |_, _, _, _| {})
            .absolute()
            .inset_0()
            .into_any_element()
    }

    pub(super) fn render_waiting(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Option<AnyElement>, Option<AnyElement>) {
        let items = self.visible_waiting(cx);
        let theme = Theme::of(cx).clone();
        let now = motion::now();
        let reduced = motion::reduced_motion(cx);

        if items.is_empty() && self.waiting.card.is_open() {
            self.close_waiting(cx);
        }
        let current = match self
            .waiting
            .current
            .as_ref()
            .and_then(|key| items.iter().position(|item| &item.key == key))
        {
            Some(ix) => Some(ix),
            None if items.is_empty() => None,
            None => {
                let ix = self.waiting.current_ix.min(items.len() - 1);
                if self.waiting.current.is_some() {
                    self.waiting.switched_at = Some(now);
                }
                self.waiting.current = Some(items[ix].key.clone());
                Some(ix)
            }
        };
        if let Some(ix) = current {
            self.waiting.current_ix = ix;
        }

        let count = items.len();
        if count == 0 && self.waiting.shown > 0 {
            self.waiting.ghost = Some((self.waiting.shown, now));
        } else if count > 0 {
            self.waiting.ghost = None;
        }
        self.waiting.shown = count;
        let pill_in = pill_enter(
            presence(&mut self.waiting.pill_since, count > 0, now),
            reduced,
            now,
        );
        let ghost_out = self
            .waiting
            .ghost
            .map(|(_, since)| pill_exit(since, reduced, now));
        if ghost_out.is_some_and(|out| out >= 1.0) {
            self.waiting.ghost = None;
        }

        let motion =
            self.waiting.card.get().map(|opened| {
                card_motion(*opened, self.waiting.card.closing_since(), reduced, now)
            });
        let card = motion
            .filter(|motion| !motion.gone)
            .zip(current)
            .map(|(motion, ix)| self.render_waiting_card(&items, ix, motion, &theme, window, cx));
        if card.is_none() || current.is_none_or(|ix| !matches!(items[ix].kind, WaitKind::Ask)) {
            self.close_ask(cx);
        }

        let active = self.waiting.card.is_open();
        let pill = (count > 0).then(|| {
            let cell = self.waiting.pill_bounds.clone();
            let pill = waiting_pill_body(pill_frame("waiting-pill", &theme), count, active, &theme)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, _| this.waiting.card.note_trigger_press()),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if !this.waiting.card.take_press_was_open() {
                        this.open_waiting(None, cx);
                    }
                }))
                .child(
                    gpui::canvas(move |bounds, _, _| cell.set(Some(bounds)), |_, _, _, _| {})
                        .absolute()
                        .inset_0(),
                );
            frosted_pill(entering(pill, pill_in), pill_in).into_any_element()
        });
        let ghost = self.waiting.ghost.zip(ghost_out).map(|((shown, _), out)| {
            let opacity = 1.0 - out;
            let pill = waiting_pill_body(
                pill_frame("waiting-pill-ghost", &theme).opacity(opacity),
                shown,
                false,
                &theme,
            );
            frosted_pill(pill, opacity).into_any_element()
        });
        let switching = self.waiting.switched_at.is_some_and(|since| {
            eased(&motion::SIDECAR_POPOVER_FADE, Some(since), reduced, now) < 1.0
        });
        if pill_in < 1.0
            || ghost.is_some()
            || switching
            || motion.is_some_and(|motion| motion.animating)
        {
            window.request_animation_frame();
        }
        (pill.or(ghost), card)
    }

    fn render_waiting_card(
        &mut self,
        items: &[WaitItem],
        ix: usize,
        motion: CardMotion,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let item = items[ix].clone();
        let reduced = motion::reduced_motion(cx);
        let switch = eased(
            &motion::SIDECAR_POPOVER_FADE,
            self.waiting.switched_at,
            reduced,
            motion::now(),
        );
        let focus = self
            .waiting
            .focus
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        if motion.live && std::mem::take(&mut self.waiting.focus_pending) {
            window.focus(&focus, cx);
        }

        let tabs = items.iter().enumerate().map(|(tab_ix, tab)| {
            let selected = tab_ix == ix;
            let key = tab.key.clone();
            div()
                .id(("waiting-tab", tab_ix))
                .debug_selector(move || format!("waiting-tab-{tab_ix}"))
                .role(gpui::Role::Tab)
                .aria_label(SharedString::from(tab.title.clone()))
                .aria_selected(selected)
                .when(selected, |el| el.flex_none())
                .when(!selected, |el| el.min_w(px(32.0)).overflow_hidden())
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .h(px(26.0))
                .px(px(8.0))
                .rounded(px(7.0))
                .cursor_pointer()
                .text_color(if selected {
                    theme.text
                } else {
                    theme.text_muted
                })
                .when(selected, |el| el.bg(theme.wash(0.08)))
                .when(!selected, |el| {
                    el.hover(|s| s.bg(theme.glass_hover()).text_color(theme.text))
                })
                .child(kind_glyph(&tab.kind, theme))
                .child(
                    div()
                        .min_w_0()
                        .max_w(px(TAB_TITLE_WIDTH))
                        .truncate()
                        .child(SharedString::from(tab.title.clone())),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.select_waiting(key.clone(), cx)))
        });
        let header = div()
            .h(px(38.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .pl(px(10.0))
            .pr(px(8.0))
            .border_b_1()
            .border_color(crate::theme::hairline(0.06))
            .text_size(crate::typography::ui_rems(12.5))
            .child(
                div()
                    .id("waiting-tabs")
                    .role(gpui::Role::TabList)
                    .aria_label("Waiting for you")
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .gap(px(4.0))
                    .overflow_hidden()
                    .children(tabs),
            )
            .child(
                div()
                    .id("waiting-close")
                    .debug_selector(|| "waiting-close".into())
                    .role(gpui::Role::Button)
                    .aria_label("Close")
                    .size(px(22.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.glass_hover()))
                    .child(
                        icon(icons::CLOSE)
                            .size(px(12.0))
                            .text_color(theme.text_muted),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.close_waiting(cx))),
            );

        let who = div()
            .px(px(14.0))
            .pt(px(12.0))
            .text_size(crate::typography::ui_rems(11.5))
            .text_color(theme.text_muted)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(4.0))
                    .child(
                        div()
                            .text_color(theme.accent)
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(kind_label(&item.kind)),
                    )
                    .when(!item.from.is_empty(), |el| {
                        el.child(SharedString::from(format!("· {}", item.from)))
                    }),
            );
        let body = match &item.kind {
            WaitKind::Ask => self.render_ask_panel(&item.chat_id, window, cx),
            WaitKind::Plan { task_id, steps } => {
                let target = match task_id {
                    Some(task_id) => Some(PlanTarget::Task(task_id.clone())),
                    None => self.plan_answer(&item.chat_id).map(PlanTarget::Question),
                };
                target.map(|target| self.render_plan_body(&item.chat_id, steps, target, theme, cx))
            }
            WaitKind::Decision {
                item_id,
                detail,
                options,
                ..
            } => Some(self.render_decision_body(item_id, &item.title, detail, options, theme, cx)),
        };
        let body = div()
            .relative()
            .left(px(SWITCH_SHIFT * (1.0 - switch)))
            .opacity(switch)
            .child(who)
            .children(body);

        let open_target = item.chat_id.clone();
        let decision = matches!(item.kind, WaitKind::Decision { .. });
        let footer_link = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .role(gpui::Role::Button)
                .aria_label(label)
                .px(px(6.0))
                .h(px(22.0))
                .flex()
                .items_center()
                .rounded(px(6.0))
                .cursor_pointer()
                .text_color(theme.text_muted)
                .hover(|s| s.bg(theme.glass_hover()).text_color(theme.text))
                .child(label)
        };
        let dismiss = match &item.kind {
            WaitKind::Decision { item_id, .. } => {
                let item_id = item_id.clone();
                Some(
                    footer_link("waiting-dismiss", "Dismiss").on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.act_waiting(
                                WaitingAction::Decide {
                                    item_id: item_id.clone(),
                                    choice: "Dismissed".into(),
                                },
                                cx,
                            )
                        },
                    )),
                )
            }
            _ => None,
        };
        let footer = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.0))
            .px(px(14.0))
            .pt(px(2.0))
            .pb(px(8.0))
            .text_size(crate::typography::ui_rems(11.5))
            .text_color(theme.text_faint)
            .child(div().flex_1().child("Esc to answer later"))
            .children(dismiss)
            .child(
                footer_link(
                    "waiting-open",
                    if decision {
                        "Open Supervisor"
                    } else {
                        "Open thread"
                    },
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.act_waiting(
                        WaitingAction::Open {
                            chat_id: open_target.clone(),
                            decision,
                        },
                        cx,
                    )
                })),
            );

        let decide = match &item.kind {
            WaitKind::Decision {
                item_id, options, ..
            } => Some((item_id.clone(), options.clone())),
            _ => None,
        };
        let card = div()
            .id("waiting-card")
            .role(gpui::Role::Dialog)
            .aria_label("Waiting for you")
            .when(motion.live, |el| {
                el.debug_selector(|| "waiting-card".into())
                    .track_focus(&focus)
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                        let key = event.keystroke.key.as_str();
                        if key == "escape" {
                            cx.stop_propagation();
                            this.close_waiting(cx);
                        } else if let Some((item_id, options)) = &decide
                            && !event.keystroke.modifiers.modified()
                            && let Some(choice) = key
                                .parse::<usize>()
                                .ok()
                                .and_then(|n| n.checked_sub(1))
                                .and_then(|n| options.get(n))
                        {
                            cx.stop_propagation();
                            this.act_waiting(
                                WaitingAction::Decide {
                                    item_id: item_id.clone(),
                                    choice: choice.clone(),
                                },
                                cx,
                            );
                        }
                    }))
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_waiting(cx)))
            })
            .occlude()
            .w_full()
            .rounded(px(14.0))
            .border_1()
            .border_color(theme.border)
            .bg(popover::surface_bg(theme))
            .when(!theme.is_frost(), |el| el.shadow_lg())
            .overflow_hidden()
            .flex()
            .flex_col()
            .text_size(crate::typography::ui_rems(13.0))
            .child(header)
            .child(body)
            .child(footer);

        let (offset, width) = match (
            self.waiting.row_bounds.get(),
            self.waiting.pill_bounds.get(),
        ) {
            (Some(row), Some(pill)) => {
                let row_width = f32::from(row.size.width);
                let width = CARD_WIDTH.min(row_width);
                let left = f32::from(pill.origin.x - row.origin.x);
                (left.min(row_width - width).max(0.0), width)
            }
            _ => (0.0, CARD_WIDTH),
        };
        div()
            .w_full()
            .child(
                div()
                    .ml(px(offset))
                    .w(px(width))
                    .child(card_frame(card, motion)),
            )
            .into_any_element()
    }

    fn render_plan_body(
        &self,
        chat_id: &str,
        steps: &[String],
        target: PlanTarget,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = self.state.read(cx);
        let (offer_task, busy, task_blocked) = match &target {
            PlanTarget::Task(_) => (false, false, None),
            PlanTarget::Question(answer) => (
                true,
                state.plan_approval(&answer.request_id).is_some(),
                crate::tasks::chat_plan_blocker(state, chat_id),
            ),
        };
        let task_off = busy || task_blocked.is_some();
        let approve = |as_task: bool| {
            let (chat_id, target) = (chat_id.to_owned(), target.clone());
            cx.listener(move |this: &mut Composer, _: &gpui::ClickEvent, _, cx| {
                cx.stop_propagation();
                this.act_waiting(
                    WaitingAction::ApprovePlan {
                        chat_id: chat_id.clone(),
                        target: target.clone(),
                        as_task,
                    },
                    cx,
                );
            })
        };
        div()
            .px(px(14.0))
            .pt(px(6.0))
            .pb(px(8.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .text_size(crate::typography::ui_rems(10.5))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text_muted)
                    .child(SharedString::from(popover::tracked_upper(&format!(
                        "Plan · {} step{}",
                        steps.len(),
                        if steps.len() == 1 { "" } else { "s" }
                    )))),
            )
            .children(steps.iter().enumerate().map(|(ix, step)| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .size(px(18.0))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .border_1()
                            .border_color(theme.border_strong)
                            .text_size(crate::typography::ui_rems(10.5))
                            .text_color(theme.text_muted)
                            .child(SharedString::from((ix + 1).to_string())),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_color(theme.text)
                            .child(SharedString::from(step.clone())),
                    )
            }))
            .child(
                div()
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_faint)
                    .child("Nothing runs until you approve."),
            )
            .child(
                div()
                    .flex()
                    .gap(px(6.0))
                    .pt(px(4.0))
                    .child(
                        popover::btn_primary(theme, "Approve")
                            .id("waiting-approve")
                            .debug_selector(|| "waiting-approve".into())
                            .role(gpui::Role::Button)
                            .aria_label("Approve")
                            .aria_disabled(busy)
                            .when(busy, |el| el.opacity(0.45))
                            .when(!busy, |el| el.on_click(approve(false))),
                    )
                    .when(offer_task, |row| {
                        row.child(
                            div()
                                .id("waiting-approve-task")
                                .debug_selector(|| "waiting-approve-task".into())
                                .role(gpui::Role::Button)
                                .aria_label("Approve as Task")
                                .aria_disabled(task_off)
                                .px(px(12.0))
                                .py(px(6.0))
                                .rounded(px(8.0))
                                .border_1()
                                .border_color(theme.border_strong)
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(theme.text)
                                .child(icon(icons::CHECKLIST).size(px(13.0)).text_color(theme.text))
                                .child("Approve as Task")
                                .when(task_off, |el| el.opacity(0.45))
                                .when_some(task_blocked, |el, reason| {
                                    el.tooltip(crate::settings::widgets::text_tooltip(reason))
                                })
                                .when(!task_off, |el| {
                                    el.cursor_pointer()
                                        .hover(|s| s.bg(theme.wash(0.05)))
                                        .on_click(approve(true))
                                }),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_decision_body(
        &self,
        item_id: &str,
        title: &str,
        detail: &str,
        options: &[String],
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rows = options.iter().enumerate().map(|(ix, option)| {
            let (item_id, choice) = (item_id.to_owned(), option.clone());
            div()
                .id(("waiting-option", ix))
                .debug_selector(move || format!("waiting-option-{ix}"))
                .role(gpui::Role::Button)
                .aria_label(SharedString::from(option.clone()))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .px(px(8.0))
                .py(px(6.0))
                .rounded(px(8.0))
                .cursor_pointer()
                .text_color(theme.text.opacity(0.85))
                .hover(|s| s.bg(theme.glass_hover()).text_color(theme.text))
                .child(
                    div()
                        .size(px(18.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(5.0))
                        .bg(theme.wash(0.08))
                        .font_family(theme.font_mono.clone())
                        .text_size(crate::typography::ui_rems(11.0))
                        .text_color(theme.text_muted)
                        .child(SharedString::from((ix + 1).to_string())),
                )
                .child(div().min_w_0().child(SharedString::from(option.clone())))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.act_waiting(
                        WaitingAction::Decide {
                            item_id: item_id.clone(),
                            choice: choice.clone(),
                        },
                        cx,
                    )
                }))
        });
        div()
            .px(px(14.0))
            .pt(px(4.0))
            .pb(px(6.0))
            .flex()
            .flex_col()
            .child(
                div()
                    .text_size(crate::typography::ui_rems(14.0))
                    .text_color(theme.text)
                    .child(SharedString::from(title.to_owned())),
            )
            .when(!detail.trim().is_empty(), |el| {
                el.child(
                    div()
                        .mt(px(4.0))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(detail.to_owned())),
                )
            })
            .child(
                div()
                    .mt(px(6.0))
                    .mx(px(-8.0))
                    .flex()
                    .flex_col()
                    .children(rows),
            )
            .into_any_element()
    }
}

fn kind_label(kind: &WaitKind) -> &'static str {
    match kind {
        WaitKind::Ask => "Question",
        WaitKind::Plan { .. } => "Plan",
        WaitKind::Decision { label, .. } => label,
    }
}

fn kind_glyph(kind: &WaitKind, theme: &Theme) -> AnyElement {
    match kind {
        WaitKind::Ask => question_glyph(theme),
        WaitKind::Plan { .. } => icon(icons::CHECKLIST)
            .size(px(13.0))
            .text_color(theme.text_muted)
            .into_any_element(),
        WaitKind::Decision { .. } => icon(icons::LUCIDE_SHIELD)
            .size(px(13.0))
            .text_color(theme.text_muted)
            .into_any_element(),
    }
}

fn waiting_pill_body(
    pill: Stateful<gpui::Div>,
    count: usize,
    active: bool,
    theme: &Theme,
) -> Stateful<gpui::Div> {
    pill.relative()
        .debug_selector(|| "waiting-pill".into())
        .role(gpui::Role::Button)
        .aria_label(SharedString::from(format!("{count} waiting for you")))
        .aria_expanded(active)
        .when(active, |el| {
            el.border_color(theme.border_strong).text_color(theme.text)
        })
        .child(pill_leading(question_glyph(theme)))
        .child(
            div()
                .text_color(theme.accent)
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(SharedString::from(count.to_string())),
        )
        .child("Waiting")
}

fn question_glyph(theme: &Theme) -> AnyElement {
    div()
        .size(px(14.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .border_1()
        .border_color(theme.accent.opacity(0.8))
        .text_size(px(9.5))
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(theme.accent)
        .child("?")
        .into_any_element()
}
