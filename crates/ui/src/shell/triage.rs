use std::collections::HashMap;

use super::*;
use crate::composer::{AskInfo, WaitItem, WaitKind, WaitingAction};
use crate::provider_instances::InstanceIcon;

const TRAY_WIDTH: f32 = 450.0;
const TRAY_RADIUS: f32 = 12.0;
const ROW_HEIGHT: f32 = 54.0;
const ROW_INDENT: f32 = 44.0;
const SHOWN_AGENTS: usize = 2;
const TICK: std::time::Duration = std::time::Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum NeedKind {
    Asking(Option<AskInfo>),
    PlanReady {
        task_id: String,
        steps: Vec<String>,
    },
    Failed,
    Decision {
        item_id: String,
        title: String,
        why: zeron_proto::supervisor::ItemWhy,
        detail: String,
        options: Vec<String>,
    },
    Task {
        task_id: String,
        title: String,
        reason: zeron_proto::NeedsYou,
    },
}

impl NeedKind {
    fn of_chat(&self) -> bool {
        !matches!(self, NeedKind::Decision { .. } | NeedKind::Task { .. })
    }

    pub(crate) fn waits(&self) -> bool {
        matches!(
            self,
            NeedKind::Asking(Some(_)) | NeedKind::PlanReady { .. } | NeedKind::Decision { .. }
        )
    }

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
            NeedKind::Decision { why, .. } => why.label().into(),
            NeedKind::Task { reason, .. } => reason.label(),
            NeedKind::PlanReady { .. } => unreachable!(),
        }
    }
}

pub(crate) struct NeedRow {
    pub chat_id: String,
    pub kind: NeedKind,
}

impl NeedRow {
    fn key(&self) -> String {
        match &self.kind {
            NeedKind::Decision { item_id, .. } => format!("decision-{item_id}"),
            NeedKind::Task { task_id, .. } => format!("task-{task_id}"),
            _ => self.chat_id.clone(),
        }
    }
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
            .find(|row| row.chat_id == chat_id && row.kind.of_chat())
            .map(|row| &row.kind)
    }
}

#[derive(Clone)]
struct ShownRow {
    key: String,
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
        let kept = |row: &ShownRow| rows.iter().any(|shown| shown.key == row.key);
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
    rows: Exits,
    card_rows: Exits,
    card_dim: [Option<crate::motion::Tween>; 3],
    ticker: Option<Task<()>>,
    asks: Vec<(String, usize, bool)>,
    frame: std::rc::Rc<Triage>,
}

