use std::collections::HashMap;

use super::*;
use crate::composer::AskInfo;

const TRAY_WIDTH: f32 = 450.0;
const TRAY_RADIUS: f32 = 12.0;
const ROW_HEIGHT: f32 = 54.0;
const ROW_INDENT: f32 = 44.0;
const SHOWN_AGENTS: usize = 2;
const TICK: std::time::Duration = std::time::Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum NeedKind {
    Asking(Option<AskInfo>),
    PlanReady { task_id: String, steps: Vec<String> },
    Failed,
}

impl NeedKind {
    fn plan(&self) -> bool {
        match self {
            NeedKind::PlanReady { .. } => true,
            NeedKind::Asking(Some(info)) => info.plan,
            _ => false,
        }
    }

    pub(crate) fn label(&self) -> String {
        match self {
            _ if self.plan() => "Plan ready".into(),
            NeedKind::Asking(Some(info)) if info.total > 1 => {
                format!("Asking {}/{}", info.page + 1, info.total)
            }
            NeedKind::Asking(_) => "Asking".into(),
            NeedKind::Failed => "Failed".into(),
            NeedKind::PlanReady { .. } => unreachable!(),
        }
    }
}

pub(crate) struct NeedRow {
    pub chat_id: String,
    pub kind: NeedKind,
}

pub(crate) struct RunRow {
    pub chat_id: String,
    pub started: Option<DateTime<Utc>>,
    pub agents: Vec<(String, Option<DateTime<Utc>>)>,
}

#[derive(Default)]
pub(crate) struct Triage {
    pub needs: Vec<NeedRow>,
    pub running: Vec<RunRow>,
}

impl Triage {
    pub(crate) fn need(&self, chat_id: &str) -> Option<&NeedKind> {
        self.needs
            .iter()
            .find(|row| row.chat_id == chat_id)
            .map(|row| &row.kind)
    }
}

#[derive(Clone)]
struct ShownRow {
    chat_id: String,
    title: String,
    meta: String,
    kind: NeedKind,
}

struct Exits {
    shown: Vec<ShownRow>,
    leaving: Vec<(usize, ShownRow, crate::motion::Tween)>,
}

impl Default for Exits {
    fn default() -> Self {
        Self {
            shown: Vec::new(),
            leaving: Vec::new(),
        }
    }
}

impl Exits {
    fn update(&mut self, rows: Vec<ShownRow>, reduced: bool, now: std::time::Instant) {
        let kept = |row: &ShownRow| rows.iter().any(|shown| shown.chat_id == row.chat_id);
        for (ix, old) in self.shown.iter().enumerate() {
            if !kept(old) {
                let mut tween = crate::motion::Tween::at(1.0, now);
                tween.retarget(0.0, crate::motion::SHELF_CLOSE, reduced, now);
                self.leaving.push((ix, old.clone(), tween));
            }
        }
        self.leaving
            .retain(|(_, row, tween)| !tween.settled_at(now) && !kept(row));
        self.shown = rows;
    }
}

#[derive(Default)]
pub(super) struct TriageTray {
    popup: popover::Popup<()>,
    focus: Option<FocusHandle>,
    expanded: Option<(String, crate::motion::Tween)>,
    rows: Exits,
    card_rows: Exits,
    ticker: Option<Task<()>>,
    asks: Vec<(String, usize, bool)>,
    frame: std::rc::Rc<Triage>,
}

impl TriageTray {
    // The shell does not re-render on composer notifies.
    pub(super) fn observe(composer: &Entity<Composer>, cx: &mut Context<Shell>) -> Subscription {
        cx.observe(composer, |shell, composer, cx| {
            let asks = composer.read(cx).ask_signature();
            let panel_open = shell.triage.popup.get().is_some() && shell.triage.expanded.is_some();
            if asks != shell.triage.asks || panel_open {
                shell.triage.asks = asks;
                cx.notify();
            }
        })
    }
}

impl Shell {
    pub(super) fn refresh_triage_frame(&mut self, cx: &App) {
        self.triage.frame = std::rc::Rc::new(self.triage(cx));
    }

    pub(super) fn triage_frame(&self) -> std::rc::Rc<Triage> {
        self.triage.frame.clone()
    }

    pub(super) fn triage_word(&self, chat_id: &str) -> Option<SharedString> {
        self.triage
            .frame
            .need(chat_id)
            .map(|kind| kind.label().into())
    }

    pub(crate) fn triage(&self, cx: &App) -> Triage {
        let state = self.state.read(cx);
        let composer = self.composer.read(cx);
        let now = Utc::now();
        let mut plans: HashMap<String, (String, Vec<String>)> = HashMap::new();
        for task in state.tasks.iter().filter(|task| !task.deleted) {
            if !task.plan_pending {
                continue;
            }
            let plan = crate::tasks::rollup(state, task, now);
            if plan.phase(task) != zeron_proto::mission::PlanPhase::PlanReady {
                continue;
            }
            if let Some(chat) = crate::tasks::linked_chat(&state.chats, task) {
                let steps = plan
                    .steps
                    .iter()
                    .map(|row| row.step.title.clone())
                    .collect();
                plans.insert(chat.id.clone(), (task.id.clone(), steps));
            }
        }
        let mut triage = Triage::default();
        for (status, chat) in state.overview_chats(now) {
            let kind = match status {
                zeron_proto::ChatIndicator::AwaitingInput if !composer.ask_answered(&chat.id) => {
                    Some(NeedKind::Asking(composer.ask_info(&chat.id)))
                }
                zeron_proto::ChatIndicator::Errored => Some(NeedKind::Failed),
                zeron_proto::ChatIndicator::Working => {
                    let session = state.session_for(&chat.id);
                    triage.running.push(RunRow {
                        chat_id: chat.id.clone(),
                        started: session.and_then(|session| session.started_at),
                        agents: session
                            .map(|session| {
                                session
                                    .background_tasks
                                    .iter()
                                    .filter(|task| {
                                        task.kind == zeron_proto::BackgroundTaskKind::Agent
                                    })
                                    .map(|task| (task.label.clone(), task.started_at))
                                    .collect()
                            })
                            .unwrap_or_default(),
                    });
                    None
                }
                _ => plans
                    .remove(&chat.id)
                    .map(|(task_id, steps)| NeedKind::PlanReady { task_id, steps }),
            };
            if let Some(kind) = kind {
                triage.needs.push(NeedRow {
                    chat_id: chat.id.clone(),
                    kind,
                });
            }
        }
        triage
    }

