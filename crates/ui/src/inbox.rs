use std::time::{Duration, Instant};

use chrono::{DateTime, Local, Utc};
use gpui::{
    AnyElement, ClickEvent, Context, Entity, EventEmitter, IntoElement, Render, SharedString,
    Subscription, Task, Window, div, prelude::*, px, relative,
};
use zeron_proto::today::{
    Part, day_slots, digest, is_overdue, today_tasks, work_day, work_day_start,
};
use zeron_proto::{Chat, SessionStatus, TodayRun, TodayRunStatus, TodayRuns, TodayRunsRequest};
use zeron_rpc::methods;

use crate::composer::{ComposerInput, ComposerInputEvent, MentionTab};
use crate::icons::{self, icon};
use crate::settings::widgets;
use crate::state::AppState;
use crate::tasks::{CardFocus, TaskCard, TaskCardEvent};
use crate::theme::Theme;
use crate::typography::ui_rems;

const PAGE_MAX_WIDTH: f32 = 1760.0;
const TILE_RADIUS: f32 = 16.0;
const CARD_WIDTH: f32 = 352.0;
const CHECK_SIZE: f32 = 18.0;
const TICK: Duration = Duration::from_secs(30);
const RUNS_EVERY: Duration = Duration::from_secs(60);
const ADD_PLACEHOLDER: &str = "Add a task for today, or press Tab to hand it to an agent";

pub enum InboxEvent {
    OpenChat(String),
    OpenTray,
    OpenTasks,
    OpenTaskCenter(String),
    CreateTask {
        task_id: String,
        title: String,
        due_on: String,
    },
    SetTaskCompleted {
        task_id: String,
        completed: bool,
    },
    Card(TaskCardEvent),
}