impl TriageTray {
    // The shell does not re-render on composer notifies.
    pub(super) fn observe(composer: &Entity<Composer>, cx: &mut Context<Shell>) -> Subscription {
        cx.observe(composer, |shell, composer, cx| {
            let asks = composer.read(cx).ask_signature();
            if asks != shell.triage.asks {
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
        let mut tasks = Vec::new();
        for task in state
            .tasks
            .iter()
            .filter(|task| !task.deleted && task.completed_at.is_none())
        {
            let plan = crate::tasks::rollup(state, task, now);
            if let Some(reason @ (zeron_proto::NeedsYou::NotSent | zeron_proto::NeedsYou::Review)) =
                plan.needs
            {
                tasks.push(NeedRow {
                    chat_id: task.chat_id.clone().unwrap_or_default(),
                    kind: NeedKind::Task {
                        task_id: task.id.clone(),
                        title: task.title.clone(),
                        reason,
                    },
                });
            }
            if !task.plan_pending || plan.phase(task) != zeron_proto::mission::PlanPhase::PlanReady
            {
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
        let mut listed = std::collections::HashSet::new();
        for (status, chat) in state.overview_chats(now) {
            listed.insert(chat.id.as_str());
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
                        agents: session.map(background_agents).unwrap_or_default(),
                    });
                    None
                }
                zeron_proto::ChatIndicator::Idle | zeron_proto::ChatIndicator::Completed
                    if outlives_turn(state, &chat.id, now) =>
                {
                    let session = state.session_for(&chat.id);
                    triage.running.push(RunRow {
                        chat_id: chat.id.clone(),
                        started: session
                            .and_then(|session| {
                                session
                                    .background_tasks
                                    .iter()
                                    .map(|task| task.started_at)
                                    .min()
                            })
                            .or_else(|| session.and_then(|session| session.started_at)),
                        agents: session.map(background_agents).unwrap_or_default(),
                    });
                    plans
                        .remove(&chat.id)
                        .map(|(task_id, steps)| NeedKind::PlanReady { task_id, steps })
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
        for session in &state.sessions {
            let chat_id = session.chat_id.as_str();
            if session.status == zeron_proto::SessionStatus::AwaitingInput
                && !listed.contains(chat_id)
                && !state.is_hidden_chat(chat_id)
                && !composer.ask_answered(chat_id)
            {
                triage.needs.push(NeedRow {
                    chat_id: chat_id.to_owned(),
                    kind: NeedKind::Asking(composer.ask_info(chat_id)),
                });
            }
        }
        for row in state
            .supervisor
            .iter()
            .flat_map(|snapshot| &snapshot.needs_you)
        {
            if let zeron_proto::supervisor::NeedsYouRow::Item(item) = row
                && item.answer.is_none()
            {
                triage.needs.push(NeedRow {
                    chat_id: item.raiser_chat_id.clone(),
                    kind: NeedKind::Decision {
                        item_id: item.id.clone(),
                        title: item.title.clone(),
                        why: item.why,
                        detail: item.detail.clone(),
                        options: item.options.clone(),
                    },
                });
            }
        }
        triage.needs.extend(tasks);
        triage
    }

    pub(super) fn triage_open(&self) -> bool {
        self.triage.popup.is_open()
    }

    pub(crate) fn toggle_triage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.triage.popup.is_open() {
            self.close_triage(cx);
        } else {
            self.open_triage(window, cx);
        }
    }

    pub(crate) fn open_triage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        if self.triage.popup.begin_close() {
            popover::reap_popup(cx, |shell: &mut Self| &mut shell.triage.popup);
        }
        cx.notify();
    }

    pub(crate) fn open_notification_target(
        &mut self,
        chat_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let waits = match self.triage(cx).need(&chat_id) {
            Some(kind) if kind.waits() => true,
            Some(_) => return self.open_triage(window, cx),
            None => false,
        };
        let bound = self.composer.read(cx).bound_chat_id(cx);
        if waits && matches!(self.route, Route::Chat) && bound.as_deref() != Some(&chat_id) {
            self.open_waiting(chat_id, cx);
        } else {
            self.open_thread(chat_id, cx);
        }
    }

    fn open_waiting(&mut self, key: String, cx: &mut Context<Self>) {
        self.composer
            .update(cx, |composer, cx| composer.open_waiting(Some(key), cx));
    }

    pub(super) fn sync_waiting(&mut self, cx: &mut Context<Self>) {
        let triage = self.triage_frame();
        let items: Vec<WaitItem> = triage
            .needs
            .iter()
            .filter(|row| row.kind.waits())
            .map(|row| self.wait_item(row, cx))
            .collect();
        self.composer
            .update(cx, |composer, cx| composer.set_waiting(items, cx));
    }

    fn wait_item(&self, row: &NeedRow, cx: &App) -> WaitItem {
        let state = self.state.read(cx);
        let chat = state.chats.iter().find(|chat| chat.id == row.chat_id);
        let (kind, from) = match &row.kind {
            NeedKind::Decision {
                item_id,
                why,
                detail,
                options,
                ..
            } => (
                WaitKind::Decision {
                    item_id: item_id.clone(),
                    label: why.label(),
                    detail: detail.clone(),
                    options: options.clone(),
                },
                match chat {
                    Some(_) => format!("Supervisor, from {}", self.triage_title(&row.chat_id, cx)),
                    None => "Supervisor".into(),
                },
            ),
            kind => {
                let from = chat
                    .and_then(|chat| self.chat_project(chat, cx))
                    .unwrap_or_default();
                let kind = match kind {
                    NeedKind::PlanReady { task_id, steps } => WaitKind::Plan {
                        task_id: Some(task_id.clone()),
                        steps: steps.clone(),
                    },
                    NeedKind::Asking(Some(info)) if info.plan => WaitKind::Plan {
                        task_id: None,
                        steps: info.steps.clone(),
                    },
                    _ => WaitKind::Ask,
                };
                (kind, from)
            }
        };
        WaitItem {
            key: row.key(),
            chat_id: row.chat_id.clone(),
            title: self.need_title(row, cx),
            from,
            kind,
        }
    }

    pub(super) fn on_waiting_action(&mut self, action: &WaitingAction, cx: &mut Context<Self>) {
        match action {
            WaitingAction::ApprovePlan {
                chat_id,
                target,
                as_task,
            } => self.approve_plan(chat_id, target.clone(), *as_task, cx),
            WaitingAction::Decide { item_id, choice } => self.mutate(
                serde_json::json!({
                    "op": "resolveSupervisorItems",
                    "answers": [{ "itemId": item_id, "choice": choice }],
                }),
                cx,
            ),
            WaitingAction::Open { decision: true, .. } => self.open_supervisor(cx),
            WaitingAction::Open { chat_id, .. } => self.open_thread(chat_id.clone(), cx),
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
            .map(|chat| crate::transcript::single_line(&chat.display_title()))
            .unwrap_or_else(|| "New session".into())
    }

    fn need_title(&self, row: &NeedRow, cx: &App) -> String {
        match &row.kind {
            NeedKind::Decision { title, .. } | NeedKind::Task { title, .. }
                if !title.trim().is_empty() =>
            {
                crate::transcript::single_line(title)
            }
            NeedKind::Task { .. } => "Untitled task".into(),
            _ => self.triage_title(&row.chat_id, cx),
        }
    }

    fn open_need(&mut self, chat_id: &str, kind: &NeedKind, cx: &mut Context<Self>) {
        match kind {
            NeedKind::Decision { .. } => self.open_supervisor(cx),
            NeedKind::Task { task_id, .. } => self.defer_open_task(task_id.clone(), cx),
            _ => self.open_thread(chat_id.to_owned(), cx),
        }
    }

    fn shown_row(&self, row: &NeedRow, meta: String, cx: &App) -> ShownRow {
        ShownRow {
            key: row.key(),
            chat_id: row.chat_id.clone(),
            title: self.need_title(row, cx),
            meta,
            kind: row.kind.clone(),
        }
    }

    fn need_meta(&self, row: &NeedRow, cx: &App) -> String {
        let state = self.state.read(cx);
        let chat = state.chats.iter().find(|chat| chat.id == row.chat_id);
        let mut parts: Vec<String> = Vec::new();
        match &row.kind {
            NeedKind::Task { .. } => parts.push("Task".into()),
            _ => {
                parts.extend(chat.and_then(|chat| self.chat_project(chat, cx)));
                if let Some(parent) = chat.and_then(|chat| chat.delegated_by()) {
                    parts.push(format!("from {}", self.triage_title(parent, cx)));
                }
            }
        }
        parts.push(row.kind.label());
        match &row.kind {
            NeedKind::Failed => {
                if let Some(at) = state.session_for(&row.chat_id).map(|s| s.updated_at) {
                    parts.push(crate::settings::devices::format_last_seen(
                        Some(at),
                        Utc::now(),
                    ));
                }
            }
            NeedKind::Task { task_id, .. } => {
                if let Some(error) = state
                    .tasks
                    .iter()
                    .find(|task| &task.id == task_id)
                    .and_then(|task| task.send_error.as_deref())
                {
                    parts.push(crate::transcript::single_line(error));
                }
            }
            _ => {}
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
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let theme = Theme::of(cx).clone();
        let needs: Vec<&NeedKind> = triage
            .needs
            .iter()
            .map(|row| &row.kind)
            .filter(|kind| !kind.waits())
            .collect();
        let running = triage.running.len();
        let mounted = self.triage.popup.get().is_some();
        if needs.is_empty() && running == 0 && !mounted {
            return None;
        }
        let open = self.triage.popup.is_open();
        let marks = {
            let state = self.state.read(cx);
            let marks: Vec<InstanceIcon> = triage
                .running
                .iter()
                .map(|row| {
                    state
                        .chats
                        .iter()
                        .find(|chat| chat.id == row.chat_id)
                        .and_then(|chat| crate::pickers::chat_mark(chat, cx))
                        .unwrap_or_else(|| {
                            let (mark, tint) = crate::pickers::agent_brand_icon(
                                state.chat_harness(&row.chat_id),
                                None,
                            );
                            InstanceIcon::Brand(mark, tint)
                        })
                })
                .collect();
            pill_marks(marks)
        };
        let badge = needs_badge(&needs, &theme);
        let working = format!("{running} Working");
        let needing = badge.map(|(_, _, word)| format!("{} {word}", needs.len()));
        let label = [(running > 0).then(|| working.clone()), needing.clone()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(", ");
        let ring = gpui::Hsla {
            a: 1.0,
            ..theme.surface
        };
        let pill = div()
            .id("triage-pill")
            .debug_selector(|| "triage-pill".into())
            .role(gpui::Role::Button)
            .aria_label(SharedString::from(label))
            .aria_expanded(open)
            .h(px(26.0))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.0))
            .pl(px(if running > 0 { 5.0 } else { 9.0 }))
            .pr(px(9.0))
            .rounded(px(13.0))
            .border_1()
            .border_color(theme.border)
            .bg(if open {
                theme.element_hover
            } else {
                crate::motion::hover_blend(
                    "triage-pill",
                    theme.element_hover.opacity(0.0),
                    theme.element_hover,
                )
            })
            .on_hover(crate::motion::hover_listener("triage-pill"))
            .cursor_pointer()
            .text_size(crate::typography::ui_rems(12.5))
            .text_color(theme.text)
            .whitespace_nowrap()
            .when(running > 0, |el| {
                el.child(mark_chips(&marks, ring, &theme))
                    .child(SharedString::from(working))
            })
            .when(running > 0 && badge.is_some(), |el| {
                el.child(
                    div()
                        .flex_none()
                        .w(px(1.0))
                        .h(px(12.0))
                        .bg(theme.border_strong),
                )
            })
            .when_some(badge.zip(needing), |el, ((glyph, tint, _), text)| {
                el.child(icons::icon(glyph).size(px(15.0)).text_color(tint))
                    .child(SharedString::from(text))
            })
            .tooltip(crate::settings::widgets::text_tooltip(format!(
                "What needs you and what is running · {}",
                crate::settings::badge_combo(self.settings.keymap.get(ShortcutId::ToggleTray))
            )))
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
                    this.open_triage(window, cx);
                }
            }));
        let tray = mounted.then(|| self.render_triage_tray(triage, &theme, cx));
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
                .map(|row| self.need_title(row, cx))
                .collect()
        };
        for title in titles(&|kind| matches!(kind, NeedKind::Asking(None))) {
            parts.push(format!("{title} asks you."));
        }
        match triage.needs.iter().filter(|row| row.kind.waits()).count() {
            0 => {}
            1 => parts.push("1 waits in the composer.".into()),
            n => parts.push(format!("{n} wait in the composer.")),
        }
        let failed = titles(&|kind| matches!(kind, NeedKind::Failed));
        if !failed.is_empty() {
            parts.push(format!("{} failed.", failed.join(", ")));
        }
        let review = titles(&|kind| {
            matches!(
                kind,
                NeedKind::Task {
                    reason: zeron_proto::NeedsYou::Review,
                    ..
                }
            )
        });
        if !review.is_empty() {
            parts.push(format!("{} ready to review.", review.join(", ")));
        }
        let unsent = titles(&|kind| {
            matches!(
                kind,
                NeedKind::Task {
                    reason: zeron_proto::NeedsYou::NotSent,
                    ..
                }
            )
        });
        if !unsent.is_empty() {
            parts.push(format!("{} not sent.", unsent.join(", ")));
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
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let popup = theme.for_popup();
        let now = crate::motion::now();
        let reduced = crate::motion::reduced_motion(cx);
        let mut animating = false;
        let unread = !self.state.read(cx).unseen_chat_ids().is_empty();

        let shown: Vec<ShownRow> = triage
            .needs
            .iter()
            .filter(|row| !row.kind.waits())
            .map(|row| self.shown_row(row, self.need_meta(row, cx), cx))
            .collect();
        self.triage.rows.update(shown.clone(), reduced, now);
        let need_count = shown.len();

        let mut need_rows: Vec<AnyElement> = shown
            .iter()
            .map(|row| self.render_need_row(row, &popup, cx))
            .collect();
        for (ix, row, tween) in self.triage.rows.leaving.clone() {
            animating = true;
            let t = tween.value_at(now);
            let ghost = div()
                .h(px(ROW_HEIGHT * t))
                .opacity(t)
                .overflow_hidden()
                .child(
                    need_row_shell(&row, &popup)
                        .id(SharedString::from(format!("triage-leaving-{}", row.key))),
                )
                .into_any_element();
            need_rows.insert(ix.min(need_rows.len()), ghost);
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
            .bg(popup.surface_overlay)
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
                        need_count,
                        popup.accent,
                        unread.then(|| {
                            mark_all_read_button(
                                &popup,
                                "triage-mark-all-read",
                                "Mark chats read",
                                cx,
                            )
                        }),
                    ))
                    .children(need_rows)
                    .child(section_heading(
                        &popup,
                        "Running",
                        triage.running.len(),
                        chat_status_tone(zeron_proto::ChatIndicator::Working, &popup).2,
                        None,
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
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = row.key.clone();
        let (target, kind) = (row.chat_id.clone(), row.kind.clone());
        let open = popover::btn_primary(theme, "Open")
            .id(SharedString::from(format!("triage-open-{key}")))
            .debug_selector({
                let id = key.clone();
                move || format!("triage-open-{id}")
            })
            .role(gpui::Role::Button)
            .aria_label(format!("Open {}", row.title))
            .flex_none()
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.close_triage(cx);
                this.open_need(&target, &kind, cx);
            }));
        let (open_target, open_kind) = (row.chat_id.clone(), row.kind.clone());
        need_row_shell(row, theme)
            .id(SharedString::from(format!("triage-need-{key}")))
            .debug_selector(move || format!("triage-need-{key}"))
            .role(gpui::Role::Button)
            .aria_label(row.title.clone())
            .cursor_pointer()
            .rounded(px(9.0))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.close_triage(cx);
                this.open_need(&open_target, &open_kind, cx);
            }))
            .child(open)
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
        let (mark, tint) =
            crate::pickers::agent_brand_icon(self.state.read(cx).chat_harness(&chat_id), None);
        let agents = row
            .agents
            .iter()
            .take(SHOWN_AGENTS)
            .enumerate()
            .map(|(ix, (label, started))| {
                let (parent, agent) = (chat_id.clone(), label.clone());
                div()
                    .id(SharedString::from(format!("triage-agent-{chat_id}-{ix}")))
                    .debug_selector({
                        let chat_id = chat_id.clone();
                        move || format!("triage-agent-{chat_id}-{ix}")
                    })
                    .role(gpui::Role::Button)
                    .aria_label(SharedString::from(format!(
                        "Open agent {}",
                        crate::transcript::single_line(label)
                    )))
                    .cursor_pointer()
                    .rounded(px(6.0))
                    .hover(|s| s.bg(theme.wash(0.05)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.close_triage(cx);
                        this.open_agent_of(parent.clone(), agent.clone(), cx);
                    }))
                    .flex()
                    .items_center()
                    .gap(px(7.0))
                    .py(px(3.0))
                    .min_w_0()
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .child(
                        icon(mark)
                            .size(px(12.0))
                            .text_color(tint.unwrap_or(theme.text_muted)),
                    )
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
            })
            .collect::<Vec<_>>();
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
                this.open_thread(target.clone(), cx);
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
            .map(|row| {
                let failed_at = self
                    .state
                    .read(cx)
                    .session_for(&row.chat_id)
                    .map(|session| session.updated_at);
                self.shown_row(row, card_need_meta(&row.kind, failed_at), cx)
            })
            .collect();
        self.triage.card_rows.update(rows.clone(), reduced, now);
        let mut animating = false;

        let mut need_rows: Vec<AnyElement> = rows
            .iter()
            .take(CARD_ROWS)
            .map(|row| self.render_card_need_row(row, &theme, cx))
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
                    SharedString::from(format!("card-leaving-{}", row.key)),
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
                .on_click(cx.listener(move |this, _, _, cx| this.open_thread(chat_id.clone(), cx)))
                .into_any_element()
            })
            .collect();

        let unread = !self.state.read(cx).unseen_chat_ids().is_empty();
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
                .on_click(cx.listener(move |this, _, _, cx| this.open_thread(target.clone(), cx)))
                .into_any_element()
            })
            .collect();

        let more = |id: &'static str,
                    hidden: usize,
                    label: String,
                    waiting: bool,
                    cx: &mut Context<Self>| {
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
                    .when(!waiting, |el| {
                        el.child(popover::kbd_hint(
                            &theme,
                            &crate::settings::badge_combo(
                                self.settings.keymap.get(ShortcutId::ToggleTray),
                            ),
                        ))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if waiting {
                            this.composer
                                .update(cx, |composer, cx| composer.open_waiting(None, cx));
                        } else {
                            this.open_triage(window, cx);
                        }
                    }))
                    .into_any_element()
            })
        };
        let hidden_needs = triage.needs.len().saturating_sub(CARD_ROWS);
        let hidden_wait = triage
            .needs
            .iter()
            .skip(CARD_ROWS)
            .all(|row| row.kind.waits());
        let needs_more = more(
            "card-needs-more",
            hidden_needs,
            if hidden_wait {
                format!("{hidden_needs} more waiting")
            } else {
                format!("{hidden_needs} more in the tray")
            },
            hidden_wait,
            cx,
        );
        let running_more = more(
            "card-running-more",
            triage.running.len().saturating_sub(CARD_ROWS),
            format!("{} more", triage.running.len().saturating_sub(CARD_ROWS)),
            false,
            cx,
        );

        let mut dims = [0.0; 3];
        let empties = [
            need_rows.is_empty(),
            run_rows.is_empty(),
            recent_rows.is_empty(),
        ];
        for (dim, (slot, empty)) in dims
            .iter_mut()
            .zip(self.triage.card_dim.iter_mut().zip(empties))
        {
            let target = if empty { 1.0 } else { 0.0 };
            let tween = slot.get_or_insert_with(|| crate::motion::Tween::at(target, now));
            if tween.target() != target {
                tween.retarget(target, crate::motion::FADE_QUICK, reduced, now);
            }
            animating |= !tween.settled_at(now);
            *dim = tween.value_at(now);
        }
        let swap = if unread {
            crate::motion::hover_t("card-needs")
        } else {
            0.0
        };
        if animating {
            window.request_animation_frame();
        }
        div()
            .id("new-work-cards")
            .debug_selector(|| "new-work-cards".into())
            .w_full()
            .flex()
            .items_stretch()
            .text_size(crate::typography::ui_rems(13.0))
            .child(
                work_card(
                    &theme,
                    "card-needs",
                    icons::BELL,
                    "Needs you",
                    Some(triage.needs.len()),
                    need_rows,
                    needs_more,
                    unread.then(|| {
                        mark_all_read_button(&theme, "card-mark-all-read", "Mark read", cx)
                    }),
                    swap,
                    dims[0],
                    true,
                )
                .on_hover(crate::motion::hover_listener("card-needs")),
            )
            .child(work_card(
                &theme,
                "card-running",
                icons::SPEEDOMETER,
                "Running",
                Some(triage.running.len()),
                run_rows,
                running_more,
                None,
                0.0,
                dims[1],
                false,
            ))
            .child(work_card(
                &theme,
                "card-recent",
                icons::CLOCK_CIRCLE,
                "Recent",
                None,
                recent_rows,
                None,
                None,
                0.0,
                dims[2],
                false,
            ))
            .into_any_element()
    }

    fn render_card_need_row(
        &mut self,
        row: &ShownRow,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = row.key.clone();
        let action = if row.kind.waits() {
            let label = if row.kind.plan() { "Review" } else { "Answer" };
            let target = row.key.clone();
            card_button(theme, true)
                .id(SharedString::from(format!("card-answer-{key}")))
                .role(gpui::Role::Button)
                .aria_label(format!("{label} {}", row.title))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.open_waiting(target.clone(), cx);
                }))
        } else {
            let (target, kind) = (row.chat_id.clone(), row.kind.clone());
            card_button(theme, false)
                .id(SharedString::from(format!("card-open-{key}")))
                .role(gpui::Role::Button)
                .aria_label(format!("Open {}", row.title))
                .child("Open")
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.open_need(&target, &kind, cx);
                }))
        };
        let tint = need_tint(&row.kind, theme);
        let (open_target, open_kind) = (row.chat_id.clone(), row.kind.clone());
        card_row_shell(
            theme,
            SharedString::from(format!("card-need-{key}")),
            tint,
            row.title.clone(),
            row.meta.clone(),
            tint,
        )
        .role(gpui::Role::Button)
        .aria_label(row.title.clone())
        .cursor_pointer()
        .hover(|s| s.bg(theme.wash(0.05)))
        .on_click(cx.listener(move |this, _, _, cx| this.open_need(&open_target, &open_kind, cx)))
        .child(action)
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
                    crate::transcript::single_line(&chat.display_title()),
                    meta,
                    completed,
                )
            })
            .collect()
    }
}