    pub(super) fn triage_open(&self) -> bool {
        self.triage.popup.is_open()
    }

    pub(crate) fn toggle_triage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.triage.popup.is_open() {
            self.close_triage(cx);
        } else {
            self.open_triage(None, window, cx);
        }
    }

    pub(crate) fn open_triage(
        &mut self,
        chat_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.triage.popup.get().is_none() {
            self.triage.rows = Exits::default();
        }
        self.triage.popup.open(());
        let focus = self
            .triage
            .focus
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        window.focus(&focus, cx);
        match chat_id {
            Some(chat_id) => self.expand_triage_row(chat_id, cx),
            None => self.collapse_triage_row(cx),
        }
        if self.triage.ticker.is_none() {
            self.triage.ticker = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(TICK).await;
                    let open = this.update(cx, |this, cx| {
                        cx.notify();
                        this.triage.popup.get().is_some()
                    });
                    if !open.unwrap_or(false) {
                        break;
                    }
                }
                this.update(cx, |this, _| this.triage.ticker = None).ok();
            }));
        }
        cx.notify();
    }

    pub(crate) fn close_triage(&mut self, cx: &mut Context<Self>) {
        self.collapse_triage_row(cx);
        if self.triage.popup.begin_close() {
            popover::reap_popup(cx, |shell: &mut Self| &mut shell.triage.popup);
        }
        cx.notify();
    }

    fn expand_triage_row(&mut self, chat_id: String, cx: &mut Context<Self>) {
        let now = crate::motion::now();
        let mut tween = crate::motion::Tween::at(0.0, now);
        tween.retarget(
            1.0,
            crate::motion::MENU_IN,
            crate::motion::reduced_motion(cx),
            now,
        );
        self.triage.expanded = Some((chat_id, tween));
        cx.notify();
    }

    fn collapse_triage_row(&mut self, cx: &mut Context<Self>) {
        self.triage.expanded = None;
        self.composer
            .update(cx, |composer, cx| composer.close_ask(cx));
        cx.notify();
    }

    pub(crate) fn open_notification_target(
        &mut self,
        chat_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.triage(cx).need(&chat_id).is_some() {
            self.open_triage(Some(chat_id), window, cx);
        } else {
            self.open_chat(chat_id, cx);
        }
    }

    fn chat_project(&self, chat: &zeron_proto::Chat, cx: &App) -> Option<String> {
        let state = self.state.read(cx);
        let space = state.space_row(chat.space_id.as_deref()?)?;
        Some(state.representative_space(space).display_name().to_owned())
    }

    fn triage_title(&self, chat_id: &str, cx: &App) -> String {
        self.state
            .read(cx)
            .chats
            .iter()
            .find(|chat| chat.id == chat_id)
            .map(|chat| crate::transcript::single_line(&crate::home::chat_title(chat)))
            .unwrap_or_else(|| "New session".into())
    }

    fn need_meta(&self, row: &NeedRow, cx: &App) -> String {
        let state = self.state.read(cx);
        let chat = state.chats.iter().find(|chat| chat.id == row.chat_id);
        let mut parts: Vec<String> = chat
            .and_then(|chat| self.chat_project(chat, cx))
            .into_iter()
            .collect();
        parts.push(row.kind.label());
        match &row.kind {
            NeedKind::PlanReady { steps, .. } => parts.push(format!(
                "{} step{}, waiting for your approval",
                steps.len(),
                if steps.len() == 1 { "" } else { "s" }
            )),
            NeedKind::Asking(Some(info)) if !info.question.is_empty() => {
                parts.push(crate::transcript::single_line(&info.question))
            }
            NeedKind::Failed => {
                if let Some(at) = state.session_for(&row.chat_id).map(|s| s.updated_at) {
                    parts.push(crate::settings::devices::format_last_seen(
                        Some(at),
                        Utc::now(),
                    ));
                }
            }
            NeedKind::Asking(_) => {}
        }
        parts.join(" · ")
    }

    fn running_meta(&self, row: &RunRow, cx: &App) -> String {
        let state = self.state.read(cx);
        let chat = state.chats.iter().find(|chat| chat.id == row.chat_id);
        let mut parts: Vec<String> = chat
            .and_then(|chat| self.chat_project(chat, cx))
            .into_iter()
            .collect();
        parts.push(match row.started {
            Some(at) => format!("Working {}", elapsed(at)),
            None => "Working".into(),
        });
        if let Some(label) = state
            .session_for(&row.chat_id)
            .and_then(|session| zeron_proto::background::ring_label(&session.background_tasks))
        {
            parts.push(label);
        }
        parts.join(" · ")
    }

    pub(super) fn render_triage_pill(
        &mut self,
        triage: &Triage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let theme = Theme::of(cx).clone();
        let (needs, running) = (triage.needs.len(), triage.running.len());
        let mounted = self.triage.popup.get().is_some();
        if needs == 0 && running == 0 && !mounted {
            return None;
        }
        let open = self.triage.popup.is_open();
        let dot = |color: gpui::Hsla| div().size(px(7.0)).flex_none().rounded_full().bg(color);
        let label = match (needs, running) {
            (0, r) => format!("{r} running"),
            (n, 0) => format!("{n} need you"),
            (n, r) => format!("{n} need you · {r} running"),
        };
        let pill = div()
            .id("triage-pill")
            .debug_selector(|| "triage-pill".into())
            .role(gpui::Role::Button)
            .aria_label(SharedString::from(label))
            .aria_expanded(open)
            .h(px(28.0))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(7.0))
            .pl(px(9.0))
            .pr(px(6.0))
            .rounded(px(14.0))
            .border_1()
            .border_color(theme.border)
            .bg(if open {
                theme.wash(0.08)
            } else {
                crate::motion::hover_blend("triage-pill", theme.surface_card, theme.wash(0.06))
            })
            .on_hover(crate::motion::hover_listener("triage-pill"))
            .cursor_pointer()
            .text_size(crate::typography::ui_rems(12.5))
            .font_weight(gpui::FontWeight::MEDIUM)
            .whitespace_nowrap()
            .when(needs > 0, |el| {
                el.child(dot(theme.accent)).child(
                    div()
                        .text_color(theme.accent)
                        .child(SharedString::from(format!("{needs} need you"))),
                )
            })
            .when(needs > 0 && running > 0, |el| {
                el.child(div().text_color(theme.text_muted).child("·"))
            })
            .when(running > 0, |el| {
                el.child(dot(chat_status_tone(
                    zeron_proto::ChatIndicator::Working,
                    &theme,
                )
                .2))
                    .child(
                        div()
                            .text_color(theme.text_muted)
                            .font_weight(gpui::FontWeight::NORMAL)
                            .child(SharedString::from(format!("{running} running"))),
                    )
            })
            .child(
                div()
                    .ml(px(2.0))
                    .px(px(5.0))
                    .h(px(18.0))
                    .flex()
                    .items_center()
                    .rounded(px(5.0))
                    .bg(theme.wash(0.06))
                    .child(popover::kbd_hint(
                        &theme,
                        &crate::settings::badge_combo(
                            self.settings.keymap.get(ShortcutId::ToggleTray),
                        ),
                    )),
            )
            .tooltip(crate::settings::widgets::text_tooltip(
                "What needs you and what is running",
            ))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.triage.popup.note_trigger_press();
                    cx.stop_propagation();
                }),
            )
            .on_click(cx.listener(|this, _, window, cx| {
                if this.triage.popup.take_press_was_open() {
                    this.close_triage(cx);
                } else {
                    this.open_triage(None, window, cx);
                }
            }));
        let tray = mounted.then(|| self.render_triage_tray(triage, &theme, window, cx));
        Some(
            div()
                .relative()
                .flex_none()
                .child(pill)
                .children(tray)
                .into_any_element(),
        )
    }

    fn supervisor_summary(&self, triage: &Triage, cx: &App) -> String {
        let mut parts: Vec<String> = Vec::new();
        let titles = |filter: &dyn Fn(&NeedKind) -> bool| -> Vec<String> {
            triage
                .needs
                .iter()
                .filter(|row| filter(&row.kind))
                .map(|row| self.triage_title(&row.chat_id, cx))
                .collect()
        };
        for title in titles(&|kind| matches!(kind, NeedKind::Asking(_)) && !kind.plan()) {
            parts.push(format!("{title} asks you."));
        }
        let plans = titles(&NeedKind::plan);
        if !plans.is_empty() {
            parts.push(format!(
                "{} {} to approve.",
                plans.join(", "),
                if plans.len() == 1 {
                    "has a plan"
                } else {
                    "have plans"
                }
            ));
        }
        let failed = titles(&|kind| matches!(kind, NeedKind::Failed));
        if !failed.is_empty() {
            parts.push(format!("{} failed.", failed.join(", ")));
        }
        if parts.is_empty() {
            parts.push("Nothing needs you.".into());
        }
        let running = triage.running.len();
        parts.push(match running {
            1 => "1 thread is running.".into(),
            n => format!("{n} threads are running."),
        });
        parts.join(" ")
    }

    fn render_triage_tray(
        &mut self,
        triage: &Triage,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let popup = theme.for_popup();
        let now = crate::motion::now();
        let reduced = crate::motion::reduced_motion(cx);
        let mut animating = false;

        let shown: Vec<ShownRow> = triage
            .needs
            .iter()
            .map(|row| ShownRow {
                chat_id: row.chat_id.clone(),
                title: self.triage_title(&row.chat_id, cx),
                meta: self.need_meta(row, cx),
                kind: row.kind.clone(),
            })
            .collect();
        self.triage.rows.update(shown.clone(), reduced, now);
        if self
            .triage
            .expanded
            .as_ref()
            .is_some_and(|(id, _)| !shown.iter().any(|row| &row.chat_id == id))
        {
            self.triage.expanded = None;
        }

        let mut need_rows: Vec<AnyElement> = shown
            .iter()
            .map(|row| self.render_need_row(row, &popup, window, cx))
            .collect();
        for (ix, row, tween) in self.triage.rows.leaving.clone() {
            animating = true;
            let t = tween.value_at(now);
            let ghost = div()
                .h(px(ROW_HEIGHT * t))
                .opacity(t)
                .overflow_hidden()
                .child(need_row_shell(&row, &popup).id(SharedString::from(format!(
                    "triage-leaving-{}",
                    row.chat_id
                ))))
                .into_any_element();
            need_rows.insert(ix.min(need_rows.len()), ghost);
        }
        if let Some((_, tween)) = &self.triage.expanded
            && !tween.settled_at(now)
        {
            animating = true;
        }
        if need_rows.is_empty() {
            need_rows.push(empty_line(&popup, "Nothing needs you."));
        }

        let running_rows: Vec<AnyElement> = triage
            .running
            .iter()
            .map(|row| self.render_running_row(row, &popup, cx))
            .collect();

        let supervisor = div()
            .id("triage-supervisor")
            .debug_selector(|| "triage-supervisor".into())
            .role(gpui::Role::Button)
            .aria_label("Open the Supervisor chat")
            .flex()
            .items_center()
            .gap(px(10.0))
            .px(px(10.0))
            .py(px(9.0))
            .rounded(px(9.0))
            .cursor_pointer()
            .hover(|s| s.bg(popup.wash(0.06)))
            .child(status_box(
                icons::LUCIDE_SHIELD,
                popup.wash(0.06),
                popup.text_muted,
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Supervisor"),
                    )
                    .child(
                        div()
                            .mt(px(2.0))
                            .truncate()
                            .text_size(crate::typography::ui_rems(12.0))
                            .text_color(popup.text_muted)
                            .child(SharedString::from(self.supervisor_summary(triage, cx))),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(popup.text_muted)
                    .child("Ask ›"),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.close_triage(cx);
                this.open_supervisor(cx);
            }));

        let footer_button = |id: &'static str, glyph: &'static str, label: &'static str| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .role(gpui::Role::Button)
                .aria_label(label)
                .flex()
                .items_center()
                .gap(px(6.0))
                .px(px(8.0))
                .h(px(28.0))
                .rounded(px(7.0))
                .cursor_pointer()
                .text_color(popup.text_muted)
                .hover(|s| s.bg(popup.wash(0.06)).text_color(popup.text))
                .child(icon(glyph).size(px(13.0)).text_color(popup.text_muted))
                .child(label)
        };
        let footer = div()
            .flex()
            .items_center()
            .mt(px(6.0))
            .pt(px(8.0))
            .px(px(6.0))
            .pb(px(2.0))
            .border_t_1()
            .border_color(popup.border)
            .child(
                footer_button("triage-history", icons::BELL, "History").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.close_triage(cx);
                        this.open_inbox(cx);
                    },
                )),
            )
            .child(div().flex_1())
            .child(
                footer_button("triage-tasks", icons::TASKS, "Tasks").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.close_triage(cx);
                        this.open_tasks(cx);
                    },
                )),
            );

        let focus = self
            .triage
            .focus
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        let card = popover::popover_card(&popup)
            .id("triage-tray")
            .debug_selector(|| "triage-tray".into())
            .role(gpui::Role::Dialog)
            .aria_label("Needs you and running")
            .track_focus(&focus)
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    this.close_triage(cx);
                }
            }))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_triage(cx)))
            .w(px(TRAY_WIDTH))
            .rounded(px(TRAY_RADIUS))
            .p(px(8.0))
            .flex()
            .flex_col()
            .text_size(crate::typography::ui_rems(13.0))
            .child(
                div()
                    .id("triage-tray-scroll")
                    .max_h(px(
                        (self.viewport_height - Theme::TITLEBAR_HEIGHT - 120.0).max(240.0)
                    ))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .child(supervisor)
                    .child(section_heading(
                        &popup,
                        "Needs you",
                        triage.needs.len(),
                        popup.accent,
                    ))
                    .children(need_rows)
                    .child(section_heading(
                        &popup,
                        "Running",
                        triage.running.len(),
                        chat_status_tone(zeron_proto::ChatIndicator::Working, &popup).2,
                    ))
                    .when(running_rows.is_empty(), |el| {
                        el.child(empty_line(&popup, "Nothing running."))
                    })
                    .children(running_rows),
            )
            .child(footer);
        if animating {
            let view = cx.entity_id();
            return popover::anchored_menu_below_end(
                "triage-tray",
                div()
                    .child(card)
                    .child(gpui::canvas(
                        |_, _, _| {},
                        move |_, _, window, _| window.on_next_frame(move |_, cx| cx.notify(view)),
                    ))
                    .into_any_element(),
                self.triage.popup.closing_since(),
            );
        }
        popover::anchored_menu_below_end(
            "triage-tray",
            card.into_any_element(),
            self.triage.popup.closing_since(),
        )
    }

    fn render_need_row(
        &mut self,
        row: &ShownRow,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let expanded = self
            .triage
            .expanded
            .as_ref()
            .filter(|(id, _)| id == &row.chat_id)
            .map(|(_, tween)| tween.value_at(crate::motion::now()));
        let chat_id = row.chat_id.clone();
        let action: AnyElement = match &row.kind {
            NeedKind::Failed | NeedKind::Asking(None) => {
                let target = chat_id.clone();
                popover::btn_primary(theme, "Open")
                    .id(SharedString::from(format!("triage-open-{chat_id}")))
                    .debug_selector({
                        let id = chat_id.clone();
                        move || format!("triage-open-{id}")
                    })
                    .role(gpui::Role::Button)
                    .aria_label(format!("Open {}", row.title))
                    .flex_none()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.close_triage(cx);
                        this.open_chat(target.clone(), cx);
                    }))
                    .into_any_element()
            }
            kind => {
                let target = chat_id.clone();
                let label = if kind.plan() { "Review" } else { "Answer" };
                let open = expanded.is_some();
                div()
                    .id(SharedString::from(format!("triage-expand-{chat_id}")))
                    .debug_selector({
                        let id = chat_id.clone();
                        move || format!("triage-expand-{id}")
                    })
                    .role(gpui::Role::Button)
                    .aria_label(format!("{label} {}", row.title))
                    .aria_expanded(open)
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .h(px(28.0))
                    .px(px(10.0))
                    .rounded(px(8.0))
                    .bg(theme.accent.opacity(0.12))
                    .hover(|s| s.bg(theme.accent.opacity(0.18)))
                    .cursor_pointer()
                    .text_color(theme.accent)
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(label)
                    .child(
                        icon(icons::LUCIDE_CHEVRON_DOWN)
                            .size(px(12.0))
                            .text_color(theme.accent)
                            .when(open, |el| {
                                el.with_transformation(gpui::Transformation::rotate(
                                    gpui::percentage(0.5),
                                ))
                            }),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if this
                            .triage
                            .expanded
                            .as_ref()
                            .is_some_and(|(id, _)| id == &target)
                        {
                            this.collapse_triage_row(cx);
                        } else {
                            this.collapse_triage_row(cx);
                            this.expand_triage_row(target.clone(), cx);
                        }
                    }))
                    .into_any_element()
            }
        };
        let open_target = chat_id.clone();
        let head = need_row_shell(row, theme)
            .id(SharedString::from(format!("triage-need-{chat_id}")))
            .debug_selector({
                let id = chat_id.clone();
                move || format!("triage-need-{id}")
            })
            .role(gpui::Role::Button)
            .aria_label(row.title.clone())
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.close_triage(cx);
                this.open_chat(open_target.clone(), cx);
            }))
            .child(action);
        let body = expanded.and_then(|shown| {
            let panel = match &row.kind {
                NeedKind::PlanReady { task_id, steps } => {
                    Some(self.render_plan_panel(&chat_id, task_id, steps, theme, cx))
                }
                NeedKind::Asking(_) => self.composer.update(cx, |composer, cx| {
                    composer.render_ask_panel(&chat_id, window, cx)
                }),
                NeedKind::Failed => None,
            }?;
            Some(
                div()
                    .pl(px(ROW_INDENT))
                    .pr(px(10.0))
                    .pb(px(10.0))
                    .pt(px(2.0))
                    .opacity(shown)
                    .child(panel),
            )
        });
        div()
            .rounded(px(9.0))
            .when(expanded.is_some(), |el| el.bg(theme.wash(0.05)))
            .child(head)
            .children(body)
            .into_any_element()
    }

    fn render_plan_panel(
        &mut self,
        chat_id: &str,
        task_id: &str,
        steps: &[String],
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let task = task_id.to_owned();
        let id = chat_id.to_owned();
        div()
            .debug_selector({
                let id = id.clone();
                move || format!("triage-plan-{id}")
            })
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
                    .justify_end()
                    .pt(px(8.0))
                    .border_t_1()
                    .border_color(theme.border)
                    .child(
                        popover::btn_primary(theme, "Approve")
                            .id(SharedString::from(format!("triage-approve-{id}")))
                            .debug_selector(move || format!("triage-approve-{id}"))
                            .role(gpui::Role::Button)
                            .aria_label("Approve")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.mutate(
                                    serde_json::json!({ "op": "approvePlan", "taskId": task }),
                                    cx,
                                );
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_running_row(
        &self,
        row: &RunRow,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let chat_id = row.chat_id.clone();
        let working = chat_status_tone(zeron_proto::ChatIndicator::Working, theme).2;
        let more = row.agents.len().saturating_sub(SHOWN_AGENTS);
        let agents = row
            .agents
            .iter()
            .take(SHOWN_AGENTS)
            .map(|(label, started)| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.0))
                    .py(px(3.0))
                    .min_w_0()
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .child(icon(icons::BOT).size(px(12.0)).text_color(theme.text_muted))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(theme.text)
                            .child(SharedString::from(crate::transcript::single_line(label))),
                    )
                    .children(
                        started.map(|at| div().flex_none().child(SharedString::from(elapsed(at)))),
                    )
            });
        let target = chat_id.clone();
        let title = self.triage_title(&chat_id, cx);
        div()
            .id(SharedString::from(format!("triage-run-{chat_id}")))
            .debug_selector(move || format!("triage-run-{chat_id}"))
            .role(gpui::Role::Button)
            .aria_label(title.clone())
            .rounded(px(9.0))
            .cursor_pointer()
            .hover(|s| s.bg(theme.wash(0.05)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.close_triage(cx);
                this.open_chat(target.clone(), cx);
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .px(px(10.0))
                    .py(px(9.0))
                    .child(status_box(
                        icons::LUCIDE_PLAY,
                        working.opacity(0.13),
                        working,
                    ))
                    .child(row_text(theme, title, self.running_meta(row, cx), None)),
            )
            .when(!row.agents.is_empty(), |el| {
                el.child(
                    div()
                        .pl(px(ROW_INDENT))
                        .pr(px(10.0))
                        .pb(px(8.0))
                        .flex()
                        .flex_col()
                        .children(agents)
                        .when(more > 0, |el| {
                            el.child(
                                div()
                                    .py(px(3.0))
                                    .text_size(crate::typography::ui_rems(12.0))
                                    .text_color(theme.text_faint)
                                    .child(SharedString::from(format!(
                                        "{more} more agent{}",
                                        if more == 1 { "" } else { "s" }
                                    ))),
                            )
                        }),
                )
            })
            .into_any_element()
    }
}