pub struct InboxPage {
    state: Entity<AppState>,
    add_input: Entity<ComposerInput>,
    card: Entity<TaskCard>,
    open_first_card: bool,
    needs: usize,
    tray_combo: String,
    runs: Option<TodayRuns>,
    runs_failed: bool,
    runs_fetched: Option<Instant>,
    runs_updated: Option<DateTime<Local>>,
    runs_task: Option<Task<()>>,
    working_seen: usize,
    last_render: Option<Instant>,
    _ticker: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<InboxEvent> for InboxPage {}

impl InboxPage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let add_input = cx.new(|cx| {
            ComposerInput::new(ADD_PLACEHOLDER, cx)
                .with_single_line()
                .with_max_chars(zeron_doc::TASK_TITLE_MAX_CHARS)
                .with_text_metrics(13.5, 18.0)
                .with_accessibility_role(gpui::Role::TextInput)
        });
        let ticker = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                if this.update(cx, |this, cx| this.tick(cx)).is_err() {
                    break;
                }
            }
        });
        let card = cx.new(|cx| TaskCard::new(state.clone(), cx));
        let subscriptions = vec![
            cx.subscribe(&add_input, |page, _, event, cx| {
                if matches!(event, ComposerInputEvent::Submitted) {
                    page.submit_add(None, cx);
                }
            }),
            cx.subscribe(&card, |_, _, event: &TaskCardEvent, cx| {
                cx.emit(InboxEvent::Card(event.clone()))
            }),
            cx.observe(&card, |_, _, cx| cx.notify()),
            cx.observe(&state, |this, _, cx| this.on_state_changed(cx)),
        ];
        let mut page = Self {
            state,
            add_input,
            card,
            open_first_card: std::env::var("ZERON_OPEN_DIALOG").as_deref() == Ok("task-card"),
            needs: 0,
            tray_combo: String::new(),
            runs: None,
            runs_failed: false,
            runs_fetched: None,
            runs_updated: None,
            runs_task: None,
            working_seen: 0,
            last_render: None,
            _ticker: ticker,
            _subscriptions: subscriptions,
        };
        page.open_knob_card(cx);
        page
    }

    pub fn card(&self) -> &Entity<TaskCard> {
        &self.card
    }

    pub fn set_triage(&mut self, needs: usize, tray_combo: String, cx: &mut Context<Self>) {
        if (self.needs, &self.tray_combo) != (needs, &tray_combo) {
            self.needs = needs;
            self.tray_combo = tray_combo;
            cx.notify();
        }
    }

    fn submit_add(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) -> bool {
        let title = self.add_input.read(cx).text().trim().to_owned();
        if title.is_empty() {
            return false;
        }
        self.add_input
            .update(cx, |input, cx| input.set_text("", cx));
        let task_id = uuid::Uuid::new_v4().to_string();
        cx.emit(InboxEvent::CreateTask {
            task_id: task_id.clone(),
            title: title.clone(),
            due_on: work_day(Local::now()).format("%Y-%m-%d").to_string(),
        });
        if window.is_some() {
            self.card.update(cx, |card, cx| {
                card.open(task_id, Some(title), CardFocus::Agent, window, cx)
            });
        }
        true
    }

    fn open_task(&mut self, task_id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.card.update(cx, |card, cx| {
            card.open(task_id, None, CardFocus::Title, Some(window), cx)
        });
    }

    fn open_knob_card(&mut self, cx: &mut Context<Self>) {
        if !self.open_first_card {
            return;
        }
        let now = Local::now();
        let first = {
            let state = self.state.read(cx);
            today_tasks(
                &state.tasks,
                &state.chats,
                work_day(now),
                work_day_start(now).with_timezone(&Utc),
            )
            .into_iter()
            .find(|task| task.completed_at.is_none())
            .map(|task| task.id.clone())
        };
        if let Some(id) = first {
            self.open_first_card = false;
            self.card.update(cx, |card, cx| {
                card.open(id, None, CardFocus::Title, None, cx)
            });
        }
    }

    fn visible(&self) -> bool {
        self.last_render
            .is_some_and(|at| at.elapsed() < TICK + Duration::from_secs(5))
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        if self.visible() {
            self.refresh(cx);
            cx.notify();
        }
    }

    fn on_state_changed(&mut self, cx: &mut Context<Self>) {
        self.open_knob_card(cx);
        let working = self
            .state
            .read(cx)
            .sessions
            .iter()
            .filter(|session| session.status == SessionStatus::Working)
            .count();
        if working != self.working_seen {
            self.working_seen = working;
            self.runs_fetched = None;
        }
        if self.visible() {
            self.refresh(cx);
            cx.notify();
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let due = self
            .runs_fetched
            .is_none_or(|at| Instant::now() - at >= RUNS_EVERY);
        if self.runs_task.is_none() && due {
            self.fetch_runs(cx);
        }
    }

    fn fetch_runs(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let request = TodayRunsRequest {
            since_ms: work_day_start(Local::now()).timestamp_millis(),
            chat_ids: None,
        };
        let params = serde_json::to_value(request).unwrap_or_default();
        self.runs_task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call_as::<TodayRuns>(methods::GET_TODAY_RUNS, params)
                .await
                .map_err(|error| error.to_string());
            this.update(cx, |this, cx| {
                this.runs_task = None;
                this.runs_fetched = Some(Instant::now());
                this.runs_failed = result.is_err();
                if let Ok(runs) = result {
                    this.runs_updated = Some(Local::now());
                    this.runs = Some(runs);
                }
                cx.notify();
            })
            .ok();
        }));
    }
}

fn tile(theme: &Theme, id: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .w_full()
        .min_w_0()
        .rounded(px(TILE_RADIUS))
        .bg(if theme.is_frost() {
            theme.wash(0.045)
        } else {
            theme.surface_card
        })
}

fn group_heading(
    theme: &Theme,
    label: &str,
    count: usize,
    action: Option<AnyElement>,
) -> gpui::Div {
    div()
        .h(px(28.0))
        .px(px(12.0))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.0))
        .text_size(ui_rems(12.5))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .text_color(theme.text_muted)
                .child(SharedString::from(label.to_owned()))
                .child(
                    div()
                        .text_color(theme.text_faint)
                        .child(SharedString::from(count.to_string())),
                ),
        )
        .children(action)
}

fn task_row(theme: &Theme, id: impl Into<gpui::ElementId>) -> gpui::Stateful<gpui::Div> {
    let hover = theme.wash(0.06);
    div()
        .id(id)
        .w_full()
        .min_w_0()
        .px(px(12.0))
        .py(px(8.0))
        .rounded(px(12.0))
        .flex()
        .items_start()
        .gap(px(10.0))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
}

fn row_text(theme: &Theme, title: String, meta: String) -> gpui::Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(
            div()
                .truncate()
                .text_size(ui_rems(13.5))
                .line_height(ui_rems(20.0))
                .text_color(theme.text)
                .child(SharedString::from(title)),
        )
        .child(
            div()
                .truncate()
                .text_size(ui_rems(12.5))
                .text_color(theme.text_muted)
                .child(SharedString::from(meta)),
        )
}