fn background_agents(session: &zeron_proto::Session) -> Vec<(String, Option<DateTime<Utc>>)> {
    session
        .background_tasks
        .iter()
        .filter(|task| task.kind == zeron_proto::BackgroundTaskKind::Agent)
        .map(|task| (task.command.clone(), Some(task.started_at)))
        .collect()
}

fn outlives_turn(state: &crate::state::AppState, chat_id: &str, now: DateTime<Utc>) -> bool {
    state
        .session_for(chat_id)
        .is_some_and(|session| !session.background_tasks.is_empty())
        || state.running_subagents_for(chat_id, now) > 0
        || state.chats.iter().any(|chat| {
            chat.delegation.as_ref().is_some_and(|d| {
                d.delegated_by_chat_id == chat_id
                    && d.status == zeron_proto::DelegationStatus::Running
            })
        })
}

const CARD_ROWS: usize = 3;
const CARD_ROW_HEIGHT: f32 = 46.0;

fn need_tint(kind: &NeedKind, theme: &Theme) -> gpui::Hsla {
    match kind {
        NeedKind::Failed
        | NeedKind::Task {
            reason: zeron_proto::NeedsYou::NotSent,
            ..
        } => theme.danger,
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
        NeedKind::Decision { why, .. } => format!("Supervisor · {}", why.label()),
        NeedKind::Task { reason, .. } => format!("Task · {}", reason.label()),
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
    rows: Vec<AnyElement>,
    more: Option<AnyElement>,
    action: Option<AnyElement>,
    swap: f32,
    dim: f32,
    first: bool,
) -> gpui::Stateful<gpui::Div> {
    let empty = rows.is_empty();
    div()
        .id(id)
        .debug_selector(move || id.into())
        .role(gpui::Role::Group)
        .aria_label(title)
        .flex_1()
        .min_w_0()
        .pt(px(10.0))
        .px(px(8.0))
        .pb(px(8.0))
        .when(!first, |el| el.border_l_1().border_color(theme.border))
        .opacity(1.0 - 0.55 * dim)
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
                .child(
                    div()
                        .ml_auto()
                        .relative()
                        .h(px(20.0))
                        .flex()
                        .items_center()
                        .when(!empty, |el| {
                            el.children(count.map(|count| {
                                div()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .opacity(1.0 - swap)
                                    .child(SharedString::from(count.to_string()))
                            }))
                        })
                        .when(swap > 0.001, |el| {
                            el.children(action.map(|action| {
                                div()
                                    .absolute()
                                    .right(px(-4.0))
                                    .top_0()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .opacity(swap)
                                    .child(action)
                            }))
                        }),
                ),
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
}

pub(super) fn mark_all_read_button(
    theme: &Theme,
    id: &'static str,
    label: &'static str,
    cx: &mut Context<Shell>,
) -> AnyElement {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .role(gpui::Role::Button)
        .aria_label("Mark chats read")
        .tooltip(crate::settings::widgets::text_tooltip(
            "Clear the unread dot on every chat. Decisions stay until you answer or dismiss them.",
        ))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(4.0))
        .h(px(20.0))
        .px(px(6.0))
        .rounded(px(5.0))
        .cursor_pointer()
        .text_size(crate::typography::ui_rems(11.5))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(theme.text_muted)
        .hover(|s| s.bg(theme.wash(0.06)).text_color(theme.text))
        .on_click(cx.listener(|this, _, _, cx| {
            cx.stop_propagation();
            this.state
                .update(cx, |state, cx| state.mark_all_chats_seen(cx));
        }))
        .child(
            icon(icons::LUCIDE_CHECK)
                .size(px(12.0))
                .text_color(theme.text_muted),
        )
        .child(label)
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

const PILL_MARKS: usize = 3;
const MARK_CHIP: f32 = 18.0;
const MARK_RING: f32 = 1.5;
const MARK_STEP: f32 = MARK_CHIP - 5.0;

fn pill_marks(marks: Vec<InstanceIcon>) -> (Vec<InstanceIcon>, usize) {
    let mut distinct: Vec<InstanceIcon> = Vec::new();
    for mark in marks {
        if !distinct.contains(&mark) {
            distinct.push(mark);
        }
    }
    let more = distinct.len().saturating_sub(PILL_MARKS);
    distinct.truncate(PILL_MARKS);
    (distinct, more)
}

fn needs_badge(
    needs: &[&NeedKind],
    theme: &Theme,
) -> Option<(&'static str, gpui::Hsla, &'static str)> {
    if needs.is_empty() {
        return None;
    }
    let failed = |kind: &&NeedKind| matches!(kind, NeedKind::Failed);
    let asking = |kind: &&NeedKind| matches!(kind, NeedKind::Asking(None));
    let alert = |tint| (icons::LUCIDE_CIRCLE_ALERT, tint);
    let ask = (
        icons::LUCIDE_MESSAGE_CIRCLE_QUESTION,
        chat_status_tone(zeron_proto::ChatIndicator::AwaitingInput, theme).2,
    );
    let ((glyph, tint), word) = if needs.iter().all(failed) {
        (alert(theme.danger), "Failed")
    } else if needs.iter().all(asking) {
        (ask, "Asking")
    } else if needs.iter().any(failed) {
        (alert(theme.danger), "Need you")
    } else if needs.iter().any(asking) {
        (ask, "Need you")
    } else {
        (alert(theme.warning), "Need you")
    };
    Some((glyph, tint, word))
}

fn mark_chips(
    (marks, more): &(Vec<InstanceIcon>, usize),
    ring: gpui::Hsla,
    theme: &Theme,
) -> gpui::Div {
    let chip = |fill: gpui::Hsla, content: AnyElement| {
        div()
            .size(px(MARK_CHIP + MARK_RING * 2.0))
            .rounded_full()
            .bg(ring)
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .size(px(MARK_CHIP))
                    .rounded_full()
                    .bg(fill)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(content),
            )
    };
    let mut chips: Vec<gpui::Div> = marks
        .iter()
        .map(|mark| {
            chip(
                theme.surface_card,
                mark.svg(theme.text).size(px(11.0)).into_any_element(),
            )
        })
        .collect();
    if *more > 0 {
        chips.push(chip(
            theme.wash(0.06),
            div()
                .text_size(crate::typography::ui_rems(10.0))
                .text_color(theme.text_muted)
                .child(SharedString::from(format!("+{more}")))
                .into_any_element(),
        ));
    }
    let width = MARK_CHIP + MARK_STEP * chips.len().saturating_sub(1) as f32;
    div()
        .flex_none()
        .relative()
        .w(px(width))
        .h(px(MARK_CHIP))
        .children(chips.into_iter().enumerate().map(|(ix, chip)| {
            chip.absolute()
                .top(px(-MARK_RING))
                .left(px(MARK_STEP * ix as f32 - MARK_RING))
        }))
}