impl Shell {
    pub(super) fn render_new_work_cards(
        &mut self,
        width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let triage = self.triage_frame();
        let now = crate::motion::now();
        let reduced = crate::motion::reduced_motion(cx);
        let rows: Vec<ShownRow> = triage
            .needs
            .iter()
            .map(|row| ShownRow {
                chat_id: row.chat_id.clone(),
                title: self.triage_title(&row.chat_id, cx),
                meta: card_need_meta(
                    &row.kind,
                    self.state
                        .read(cx)
                        .session_for(&row.chat_id)
                        .map(|session| session.updated_at),
                ),
                kind: row.kind.clone(),
            })
            .collect();
        self.triage.card_rows.update(rows.clone(), reduced, now);
        let tray_mounted = self.triage.popup.get().is_some();
        if !tray_mounted
            && self
                .triage
                .expanded
                .as_ref()
                .is_some_and(|(id, _)| !rows.iter().any(|row| &row.chat_id == id))
        {
            self.triage.expanded = None;
        }
        let mut animating = self
            .triage
            .expanded
            .as_ref()
            .is_some_and(|(_, tween)| !tween.settled_at(now));

        let mut need_rows: Vec<AnyElement> = rows
            .iter()
            .take(CARD_ROWS)
            .map(|row| self.render_card_need_row(row, tray_mounted, &theme, window, cx))
            .collect();
        for (ix, row, tween) in self.triage.card_rows.leaving.clone() {
            if ix >= CARD_ROWS {
                continue;
            }
            animating = true;
            let t = tween.value_at(now);
            let ghost = div()
                .h(px(CARD_ROW_HEIGHT * t))
                .opacity(t)
                .overflow_hidden()
                .child(card_row_shell(
                    &theme,
                    SharedString::from(format!("card-leaving-{}", row.chat_id)),
                    need_tint(&row.kind, &theme),
                    row.title.clone(),
                    row.meta.clone(),
                    need_tint(&row.kind, &theme),
                ))
                .into_any_element();
            need_rows.insert(ix.min(need_rows.len()), ghost);
        }

        let working = chat_status_tone(zeron_proto::ChatIndicator::Working, &theme).2;
        let run_rows: Vec<AnyElement> = triage
            .running
            .iter()
            .take(CARD_ROWS)
            .map(|row| {
                let chat_id = row.chat_id.clone();
                let state = self.state.read(cx);
                let project = state
                    .chats
                    .iter()
                    .find(|chat| chat.id == chat_id)
                    .and_then(|chat| self.chat_project(chat, cx));
                let doing = match row.agents.len() {
                    0 => "Working".to_string(),
                    1 => "1 agent".to_string(),
                    n => format!("{n} agents"),
                };
                let meta = project.map_or(doing.clone(), |project| format!("{project} · {doing}"));
                let title = self.triage_title(&chat_id, cx);
                card_row_shell(
                    &theme,
                    SharedString::from(format!("card-run-{chat_id}")),
                    working,
                    title.clone(),
                    meta,
                    theme.text_muted,
                )
                .role(gpui::Role::Button)
                .aria_label(title)
                .cursor_pointer()
                .hover(|s| s.bg(theme.wash(0.05)))
                .on_click(cx.listener(move |this, _, _, cx| this.open_chat(chat_id.clone(), cx)))
                .into_any_element()
            })
            .collect();

        let recent = self.recent_card_rows(cx);
        let done = chat_status_tone(zeron_proto::ChatIndicator::Completed, &theme).2;
        let recent_rows: Vec<AnyElement> = recent
            .into_iter()
            .map(|(chat_id, title, meta, completed)| {
                let target = chat_id.clone();
                card_row_shell(
                    &theme,
                    SharedString::from(format!("card-recent-{chat_id}")),
                    if completed { done } else { theme.text_faint },
                    title.clone(),
                    meta,
                    theme.text_muted,
                )
                .role(gpui::Role::Button)
                .aria_label(title)
                .cursor_pointer()
                .hover(|s| s.bg(theme.wash(0.05)))
                .on_click(cx.listener(move |this, _, _, cx| this.open_chat(target.clone(), cx)))
                .into_any_element()
            })
            .collect();

        let more = |id: &'static str, hidden: usize, label: String, cx: &mut Context<Self>| {
            (hidden > 0).then(|| {
                div()
                    .id(id)
                    .debug_selector(move || id.into())
                    .role(gpui::Role::Button)
                    .aria_label(SharedString::from(label.clone()))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .px(px(6.0))
                    .py(px(5.0))
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .text_size(crate::typography::ui_rems(11.5))
                    .text_color(theme.text_muted)
                    .hover(|s| s.bg(theme.wash(0.05)).text_color(theme.text))
                    .child(SharedString::from(label))
                    .child(popover::kbd_hint(
                        &theme,
                        &crate::settings::badge_combo(
                            self.settings.keymap.get(ShortcutId::ToggleTray),
                        ),
                    ))
                    .on_click(cx.listener(|this, _, window, cx| this.open_triage(None, window, cx)))
                    .into_any_element()
            })
        };
        let needs_more = more(
            "card-needs-more",
            triage.needs.len().saturating_sub(CARD_ROWS),
            format!(
                "{} more in the tray",
                triage.needs.len().saturating_sub(CARD_ROWS)
            ),
            cx,
        );
        let running_more = more(
            "card-running-more",
            triage.running.len().saturating_sub(CARD_ROWS),
            format!("{} more", triage.running.len().saturating_sub(CARD_ROWS)),
            cx,
        );