fn chat_title(chat: &Chat) -> String {
    crate::transcript::single_line(&crate::home::chat_title(chat))
}

impl InboxPage {
    fn render_banner(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.needs == 0 {
            return None;
        }
        let accent = theme.accent;
        let line = match self.needs {
            1 => "1 thing needs you right now.".to_owned(),
            n => format!("{n} things need you right now."),
        };
        Some(
            tile(theme, "inbox-tray-banner")
                .px(px(16.0))
                .py(px(12.0))
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(
                    icon(icons::BELL)
                        .size(px(16.0))
                        .text_color(theme.text_muted),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(ui_rems(13.5))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(line)),
                )
                .child(
                    div()
                        .id("inbox-open-tray")
                        .debug_selector(|| "inbox-open-tray".into())
                        .role(gpui::Role::Button)
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .h(px(30.0))
                        .px(px(12.0))
                        .rounded(px(8.0))
                        .bg(accent.opacity(0.12))
                        .hover(move |s| s.bg(accent.opacity(0.18)))
                        .cursor_pointer()
                        .text_size(ui_rems(13.0))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(accent)
                        .child("Open tray")
                        .child(crate::popover::kbd_hint(theme, &self.tray_combo))
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(InboxEvent::OpenTray))),
                )
                .into_any_element(),
        )
    }

    fn render_history(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let now = Utc::now();
        let state = self.state.read(cx);
        let title = |chat_id: &str, fallback: Option<&String>| {
            state
                .chats
                .iter()
                .find(|chat| chat.id == chat_id)
                .map(chat_title)
                .or_else(|| fallback.map(|title| crate::transcript::single_line(title)))
                .unwrap_or_else(|| "New session".to_owned())
        };
        let mut items: Vec<(i64, &'static str, gpui::Hsla, String, &'static str, String)> =
            Vec::new();
        if let Some(runs) = &self.runs {
            for run in &runs.runs {
                let (glyph, tint, what) = match run.status {
                    TodayRunStatus::Complete => (icons::CHECK, theme.text_muted, "Finished"),
                    TodayRunStatus::Aborted => {
                        (icons::LUCIDE_CIRCLE_ALERT, theme.danger, "Stopped")
                    }
                    TodayRunStatus::Running | TodayRunStatus::Unknown => continue,
                };
                let at = run.started_at_ms + run.duration_ms.unwrap_or(0);
                items.push((
                    at,
                    glyph,
                    tint,
                    run.chat_id.clone(),
                    what,
                    title(&run.chat_id, run.title.as_ref()),
                ));
            }
            for wait in &runs.waits {
                let Some(at) = wait.ended_at_ms else {
                    continue;
                };
                items.push((
                    at,
                    icons::LUCIDE_MESSAGE_CIRCLE_QUESTION,
                    theme.accent,
                    wait.chat_id.clone(),
                    "You answered",
                    title(&wait.chat_id, None),
                ));
            }
        }
        items.sort_by(|a, b| b.0.cmp(&a.0));
        let rows: Vec<AnyElement> = items
            .into_iter()
            .enumerate()
            .map(|(ix, (at, glyph, tint, chat_id, what, title))| {
                task_row(theme, ("inbox-history", ix))
                    .items_center()
                    .child(
                        div()
                            .size(px(24.0))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(7.0))
                            .bg(tint.opacity(0.12))
                            .child(icon(glyph).size(px(14.0)).text_color(tint)),
                    )
                    .child(row_text(theme, title, what.to_owned()))
                    .child(
                        div()
                            .flex_none()
                            .text_size(ui_rems(12.5))
                            .text_color(theme.text_muted)
                            .child(SharedString::from(crate::cloud::ago(at, now))),
                    )
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(InboxEvent::OpenChat(chat_id.clone()))
                    }))
                    .into_any_element()
            })
            .collect();
        let count = rows.len();
        tile(theme, "inbox-history")
            .px(px(8.0))
            .py(px(12.0))
            .flex()
            .flex_col()
            .child(group_heading(theme, "Today", count, None))
            .when(rows.is_empty(), |el| {
                el.child(
                    div()
                        .px(px(12.0))
                        .py(px(8.0))
                        .text_size(ui_rems(13.0))
                        .text_color(theme.text_muted)
                        .child(if self.runs.is_none() {
                            "Loading today’s history…"
                        } else {
                            "Nothing finished or answered yet today."
                        }),
                )
            })
            .children(rows)
            .into_any_element()
    }

    fn render_tasks(
        &self,
        theme: &Theme,
        now: DateTime<Local>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let day_start = work_day_start(now).with_timezone(&Utc);
        let today = work_day(now);
        let today_key = today.format("%Y-%m-%d").to_string();
        let selected = self.card.read(cx).task_id().map(str::to_owned);
        let state = self.state.read(cx);
        let tasks = today_tasks(&state.tasks, &state.chats, today, day_start);
        let open = tasks
            .iter()
            .filter(|task| task.completed_at.is_none())
            .count();
        let rows: Vec<AnyElement> = tasks
            .iter()
            .map(|task| {
                let done = task.completed_at.is_some();
                let overdue = is_overdue(task, &state.chats, &today_key);
                let meta = if done {
                    None
                } else if let Some(chat) = crate::tasks::linked_chat(&state.chats, task) {
                    Some(format!("In {}", chat_title(chat)))
                } else if overdue {
                    Some("Overdue".to_owned())
                } else {
                    task.due_on.as_ref().map(|_| "Today".to_owned())
                };
                let task_id = task.id.clone();
                let accent = theme.accent;
                let check = div()
                    .id(SharedString::from(format!("inbox-task-check-{}", task.id)))
                    .role(gpui::Role::CheckBox)
                    .aria_label(if done {
                        "Mark as not done"
                    } else {
                        "Mark as done"
                    })
                    .size(px(CHECK_SIZE))
                    .flex_none()
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .map(|el| {
                        if done {
                            el.bg(accent).child(
                                icon(icons::CHECK)
                                    .size(px(12.0))
                                    .text_color(theme.on_accent),
                            )
                        } else {
                            el.border_1()
                                .border_color(theme.text_muted)
                                .hover(move |s| s.border_color(accent).bg(accent.opacity(0.12)))
                        }
                    })
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.stop_propagation();
                        cx.emit(InboxEvent::SetTaskCompleted {
                            task_id: task_id.clone(),
                            completed: !done,
                        })
                    }));
                let open_id = task.id.clone();
                let orchestrated = crate::tasks::has_lead(&state.chats, task);
                let is_selected = selected.as_deref() == Some(task.id.as_str());
                div()
                    .id(SharedString::from(format!("inbox-task-{}", task.id)))
                    .debug_selector(|| format!("inbox-task-{}", task.id))
                    .role(gpui::Role::ListItem)
                    .min_h(px(42.0))
                    .px(px(12.0))
                    .rounded(px(12.0))
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .cursor_pointer()
                    .when(is_selected, |el| el.bg(theme.wash(0.08)))
                    .when(!is_selected, |el| el.hover(|s| s.bg(theme.wash(0.06))))
                    .when(done, |el| el.opacity(0.8))
                    .child(check)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_baseline()
                            .gap(px(10.0))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(ui_rems(13.5))
                                    .text_color(if done { theme.text_muted } else { theme.text })
                                    .when(done, |el| el.line_through())
                                    .child(SharedString::from(task.title.clone())),
                            )
                            .children(meta.map(|meta| {
                                div()
                                    .flex_none()
                                    .max_w(relative(0.5))
                                    .truncate()
                                    .text_size(ui_rems(12.5))
                                    .text_color(if overdue {
                                        theme.danger
                                    } else {
                                        theme.text_muted
                                    })
                                    .child(SharedString::from(meta))
                            })),
                    )
                    .on_click(cx.listener(move |page, _: &ClickEvent, window, cx| {
                        if orchestrated {
                            cx.emit(InboxEvent::OpenTaskCenter(open_id.clone()));
                        } else {
                            page.open_task(open_id.clone(), window, cx)
                        }
                    }))
                    .into_any_element()
            })
            .collect();
        let add = div()
            .id("inbox-add-task")
            .debug_selector(|| "inbox-add-task".into())
            .min_h(px(42.0))
            .px(px(12.0))
            .flex()
            .items_center()
            .gap(px(12.0))
            .capture_action(cx.listener(|page, _: &MentionTab, window, cx| {
                if page.submit_add(Some(window), cx) {
                    cx.stop_propagation();
                }
            }))
            .child(crate::tasks::dashed_circle(theme))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(ui_rems(13.5))
                    .child(self.add_input.clone()),
            );
        let all_tasks = div()
            .id("inbox-all-tasks")
            .debug_selector(|| "inbox-all-tasks".into())
            .role(gpui::Role::Link)
            .cursor_pointer()
            .text_size(ui_rems(12.5))
            .text_color(theme.text_muted)
            .hover(|s| s.text_color(theme.text))
            .child("All tasks")
            .on_click(cx.listener(|_, _, _, cx| cx.emit(InboxEvent::OpenTasks)))
            .into_any_element();
        tile(theme, "inbox-tasks")
            .px(px(8.0))
            .py(px(12.0))
            .flex()
            .flex_col()
            .child(group_heading(theme, "Today’s tasks", open, Some(all_tasks)))
            .child(div().flex().flex_col().gap(px(2.0)).children(rows))
            .child(add)
            .into_any_element()
    }

    fn render_day(
        &self,
        theme: &Theme,
        now: DateTime<Local>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let runs: &[TodayRun] = self.runs.as_ref().map_or(&[], |runs| &runs.runs);
        let slots = day_slots(runs, now);
        let (open, done, overdue) = {
            let state = self.state.read(cx);
            let today_key = work_day(now).format("%Y-%m-%d").to_string();
            let tasks = today_tasks(
                &state.tasks,
                &state.chats,
                work_day(now),
                work_day_start(now).with_timezone(&Utc),
            );
            let open: Vec<_> = tasks.iter().filter(|t| t.completed_at.is_none()).collect();
            let overdue = open
                .iter()
                .filter(|task| is_overdue(task, &state.chats, &today_key))
                .count();
            (open.len(), tasks.len() - open.len(), overdue)
        };
        let sentences = digest(runs, &slots, open, done, overdue);
        let heading = div()
            .flex()
            .items_baseline()
            .justify_between()
            .gap(px(12.0))
            .text_size(ui_rems(12.5))
            .text_color(theme.text_muted)
            .child("Your day so far")
            .children(
                zeron_proto::today::updated_label(self.runs_updated, self.runs_failed)
                    .map(|label| div().opacity(0.8).child(SharedString::from(label))),
            );
        let body = if sentences.is_empty() {
            let copy = if self.runs_failed && self.runs.is_none() {
                "Today’s runs did not load."
            } else {
                "Nothing ran yet today. This fills in as your agents work."
            };
            div()
                .text_size(ui_rems(16.0))
                .text_color(theme.text_muted)
                .child(copy)
        } else {
            let mut text = String::new();
            let mut highlights = Vec::new();
            for (ix, parts) in sentences.into_iter().enumerate() {
                if ix > 0 {
                    text.push(' ');
                }
                for part in parts {
                    let (run, color) = match part {
                        Part::Text(run) => (run, None),
                        Part::Strong(run) => (run, Some(theme.text)),
                        Part::Bad(run) => (run, Some(theme.danger)),
                    };
                    let start = text.len();
                    text.push_str(&run);
                    if let Some(color) = color {
                        highlights.push((
                            start..text.len(),
                            gpui::HighlightStyle {
                                color: Some(color),
                                font_weight: Some(gpui::FontWeight::MEDIUM),
                                ..Default::default()
                            },
                        ));
                    }
                }
            }
            div()
                .debug_selector(|| "inbox-digest".into())
                .text_size(ui_rems(20.0))
                .line_height(ui_rems(32.0))
                .text_color(theme.text_muted)
                .child(gpui::StyledText::new(text).with_highlights(highlights))
        };
        let max = slots
            .iter()
            .flat_map(|slot| slot.hours.iter().map(|(count, _)| *count))
            .max()
            .unwrap_or(0)
            .max(1);
        let columns: Vec<AnyElement> = slots
            .iter()
            .map(|slot| {
                let bars = slot.hours.iter().map(|&(count, future)| {
                    let fill = if future || count == 0 {
                        theme.wash(0.08)
                    } else if slot.now {
                        theme.text
                    } else {
                        theme.text_muted.opacity(0.45)
                    };
                    div()
                        .flex_1()
                        .max_w(px(8.0))
                        .min_h(px(3.0))
                        .h(relative(count as f32 / max as f32))
                        .rounded(px(2.0))
                        .bg(fill)
                });
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .justify_between()
                            .gap(px(8.0))
                            .text_size(ui_rems(12.5))
                            .text_color(theme.text_muted)
                            .child(div().flex().child(slot.label).when(slot.now, |el| {
                                el.child(div().text_color(theme.text).child(", now"))
                            }))
                            .child(
                                div()
                                    .flex()
                                    .gap(px(4.0))
                                    .child(
                                        div()
                                            .when(slot.now, |el| {
                                                el.font_weight(gpui::FontWeight::MEDIUM)
                                                    .text_color(theme.text)
                                            })
                                            .child(SharedString::from(slot.prompts.to_string())),
                                    )
                                    .child(if slot.prompts == 1 {
                                        "prompt"
                                    } else {
                                        "prompts"
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .h(px(28.0))
                            .flex()
                            .items_end()
                            .justify_between()
                            .gap(px(3.0))
                            .children(bars),
                    )
                    .into_any_element()
            })
            .collect();
        tile(theme, "inbox-day")
            .px(px(28.0))
            .py(px(24.0))
            .flex()
            .flex_col()
            .gap(px(20.0))
            .child(heading)
            .child(body)
            .when(!columns.is_empty(), |el| {
                el.child(
                    div()
                        .pt(px(16.0))
                        .border_t_1()
                        .border_color(theme.border)
                        .flex()
                        .gap(px(28.0))
                        .children(columns),
                )
            })
            .into_any_element()
    }
}

impl Render for InboxPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let first = !self.visible();
        self.last_render = Some(Instant::now());
        if first {
            self.refresh(cx);
        }
        let theme = Theme::of(cx).clone();
        let now = Local::now();
        let header = div()
            .flex()
            .flex_wrap()
            .items_baseline()
            .gap_x(px(12.0))
            .gap_y(px(4.0))
            .child(
                div()
                    .text_size(ui_rems(30.0))
                    .line_height(ui_rems(36.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child("Inbox"),
            )
            .child(
                div()
                    .debug_selector(|| "inbox-greeting".into())
                    .text_size(ui_rems(15.0))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(format!(
                        "History: what your agents asked, finished and failed. Live items wait in the {} tray.",
                        self.tray_combo
                    ))),
            );
        let banner = self.render_banner(&theme, cx);
        let history = self.render_history(&theme, cx);
        let tasks_card = self.render_tasks(&theme, now, cx);
        let day_card = self.render_day(&theme, now, cx);
        let column = div()
            .w_full()
            .max_w(px(PAGE_MAX_WIDTH))
            .mx_auto()
            .px(px(40.0))
            .pt(px(Theme::TITLEBAR_HEIGHT + 28.0))
            .pb(px(48.0))
            .flex()
            .flex_col()
            .gap(px(28.0))
            .child(header)
            .children(banner)
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_start()
                    .gap(px(16.0))
                    .child(
                        div()
                            .flex_grow(1.0)
                            .flex_basis(px(300.0))
                            .flex()
                            .flex_col()
                            .gap(px(16.0))
                            .child(history)
                            .child(tasks_card),
                    )
                    .child(
                        div()
                            .flex_grow(2.0)
                            .flex_basis(px(600.0))
                            .min_w(px(360.0))
                            .flex()
                            .flex_col()
                            .gap(px(16.0))
                            .child(day_card),
                    ),
            );
        let frame = {
            let card = self.card.read(cx);
            card.frame(crate::motion::now())
                .filter(|frame| card.is_open() || frame.panel > 0.0)
        };
        let card = frame.map(|frame| {
            div()
                .absolute()
                .top(px(Theme::TITLEBAR_HEIGHT + 16.0))
                .right(px(16.0 - frame.shift))
                .bottom(px(16.0))
                .w(px(CARD_WIDTH))
                .flex()
                .flex_col()
                .opacity(frame.panel)
                .occlude()
                .child(crate::frost::frosted(
                    24.0,
                    crate::frost::MENU_BLUR * frame.panel,
                    self.card.clone(),
                ))
        });
        div()
            .id("inbox-root")
            .relative()
            .size_full()
            .on_key_down(cx.listener(|page, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && page.card.read(cx).is_open() {
                    cx.stop_propagation();
                    page.card
                        .update(cx, |card, cx| card.close(Some(window), cx));
                }
            }))
            .child(widgets::scroll_faded(
                "inbox-page-scroll",
                div()
                    .id("inbox-page")
                    .debug_selector(|| "inbox-page".into())
                    .size_full()
                    .overflow_y_scroll()
                    .child(column),
            ))
            .children(card)
    }
}