fn need_row_shell(row: &ShownRow, theme: &Theme) -> gpui::Div {
    let (glyph, tint) = match &row.kind {
        NeedKind::Failed => (icons::LUCIDE_CIRCLE_ALERT, theme.danger),
        kind @ NeedKind::Task { .. } => (icons::TASKS, need_tint(kind, theme)),
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

fn section_heading(
    theme: &Theme,
    label: &str,
    count: usize,
    tint: gpui::Hsla,
    action: Option<AnyElement>,
) -> AnyElement {
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
        .children(action.map(|action| div().ml_auto().child(action)))
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext, TestAppContext};

    #[test]
    fn pill_stacks_distinct_providers_and_names_the_need() {
        let mark = |harness| InstanceIcon::harness(harness);
        let (shown, more) = pill_marks(vec![
            mark(zeron_proto::HarnessId::ClaudeCode),
            mark(zeron_proto::HarnessId::Codex),
            mark(zeron_proto::HarnessId::ClaudeCode),
            mark(zeron_proto::HarnessId::Cursor),
            mark(zeron_proto::HarnessId::Grok),
            mark(zeron_proto::HarnessId::Pi),
        ]);
        assert_eq!(
            shown,
            [
                mark(zeron_proto::HarnessId::ClaudeCode),
                mark(zeron_proto::HarnessId::Codex),
                mark(zeron_proto::HarnessId::Cursor),
            ]
        );
        assert_eq!(more, 2);

        let theme = Theme::default();
        let task = NeedKind::Task {
            task_id: "t".into(),
            title: "t".into(),
            reason: zeron_proto::NeedsYou::Review,
        };
        let ask = NeedKind::Asking(None);
        let word = |needs: &[&NeedKind]| needs_badge(needs, &theme);
        let tone = chat_status_tone(zeron_proto::ChatIndicator::AwaitingInput, &theme).2;
        let alert = icons::LUCIDE_CIRCLE_ALERT;
        let question = icons::LUCIDE_MESSAGE_CIRCLE_QUESTION;
        assert_eq!(word(&[]), None);
        assert_eq!(
            word(&[&NeedKind::Failed, &NeedKind::Failed]),
            Some((alert, theme.danger, "Failed"))
        );
        assert_eq!(word(&[&ask]), Some((question, tone, "Asking")));
        assert_eq!(
            word(&[&ask, &NeedKind::Failed]),
            Some((alert, theme.danger, "Need you"))
        );
        assert_eq!(word(&[&ask, &task]), Some((question, tone, "Need you")));
        assert_eq!(word(&[&task]), Some((alert, theme.warning, "Need you")));
    }

    #[gpui::test]
    fn triage_lists_every_thread_that_needs_you_or_still_runs(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            crate::app_menus::init(cx);
            crate::terminal::panel::init(cx);
            settings::init(UiSettings::default(), dir.path(), cx);
            crate::history::init(
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                cx,
            );
        });
        let now = Utc::now();
        let chat = |id: &str, delegated_by: Option<&str>| -> zeron_proto::Chat {
            let mut chat = serde_json::json!({
                "id": id, "deviceId": "mac", "title": id, "archived": false, "createdAt": now,
            });
            if let Some(owner) = delegated_by {
                chat["delegation"] = serde_json::json!({
                    "delegatedByChatId": owner, "depth": 1, "anchor": "task", "wake": "always",
                    "status": "running", "startedAt": now,
                });
            }
            serde_json::from_value(chat).unwrap()
        };
        let session = |id: &str, status, background_tasks| zeron_proto::Session {
            last_turn_outcome: None,
            terminal_activity: None,
            last_completed_turn: None,
            chat_id: id.into(),
            device_id: "mac".into(),
            status,
            held_by_subagents: false,
            compacting: false,
            classifying: None,
            background_tasks,
            started_at: None,
            updated_at: now,
            running_subagents: 0,
            compact_plan: None,
        };
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| {
            state.local_device_id = Some("mac".into());
            state.apply_chats(vec![
                chat("lead", None),
                chat("asker", Some("lead")),
                chat("shelled", None),
                chat("quiet", None),
                chat("review", None),
            ]);
            state.apply_sessions(vec![
                session("asker", zeron_proto::SessionStatus::AwaitingInput, vec![]),
                session(
                    "shelled",
                    zeron_proto::SessionStatus::Idle,
                    vec![zeron_proto::BackgroundTask {
                        id: "build".into(),
                        command: "build".into(),
                        started_at: now,
                        ..Default::default()
                    }],
                ),
                session("quiet", zeron_proto::SessionStatus::Idle, vec![]),
                session("review", zeron_proto::SessionStatus::Idle, vec![]),
            ]);
            state.tasks = vec![
                serde_json::from_value(serde_json::json!({
                    "id": "unsent", "title": "Ship it", "sendError": "host offline",
                }))
                .unwrap(),
                serde_json::from_value(serde_json::json!({
                    "id": "reviewed", "title": "Check it", "chatId": "review",
                }))
                .unwrap(),
            ];
            state.supervisor = Some(zeron_proto::supervisor::SupervisorSnapshot {
                needs_you: vec![zeron_proto::supervisor::NeedsYouRow::Item(
                    zeron_proto::supervisor::SupervisorItem {
                        id: "d1".into(),
                        at: now,
                        raised_by: zeron_proto::supervisor::Raiser::Supervisor,
                        raiser_chat_id: "lead".into(),
                        task_id: None,
                        title: "Spend more?".into(),
                        why: zeron_proto::supervisor::ItemWhy::Spend,
                        detail: String::new(),
                        options: Vec::new(),
                        answer: None,
                    },
                )],
                ..Default::default()
            });
        });
        let boot = EngineBootConfig {
            data_dir: dir.path().into(),
            ipc_port: 0,
            edge_url: "http://127.0.0.1:1".into(),
            edge_token: None,
            org_id: None,
            workos_client_id: None,
            default_harness: zeron_proto::HarnessId::Mock,
        };
        let window = cx.add_window(|_, cx| Shell::new(state, boot, cx));
        let (needs, asker_meta, mut running, waiting, summary) = window
            .update(cx, |shell, _, cx| {
                let triage = shell.triage(cx);
                let needs: Vec<String> = triage
                    .needs
                    .iter()
                    .filter(|row| !row.kind.waits())
                    .map(NeedRow::key)
                    .collect();
                let waiting: Vec<(String, WaitKind)> = triage
                    .needs
                    .iter()
                    .filter(|row| row.kind.waits())
                    .map(|row| shell.wait_item(row, cx))
                    .map(|item| (item.key, item.kind))
                    .collect();
                let asker = triage.needs.iter().find(|row| row.chat_id == "asker");
                let meta = asker.map(|row| shell.need_meta(row, cx));
                let running: Vec<String> = triage
                    .running
                    .iter()
                    .map(|row| row.chat_id.clone())
                    .collect();
                let summary = shell.supervisor_summary(&triage, cx);
                (needs, meta, running, waiting, summary)
            })
            .unwrap();
        running.sort();
        assert_eq!(
            needs,
            ["asker", "task-unsent", "task-reviewed"],
            "a question whose details have not loaded stays in the tray"
        );
        assert_eq!(
            waiting,
            [(
                "decision-d1".into(),
                WaitKind::Decision {
                    item_id: "d1".into(),
                    label: "Spend",
                    detail: String::new(),
                    options: Vec::new(),
                }
            )],
            "decisions wait at the composer, not in the titlebar tray"
        );
        assert!(
            asker_meta.is_some_and(|meta| meta.contains("from lead")),
            "a delegated child's row names its parent"
        );
        assert_eq!(running, ["lead", "shelled"]);
        assert_eq!(
            summary,
            "asker asks you. 1 waits in the composer. Check it ready to review. Ship it not sent. 2 threads are running."
        );
    }
}