        let gap = 10.0;
        let unit = ((width - 2.0 * gap) / 3.4).max(0.0);
        if animating {
            window.request_animation_frame();
        }
        div()
            .id("new-work-cards")
            .debug_selector(|| "new-work-cards".into())
            .w(px(width))
            .mx_auto()
            .flex()
            .items_start()
            .gap(px(gap))
            .text_size(crate::typography::ui_rems(13.0))
            .child(work_card(
                &theme,
                "card-needs",
                icons::BELL,
                "Needs you",
                Some(triage.needs.len()),
                unit * 1.4,
                need_rows,
                needs_more,
            ))
            .child(work_card(
                &theme,
                "card-running",
                icons::SPEEDOMETER,
                "Running",
                Some(triage.running.len()),
                unit,
                run_rows,
                running_more,
            ))
            .child(work_card(
                &theme,
                "card-recent",
                icons::CLOCK_CIRCLE,
                "Recent",
                None,
                unit,
                recent_rows,
                None,
            ))
            .into_any_element()
    }

    fn render_card_need_row(
        &mut self,
        row: &ShownRow,
        tray_mounted: bool,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let chat_id = row.chat_id.clone();
        let expanded = self
            .triage
            .expanded
            .as_ref()
            .filter(|(id, _)| id == &row.chat_id)
            .map(|(_, tween)| tween.value_at(crate::motion::now()));
        let action: AnyElement = match &row.kind {
            NeedKind::Failed | NeedKind::Asking(None) => {
                let target = chat_id.clone();
                card_button(theme, false)
                    .id(SharedString::from(format!("card-open-{chat_id}")))
                    .role(gpui::Role::Button)
                    .aria_label(format!("Open {}", row.title))
                    .child("Open")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.open_chat(target.clone(), cx);
                    }))
                    .into_any_element()
            }
            kind => {
                let target = chat_id.clone();
                let label = if kind.plan() { "Review" } else { "Answer" };
                card_button(theme, true)
                    .id(SharedString::from(format!("card-expand-{chat_id}")))
                    .role(gpui::Role::Button)
                    .aria_label(format!("{label} {}", row.title))
                    .aria_expanded(expanded.is_some())
                    .child(label)
                    .child(
                        icon(icons::LUCIDE_CHEVRON_DOWN)
                            .size(px(12.0))
                            .text_color(theme.accent)
                            .when(expanded.is_some(), |el| {
                                el.with_transformation(gpui::Transformation::rotate(
                                    gpui::percentage(0.5),
                                ))
                            }),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        let open = this
                            .triage
                            .expanded
                            .as_ref()
                            .is_some_and(|(id, _)| id == &target);
                        this.collapse_triage_row(cx);
                        if !open {
                            this.expand_triage_row(target.clone(), cx);
                        }
                    }))
                    .into_any_element()
            }
        };
        let tint = need_tint(&row.kind, theme);
        let open_target = chat_id.clone();
        let head = card_row_shell(
            theme,
            SharedString::from(format!("card-need-{chat_id}")),
            tint,
            row.title.clone(),
            row.meta.clone(),
            tint,
        )
        .role(gpui::Role::Button)
        .aria_label(row.title.clone())
        .cursor_pointer()
        .hover(|s| s.bg(theme.wash(0.05)))
        .on_click(cx.listener(move |this, _, _, cx| this.open_chat(open_target.clone(), cx)))
        .child(action);
        let body = expanded.filter(|_| !tray_mounted).and_then(|shown| {
            let panel = match &row.kind {
                NeedKind::PlanReady { task_id, steps } => {
                    Some(self.render_plan_panel(&chat_id, task_id, steps, theme, cx))
                }
                NeedKind::Asking(_) => self.composer.update(cx, |composer, cx| {
                    composer.render_ask_panel(&chat_id, window, cx)
                }),
                NeedKind::Failed => None,
            }?;
            Some(
                div()
                    .pl(px(CARD_ROW_INDENT))
                    .pr(px(6.0))
                    .pb(px(6.0))
                    .opacity(shown)
                    .child(panel),
            )
        });
        div()
            .rounded(px(9.0))
            .when(expanded.is_some() && !tray_mounted, |el| {
                el.bg(theme.wash(0.04))
            })
            .child(head)
            .children(body)
            .into_any_element()
    }

    fn recent_card_rows(&self, cx: &App) -> Vec<(String, String, String, bool)> {
        let state = self.state.read(cx);
        let now = Utc::now();
        let mut done: Vec<(zeron_proto::ChatIndicator, &zeron_proto::Chat)> = state
            .overview_chats(now)
            .into_iter()
            .filter(|(status, _)| {
                matches!(
                    status,
                    zeron_proto::ChatIndicator::Completed | zeron_proto::ChatIndicator::Idle
                )
            })
            .collect();
        done.sort_by_key(|(_, chat)| {
            std::cmp::Reverse(chat.last_message_at.unwrap_or(chat.created_at))
        });
        done.into_iter()
            .take(CARD_ROWS)
            .map(|(status, chat)| {
                let completed = status == zeron_proto::ChatIndicator::Completed;
                let ago = crate::settings::devices::format_last_seen(
                    Some(chat.last_message_at.unwrap_or(chat.created_at)),
                    now,
                );
                let when = if completed {
                    format!("Done {ago}")
                } else {
                    ago
                };
                let meta = match self.chat_project(chat, cx) {
                    Some(project) => format!("{project} · {when}"),
                    None => when,
                };
                (
                    chat.id.clone(),
                    crate::transcript::single_line(&crate::home::chat_title(chat)),
                    meta,
                    completed,
                )
            })
            .collect()
    }
}

const CARD_ROWS: usize = 3;
const CARD_ROW_HEIGHT: f32 = 46.0;
const CARD_ROW_INDENT: f32 = 21.0;

fn need_tint(kind: &NeedKind, theme: &Theme) -> gpui::Hsla {
    match kind {
        NeedKind::Failed => theme.danger,
        _ => theme.accent,
    }
}

fn card_need_meta(kind: &NeedKind, failed_at: Option<DateTime<Utc>>) -> String {
    match kind {
        NeedKind::PlanReady { steps, .. } => format!(
            "Plan ready · {} step{}",
            steps.len(),
            if steps.len() == 1 { "" } else { "s" }
        ),
        NeedKind::Asking(Some(info)) if info.plan => "Plan ready".into(),
        NeedKind::Asking(Some(info)) if !info.question.is_empty() => {
            format!("Asks: {}", crate::transcript::single_line(&info.question))
        }
        NeedKind::Asking(_) => "Asking".into(),
        NeedKind::Failed => match failed_at {
            Some(at) => format!(
                "Failed {}",
                crate::settings::devices::format_last_seen(Some(at), Utc::now())
            ),
            None => "Failed".into(),
        },
    }
}

fn card_row_shell(
    theme: &Theme,
    id: SharedString,
    dot: gpui::Hsla,
    title: String,
    meta: String,
    meta_color: gpui::Hsla,
) -> gpui::Stateful<gpui::Div> {
    let selector = id.clone();
    div()
        .id(id)
        .debug_selector(move || selector.to_string())
        .h(px(CARD_ROW_HEIGHT))
        .px(px(6.0))
        .flex()
        .items_center()
        .gap(px(8.0))
        .rounded(px(9.0))
        .child(div().size(px(7.0)).flex_none().rounded_full().bg(dot))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(theme.text)
                        .child(SharedString::from(title)),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(crate::typography::ui_rems(11.5))
                        .text_color(meta_color)
                        .child(SharedString::from(meta)),
                ),
        )
}

fn card_button(theme: &Theme, accent: bool) -> gpui::Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(4.0))
        .h(px(26.0))
        .px(px(10.0))
        .rounded(px(7.0))
        .cursor_pointer()
        .text_size(crate::typography::ui_rems(12.0))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .when(accent, |el| {
            el.bg(theme.accent.opacity(0.12))
                .hover(|s| s.bg(theme.accent.opacity(0.18)))
                .text_color(theme.accent)
        })
        .when(!accent, |el| {
            el.bg(theme.wash(0.06))
                .hover(|s| s.bg(theme.wash(0.1)))
                .text_color(theme.text)
        })
}

#[allow(clippy::too_many_arguments)]
fn work_card(
    theme: &Theme,
    id: &'static str,
    glyph: &'static str,
    title: &'static str,
    count: Option<usize>,
    width: f32,
    rows: Vec<AnyElement>,
    more: Option<AnyElement>,
) -> AnyElement {
    let empty = rows.is_empty();
    div()
        .id(id)
        .debug_selector(move || id.into())
        .role(gpui::Role::Group)
        .aria_label(title)
        .w(px(width))
        .flex_none()
        .min_w_0()
        .pt(px(10.0))
        .px(px(8.0))
        .pb(px(8.0))
        .rounded(px(10.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface_card)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .px(px(6.0))
                .pb(px(6.0))
                .text_size(crate::typography::ui_rems(12.0))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.text_muted)
                .child(icon(glyph).size(px(15.0)).text_color(theme.text_muted))
                .child(title)
                .children(count.map(|count| {
                    div()
                        .ml_auto()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(SharedString::from(count.to_string()))
                })),
        )
        .children(rows)
        .when(empty, |el| {
            el.child(
                div()
                    .px(px(6.0))
                    .py(px(6.0))
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_faint)
                    .child("Nothing"),
            )
        })
        .children(more)
        .into_any_element()
}

fn elapsed(at: DateTime<Utc>) -> String {
    let secs = Utc::now().signed_duration_since(at).num_seconds().max(0);
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

fn status_box(glyph: &'static str, bg: gpui::Hsla, fg: gpui::Hsla) -> gpui::Div {
    div()
        .size(px(24.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(7.0))
        .bg(bg)
        .child(icon(glyph).size(px(14.0)).text_color(fg))
}

fn row_text(
    theme: &Theme,
    title: String,
    meta: String,
    word: Option<(String, gpui::Hsla)>,
) -> gpui::Div {
    let highlights: Vec<_> = word
        .and_then(|(word, color)| {
            let start = meta.find(&word)?;
            Some((start..start + word.len(), color.into()))
        })
        .into_iter()
        .collect();
    div()
        .flex_1()
        .min_w_0()
        .child(
            div()
                .truncate()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(SharedString::from(title)),
        )
        .child(
            div()
                .mt(px(2.0))
                .truncate()
                .text_size(crate::typography::ui_rems(12.0))
                .text_color(theme.text_muted)
                .child(gpui::StyledText::new(meta).with_highlights(highlights)),
        )
}

fn need_row_shell(row: &ShownRow, theme: &Theme) -> gpui::Div {
    let (glyph, tint) = match &row.kind {
        NeedKind::Failed => (icons::LUCIDE_CIRCLE_ALERT, theme.danger),
        kind if kind.plan() => (icons::CHECKLIST, theme.accent),
        _ => (icons::LUCIDE_MESSAGE_CIRCLE_QUESTION, theme.accent),
    };
    div()
        .h(px(ROW_HEIGHT))
        .flex()
        .items_center()
        .gap(px(10.0))
        .px(px(10.0))
        .rounded(px(9.0))
        .child(status_box(glyph, tint.opacity(0.13), tint))
        .child(row_text(
            theme,
            row.title.clone(),
            row.meta.clone(),
            Some((row.kind.label(), tint)),
        ))
}

fn section_heading(theme: &Theme, label: &str, count: usize, tint: gpui::Hsla) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .px(px(10.0))
        .pt(px(10.0))
        .pb(px(4.0))
        .text_size(crate::typography::ui_rems(10.5))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(theme.text_muted)
        .child(SharedString::from(popover::tracked_upper(label)))
        .child(
            div()
                .h(px(18.0))
                .min_w(px(18.0))
                .px(px(5.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(tint.opacity(0.13))
                .text_color(tint)
                .child(SharedString::from(count.to_string())),
        )
        .into_any_element()
}

fn empty_line(theme: &Theme, text: &'static str) -> AnyElement {
    div()
        .px(px(10.0))
        .py(px(6.0))
        .text_size(crate::typography::ui_rems(12.0))
        .text_color(theme.text_muted)
        .child(text)
        .into_any_element()
}
