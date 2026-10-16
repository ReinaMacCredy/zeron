use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, NaiveDate, TimeZone, Timelike, Utc};
use gpui::{
    AnyElement, Context, Entity, EventEmitter, Hsla, IntoElement, Render, SharedString,
    Subscription, Task, Window, div, prelude::*, px, relative,
};
use zeron_proto::change_request_assessment::{AttentionReason, Facts};
use zeron_proto::{
    AgentAccountsSnapshot, AgentUsageWindow, ChangeRequestListItem, Chat, ChatIndicator, HarnessId,
    SessionStatus, TodayRun, TodayRunStatus, TodayRuns, TodayRunsRequest,
};
use zeron_rpc::methods;

use crate::composer::{Composer, ComposerInput, ComposerInputEvent};
use crate::home::HomePrs;
use crate::icons::{self, icon};
use crate::settings::accounts;
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;

const PAGE_MAX_WIDTH: f32 = 1080.0;
const CARD_RADIUS: f32 = 14.0;
const LEFT_COLUMN: f32 = 400.0;
const ROW_HEIGHT: f32 = 34.0;
const TICK: Duration = Duration::from_secs(30);
const PRS_EVERY: Duration = Duration::from_secs(120);
const RUNS_EVERY: Duration = Duration::from_secs(60);
const RUNS_SHOWN: usize = 8;
const DAY_STARTS_AT_HOUR: u32 = 4;

pub enum InboxEvent {
    OpenChat(String),
    OpenPullRequests,
    CreateTask { title: String, due_on: String },
    SetTaskCompleted { task_id: String, completed: bool },
}

pub struct InboxPage {
    state: Entity<AppState>,
    add_input: Entity<ComposerInput>,
    accounts: Option<AgentAccountsSnapshot>,
    asks: HashMap<String, bool>,
    prs: Option<HomePrs>,
    prs_fetched: Option<Instant>,
    prs_task: Option<Task<()>>,
    projects_seen: Vec<String>,
    runs: Option<Result<TodayRuns, String>>,
    runs_fetched: Option<Instant>,
    runs_task: Option<Task<()>>,
    working_seen: usize,
    last_render: Option<Instant>,
    _ticker: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<InboxEvent> for InboxPage {}

impl InboxPage {
    pub fn new(
        state: Entity<AppState>,
        composer: Entity<Composer>,
        cx: &mut Context<Self>,
    ) -> Self {
        let add_input = cx.new(|cx| {
            ComposerInput::new("Add a task for today…", cx)
                .with_single_line()
                .with_text_metrics(13.0, 18.0)
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
        let subscriptions = vec![
            cx.subscribe(&add_input, |_, input, event, cx| {
                if matches!(event, ComposerInputEvent::Submitted) {
                    let title = input.read(cx).text().trim().to_owned();
                    if !title.is_empty() {
                        cx.emit(InboxEvent::CreateTask {
                            title,
                            due_on: Local::now().date_naive().format("%Y-%m-%d").to_string(),
                        });
                        input.update(cx, |input, cx| input.set_text("", cx));
                    }
                }
            }),
            cx.observe(&composer, |this, composer, cx| {
                let asks = composer.read(cx).ask_kinds();
                if asks != this.asks {
                    this.asks = asks;
                    cx.notify();
                }
            }),
            cx.observe(&state, |this, _, cx| this.on_state_changed(cx)),
        ];
        let asks = composer.read(cx).ask_kinds();
        Self {
            state,
            add_input,
            accounts: None,
            asks,
            prs: None,
            prs_fetched: None,
            prs_task: None,
            projects_seen: Vec::new(),
            runs: None,
            runs_fetched: None,
            runs_task: None,
            working_seen: 0,
            last_render: None,
            _ticker: ticker,
            _subscriptions: subscriptions,
        }
    }

    pub fn set_accounts(
        &mut self,
        accounts: Option<AgentAccountsSnapshot>,
        cx: &mut Context<Self>,
    ) {
        if self.accounts != accounts {
            self.accounts = accounts;
            cx.notify();
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
        let now = Instant::now();
        let projects: Vec<String> = crate::home::local_git_project_paths(self.state.read(cx))
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();
        if projects != self.projects_seen {
            self.projects_seen = projects;
            self.prs_fetched = None;
        }
        let due = |fetched: Option<Instant>, every| fetched.is_none_or(|at| now - at >= every);
        if self.prs_task.is_none() && due(self.prs_fetched, PRS_EVERY) {
            self.fetch_prs(cx);
        }
        if self.runs_task.is_none() && due(self.runs_fetched, RUNS_EVERY) {
            self.fetch_runs(cx);
        }
    }

    fn fetch_prs(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let projects = crate::home::local_git_project_paths(self.state.read(cx));
        self.prs_task = Some(cx.spawn(async move |this, cx| {
            let prs = crate::home::load_home_prs(&engine, projects).await;
            this.update(cx, |this, cx| {
                this.prs_task = None;
                this.prs_fetched = Some(Instant::now());
                this.prs = Some(prs);
                cx.notify();
            })
            .ok();
        }));
    }

    fn fetch_runs(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let request = TodayRunsRequest {
            since_ms: local_midnight(Local::now()).timestamp_millis(),
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
                this.runs = Some(result);
                cx.notify();
            })
            .ok();
        }));
    }
}

pub(crate) fn work_day(now: DateTime<Local>) -> NaiveDate {
    (now - chrono::Duration::hours(i64::from(DAY_STARTS_AT_HOUR))).date_naive()
}

pub(crate) fn greeting(now: DateTime<Local>) -> &'static str {
    match now.hour() {
        h if (DAY_STARTS_AT_HOUR..12).contains(&h) => "Good morning",
        12..=17 => "Good afternoon",
        _ => "Good evening",
    }
}

pub(crate) fn header_line(now: DateTime<Local>, needs: usize) -> String {
    let date = work_day(now).format("%A, %B %-d");
    let needs = match needs {
        0 => "Nothing needs you.".to_owned(),
        1 => "1 thing needs you.".to_owned(),
        n => format!("{n} things need you."),
    };
    format!("{}. {date}. {needs}", greeting(now))
}

fn local_midnight(now: DateTime<Local>) -> DateTime<Local> {
    now.date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|midnight| Local.from_local_datetime(&midnight).earliest())
        .unwrap_or(now)
}

pub(crate) fn resets_in(resets_at: DateTime<Utc>, now: DateTime<Utc>) -> Option<String> {
    let seconds = (resets_at - now).num_seconds();
    if seconds <= 0 {
        return None;
    }
    let minutes = (seconds + 59) / 60;
    let (hours, days) = (minutes / 60, minutes / (60 * 24));
    Some(match () {
        _ if minutes < 60 => format!("{minutes}m"),
        _ if hours < 24 => match minutes % 60 {
            0 => format!("{hours}h"),
            m => format!("{hours}h {m}m"),
        },
        _ => match hours % 24 {
            0 => format!("{days}d"),
            h => format!("{days}d {h}h"),
        },
    })
}

pub(crate) fn awaiting_chats(state: &AppState, now: DateTime<Utc>) -> Vec<&Chat> {
    state
        .overview_chats(now)
        .into_iter()
        .filter(|(indicator, _)| *indicator == ChatIndicator::AwaitingInput)
        .map(|(_, chat)| chat)
        .collect()
}

pub(crate) fn attention_prs(prs: &HomePrs) -> Vec<(&ChangeRequestListItem, Vec<AttentionReason>)> {
    let mut seen = HashSet::new();
    prs.mine
        .iter()
        .chain(&prs.review)
        .filter(|item| seen.insert(item.url.as_str()))
        .filter_map(|item| {
            let attention = Facts::from_item(item).assess().attention;
            (!attention.is_empty()).then_some((item, attention))
        })
        .collect()
}

fn attention_breakdown(prs: &[(&ChangeRequestListItem, Vec<AttentionReason>)]) -> String {
    [
        (AttentionReason::ReviewRequested, "to review"),
        (AttentionReason::FeedbackOnAuthored, "need changes"),
        (AttentionReason::FailingCi, "failing CI"),
    ]
    .into_iter()
    .filter_map(|(reason, label)| {
        let count = prs.iter().filter(|(_, why)| why.contains(&reason)).count();
        (count > 0).then(|| format!("{count} {label}"))
    })
    .collect::<Vec<_>>()
    .join(" · ")
}

pub(crate) fn today_tasks(
    tasks: &[zeron_proto::Task],
    today: NaiveDate,
    day_start: DateTime<Utc>,
) -> Vec<&zeron_proto::Task> {
    let today = today.format("%Y-%m-%d").to_string();
    let mut open: Vec<&zeron_proto::Task> = tasks
        .iter()
        .filter(|task| !task.deleted && task.completed_at.is_none())
        .filter(|task| {
            task.chat_id.is_some()
                || task
                    .due_on
                    .as_deref()
                    .is_some_and(|due| due <= today.as_str())
        })
        .collect();
    open.sort_by(|a, b| a.order_key.cmp(&b.order_key));
    let mut done: Vec<&zeron_proto::Task> = tasks
        .iter()
        .filter(|task| !task.deleted && task.completed_at.is_some_and(|at| at >= day_start))
        .collect();
    done.sort_by_key(|task| std::cmp::Reverse(task.completed_at));
    open.extend(done);
    open
}

pub(crate) fn tightest_window(
    snapshot: &AgentAccountsSnapshot,
) -> Option<(HarnessId, &AgentUsageWindow)> {
    accounts::sidebar_accounts(snapshot)
        .into_iter()
        .flat_map(|account| {
            account
                .usage_windows
                .iter()
                .map(move |window| (account.harness, window))
        })
        .max_by(|a, b| a.1.used_fraction.total_cmp(&b.1.used_fraction))
}

fn run_duration(ms: i64) -> String {
    let seconds = (ms / 1000).max(1);
    match seconds {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m {}s", s / 60, s % 60),
        s => format!("{}h {}m", s / 3600, (s % 3600) / 60),
    }
}

fn card_fill(theme: &Theme) -> Hsla {
    if theme.is_frost() {
        theme.wash(0.045)
    } else {
        theme.surface_card
    }
}

fn card(
    theme: &Theme,
    id: &'static str,
    title: &str,
    count: Option<usize>,
    accessory: Option<AnyElement>,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .w_full()
        .flex()
        .flex_col()
        .rounded(px(CARD_RADIUS))
        .border_1()
        .border_color(theme.border)
        .bg(card_fill(theme))
        .p(px(6.0))
        .child(
            div()
                .h(px(32.0))
                .px(px(10.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .text_size(crate::typography::ui_rems(13.0))
                .child(
                    div()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(theme.text)
                        .child(SharedString::from(title.to_owned())),
                )
                .when_some(count, |el, count| {
                    el.child(
                        div()
                            .text_color(theme.text_faint)
                            .child(SharedString::from(count.to_string())),
                    )
                })
                .when_some(accessory, |el, accessory| {
                    el.child(div().flex_1()).child(accessory)
                }),
        )
}

fn row_base(theme: &Theme, id: impl Into<gpui::ElementId>) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .min_h(px(ROW_HEIGHT))
        .px(px(10.0))
        .py(px(6.0))
        .rounded(px(9.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .text_size(crate::typography::ui_rems(13.0))
        .hover(|style| style.bg(theme.element_hover))
}

fn row_text(theme: &Theme, title: String, meta: Option<String>) -> gpui::Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .child(
            div()
                .truncate()
                .text_color(theme.text)
                .child(SharedString::from(title)),
        )
        .when_some(meta, |el, meta| {
            el.child(
                div()
                    .truncate()
                    .text_size(crate::typography::ui_rems(11.5))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(meta)),
            )
        })
}

fn brand_mark(theme: &Theme, harness: Option<HarnessId>) -> AnyElement {
    let (mark, tint) = harness
        .map(crate::pickers::harness_brand_icon)
        .unwrap_or((icons::CHAT_ROUND_LINE, None));
    icon(mark)
        .size(px(14.0))
        .flex_none()
        .text_color(tint.unwrap_or(theme.text_muted))
        .into_any_element()
}

fn empty_copy(theme: &Theme, copy: &str) -> AnyElement {
    div()
        .px(px(10.0))
        .pt(px(2.0))
        .pb(px(10.0))
        .text_size(crate::typography::ui_rems(12.5))
        .text_color(theme.text_faint)
        .child(SharedString::from(copy.to_owned()))
        .into_any_element()
}

fn chat_title(chat: &Chat) -> String {
    crate::transcript::single_line(&crate::home::chat_title(chat))
}

impl InboxPage {
    fn render_needs_you(
        &self,
        theme: &Theme,
        chats: &[Chat],
        prs: &[(&ChangeRequestListItem, Vec<AttentionReason>)],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = self.state.read(cx);
        let total = chats.len() + prs.len();
        let mut body: Vec<AnyElement> = chats
            .iter()
            .map(|chat| {
                let project = chat
                    .space_id
                    .as_deref()
                    .and_then(|id| state.space_row(id))
                    .map(|space| state.representative_space(space).display_name().to_owned());
                let ask = match self.asks.get(&chat.id) {
                    Some(true) => "approve a command",
                    Some(false) => "has a question",
                    None => "waiting for you",
                };
                let meta = match project {
                    Some(project) => format!("{project}, {ask}"),
                    None => ask.to_owned(),
                };
                let chat_id = chat.id.clone();
                row_base(
                    theme,
                    SharedString::from(format!("inbox-needs-{}", chat.id)),
                )
                .cursor_pointer()
                .child(brand_mark(theme, chat.config.as_ref().map(|c| c.harness)))
                .child(row_text(theme, chat_title(chat), Some(meta)))
                .child(
                    icon(icons::ALT_ARROW_RIGHT)
                        .size(px(12.0))
                        .flex_none()
                        .text_color(theme.text_faint),
                )
                .on_click(
                    cx.listener(move |_, _, _, cx| cx.emit(InboxEvent::OpenChat(chat_id.clone()))),
                )
                .into_any_element()
            })
            .collect();
        if !prs.is_empty() {
            let title = match prs.len() {
                1 => "1 pull request needs you".to_owned(),
                n => format!("{n} pull requests need you"),
            };
            body.push(
                row_base(theme, "inbox-needs-prs")
                    .cursor_pointer()
                    .child(
                        icon(icons::LUCIDE_GIT_PULL_REQUEST)
                            .size(px(14.0))
                            .flex_none()
                            .text_color(theme.text_muted),
                    )
                    .child(row_text(theme, title, Some(attention_breakdown(prs))))
                    .child(
                        icon(icons::ALT_ARROW_RIGHT)
                            .size(px(12.0))
                            .flex_none()
                            .text_color(theme.text_faint),
                    )
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(InboxEvent::OpenPullRequests)))
                    .into_any_element(),
            );
        }
        if body.is_empty() {
            body.push(empty_copy(theme, "Nothing needs you right now."));
        }
        card(theme, "inbox-needs-you", "Needs you", Some(total), None)
            .children(body)
            .into_any_element()
    }

    fn render_tasks(
        &self,
        theme: &Theme,
        now: DateTime<Local>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let day_start = local_midnight(now).with_timezone(&Utc);
        let today = now.date_naive();
        let today_key = today.format("%Y-%m-%d").to_string();
        let state = self.state.read(cx);
        let tasks = today_tasks(&state.tasks, today, day_start);
        let open = tasks
            .iter()
            .filter(|task| task.completed_at.is_none())
            .count();
        let rows: Vec<AnyElement> = tasks
            .iter()
            .map(|task| {
                let done = task.completed_at.is_some();
                let meta = if done {
                    None
                } else if let Some(chat) = task
                    .chat_id
                    .as_deref()
                    .and_then(|id| state.chats.iter().find(|chat| chat.id == id))
                {
                    Some(format!("In {}", chat_title(chat)))
                } else {
                    match task.due_on.as_deref() {
                        Some(due) if due < today_key.as_str() => Some("Overdue".to_owned()),
                        Some(_) => Some("Today".to_owned()),
                        None => None,
                    }
                };
                let task_id = task.id.clone();
                let check = div()
                    .id(SharedString::from(format!("inbox-task-check-{}", task.id)))
                    .size(px(16.0))
                    .flex_none()
                    .rounded_full()
                    .border_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .map(|el| {
                        if done {
                            el.border_color(theme.accent).bg(theme.accent).child(
                                icon(icons::CHECK)
                                    .size(px(10.0))
                                    .text_color(theme.on_accent),
                            )
                        } else {
                            el.border_color(theme.text_faint)
                                .hover(|style| style.border_color(theme.text_muted))
                        }
                    })
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.stop_propagation();
                        cx.emit(InboxEvent::SetTaskCompleted {
                            task_id: task_id.clone(),
                            completed: !done,
                        })
                    }));
                row_base(theme, SharedString::from(format!("inbox-task-{}", task.id)))
                    .when(done, |el| el.opacity(0.55))
                    .child(check)
                    .child(row_text(theme, task.title.clone(), meta))
                    .into_any_element()
            })
            .collect();
        let add = div()
            .mx(px(4.0))
            .mt(px(2.0))
            .h(px(ROW_HEIGHT))
            .px(px(6.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .child(
                icon(icons::PLUS)
                    .size(px(14.0))
                    .flex_none()
                    .text_color(theme.text_faint),
            )
            .child(div().flex_1().min_w_0().child(self.add_input.clone()));
        card(theme, "inbox-tasks", "Today's tasks", Some(open), None)
            .children(rows)
            .child(add)
            .into_any_element()
    }

    fn render_runs(&self, theme: &Theme, awaiting: &[Chat], cx: &mut Context<Self>) -> AnyElement {
        let runs: &[TodayRun] = match &self.runs {
            Some(Ok(runs)) => &runs.runs,
            _ => &[],
        };
        let mut body: Vec<AnyElement> = runs
            .iter()
            .take(RUNS_SHOWN)
            .enumerate()
            .map(|(index, run)| {
                let started = chrono::DateTime::from_timestamp_millis(run.started_at_ms)
                    .map(|at| at.with_timezone(&Local).format("%-I:%M %p").to_string())
                    .unwrap_or_default();
                let outcome = match (run.status, run.duration_ms) {
                    (TodayRunStatus::Running, _)
                        if awaiting.iter().any(|chat| chat.id == run.chat_id) =>
                    {
                        "Waiting for you".to_owned()
                    }
                    (TodayRunStatus::Running, _) => "Working…".to_owned(),
                    (TodayRunStatus::Aborted, Some(ms)) => {
                        format!("Stopped after {}", run_duration(ms))
                    }
                    (TodayRunStatus::Aborted, None) => "Stopped".to_owned(),
                    (_, Some(ms)) => format!("Worked for {}", run_duration(ms)),
                    (_, None) => "Finished".to_owned(),
                };
                let title = run
                    .title
                    .as_deref()
                    .map(crate::transcript::single_line)
                    .filter(|title| !title.trim().is_empty())
                    .unwrap_or_else(|| "New session".to_owned());
                let chat_id = run.chat_id.clone();
                row_base(theme, ("inbox-run", index))
                    .cursor_pointer()
                    .child(brand_mark(theme, run.harness))
                    .child(row_text(theme, title, Some(outcome)))
                    .child(
                        div()
                            .flex_none()
                            .text_size(crate::typography::ui_rems(11.5))
                            .text_color(theme.text_faint)
                            .child(SharedString::from(started)),
                    )
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(InboxEvent::OpenChat(chat_id.clone()))
                    }))
                    .into_any_element()
            })
            .collect();
        if runs.len() > RUNS_SHOWN {
            body.push(empty_copy(
                theme,
                &format!("{} more earlier today", runs.len() - RUNS_SHOWN),
            ));
        }
        if body.is_empty() {
            let copy = match &self.runs {
                Some(Err(_)) => "Today's runs did not load.",
                _ => "Nothing ran yet today. This fills in as your agents work.",
            };
            body.push(empty_copy(theme, copy));
        }
        let count = (!runs.is_empty()).then_some(runs.len());
        card(theme, "inbox-today", "Today", count, None)
            .children(body)
            .into_any_element()
    }

    fn render_usage(&self, theme: &Theme) -> Option<AnyElement> {
        let (harness, window) = tightest_window(self.accounts.as_ref()?)?;
        let used = window.used_fraction.clamp(0.0, 1.0);
        let remaining = 1.0 - used;
        let fill = accounts::usage_color(accounts::usage_level(used), theme);
        let detail = window
            .resets_at
            .and_then(|at| resets_in(at, Utc::now()))
            .map(|left| format!("Resets in {left}"))
            .unwrap_or_else(|| "Your tightest limit".to_owned());
        let lead = format!(
            "Left on {}, {}",
            harness.display_name(),
            window.label.to_lowercase()
        );
        Some(
            div()
                .id("inbox-usage")
                .w_full()
                .rounded(px(CARD_RADIUS))
                .border_1()
                .border_color(theme.border)
                .bg(card_fill(theme))
                .px(px(16.0))
                .py(px(14.0))
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .text_size(crate::typography::ui_rems(13.0))
                        .text_color(theme.text_muted)
                        .child(brand_mark(theme, Some(harness)))
                        .child(div().flex_1().truncate().child(SharedString::from(lead)))
                        .child(
                            div()
                                .text_size(crate::typography::ui_rems(20.0))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(theme.text)
                                .child(SharedString::from(format!(
                                    "{}%",
                                    (remaining * 100.0).round() as u32
                                ))),
                        ),
                )
                .child(
                    div()
                        .h(px(6.0))
                        .w_full()
                        .rounded_full()
                        .bg(theme.wash(0.08))
                        .child(
                            div()
                                .h_full()
                                .w(relative(remaining))
                                .rounded_full()
                                .bg(fill.opacity(0.85)),
                        ),
                )
                .child(
                    div()
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.text_faint)
                        .child(SharedString::from(detail)),
                )
                .into_any_element(),
        )
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
        let chats: Vec<Chat> = awaiting_chats(self.state.read(cx), now.with_timezone(&Utc))
            .into_iter()
            .cloned()
            .collect();
        let prs = self.prs.as_ref().map(attention_prs).unwrap_or_default();
        let needs = chats.len() + prs.len();
        let header = div()
            .px(px(4.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .text_size(crate::typography::ui_rems(28.0))
                    .line_height(crate::typography::ui_rems(34.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.text)
                    .child("Inbox"),
            )
            .child(
                div()
                    .text_size(crate::typography::ui_rems(14.0))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(header_line(now, needs))),
            );
        let needs_card = self.render_needs_you(&theme, &chats, &prs, cx);
        let tasks_card = self.render_tasks(&theme, now, cx);
        let runs_card = self.render_runs(&theme, &chats, cx);
        let usage_card = self.render_usage(&theme);
        let column = widgets::page_column()
            .max_w(px(PAGE_MAX_WIDTH))
            .gap(px(24.0))
            .child(header)
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
                            .flex_1()
                            .min_w(px(300.0))
                            .max_w(px(LEFT_COLUMN))
                            .flex()
                            .flex_col()
                            .gap(px(16.0))
                            .child(needs_card)
                            .child(tasks_card),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(320.0))
                            .flex()
                            .flex_col()
                            .gap(px(16.0))
                            .child(runs_card)
                            .children(usage_card),
                    ),
            );
        widgets::scroll_faded(
            "inbox-page-scroll",
            div()
                .id("inbox-page")
                .debug_selector(|| "inbox-page".into())
                .size_full()
                .overflow_y_scroll()
                .child(column),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    #[test]
    fn greeting_follows_a_day_that_starts_at_four() {
        assert_eq!(greeting(local(2026, 10, 16, 3, 59)), "Good evening");
        assert_eq!(greeting(local(2026, 10, 16, 4, 0)), "Good morning");
        assert_eq!(greeting(local(2026, 10, 16, 11, 59)), "Good morning");
        assert_eq!(greeting(local(2026, 10, 16, 12, 0)), "Good afternoon");
        assert_eq!(greeting(local(2026, 10, 16, 17, 59)), "Good afternoon");
        assert_eq!(greeting(local(2026, 10, 16, 18, 0)), "Good evening");
        assert_eq!(
            header_line(local(2026, 10, 17, 2, 30), 0),
            "Good evening. Friday, October 16. Nothing needs you."
        );
        assert_eq!(
            header_line(local(2026, 10, 17, 9, 0), 1),
            "Good morning. Saturday, October 17. 1 thing needs you."
        );
        assert_eq!(
            header_line(local(2026, 10, 17, 13, 0), 3),
            "Good afternoon. Saturday, October 17. 3 things need you."
        );
    }

    #[test]
    fn resets_in_reads_minutes_hours_then_days() {
        let now = Utc.with_ymd_and_hms(2026, 10, 17, 12, 0, 0).unwrap();
        let later = |secs: i64| now + chrono::Duration::seconds(secs);
        assert_eq!(resets_in(now, now), None);
        assert_eq!(resets_in(later(-60), now), None);
        assert_eq!(resets_in(later(30), now).as_deref(), Some("1m"));
        assert_eq!(resets_in(later(59 * 60), now).as_deref(), Some("59m"));
        assert_eq!(resets_in(later(3600), now).as_deref(), Some("1h"));
        assert_eq!(
            resets_in(later(2 * 3600 + 14 * 60), now).as_deref(),
            Some("2h 14m")
        );
        assert_eq!(resets_in(later(24 * 3600), now).as_deref(), Some("1d"));
        assert_eq!(
            resets_in(later(3 * 86_400 + 4 * 3600), now).as_deref(),
            Some("3d 4h")
        );
    }

    fn task(
        id: &str,
        due: Option<&str>,
        chat: Option<&str>,
        completed: Option<i64>,
    ) -> zeron_proto::Task {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "title": id,
            "dueOn": due,
            "chatId": chat,
            "orderKey": id,
            "completedAt": completed.map(|s| Utc.timestamp_opt(s, 0).unwrap()),
        }))
        .unwrap()
    }

    #[test]
    fn today_tasks_keep_due_linked_and_done_today() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 17).unwrap();
        let day_start = Utc.with_ymd_and_hms(2026, 10, 17, 0, 0, 0).unwrap();
        let mut deleted = task("deleted", Some("2026-10-17"), None, None);
        deleted.deleted = true;
        let start = day_start.timestamp();
        let tasks = vec![
            task("a-overdue", Some("2026-10-16"), None, None),
            task("b-today", Some("2026-10-17"), None, None),
            task("c-tomorrow", Some("2026-10-18"), None, None),
            task("d-backlog", None, None, None),
            task("e-linked", None, Some("chat"), None),
            task(
                "f-done-yesterday",
                Some("2026-10-16"),
                None,
                Some(start - 60),
            ),
            task("g-done-early", None, None, Some(start + 60)),
            task("h-done-late", Some("2026-10-20"), None, Some(start + 600)),
            deleted,
        ];
        let ids: Vec<&str> = today_tasks(&tasks, today, day_start)
            .into_iter()
            .map(|task| task.id.as_str())
            .collect();
        assert_eq!(
            ids,
            [
                "a-overdue",
                "b-today",
                "e-linked",
                "h-done-late",
                "g-done-early"
            ]
        );
    }

    fn pr(url: &str, authored: bool, requested: bool, ci_failed: bool) -> ChangeRequestListItem {
        serde_json::from_value(serde_json::json!({
            "provider": "github",
            "repository": "reina/zeron",
            "headRefOid": "0123456789abcdef0123456789abcdef01234567",
            "ci": { "state": if ci_failed { "failed" } else { "passed" } },
            "viewerDidAuthor": authored,
            "viewerReviewRequested": requested,
            "number": 1,
            "title": url,
            "url": url,
            "state": "open",
            "isDraft": false,
            "reviewDecision": "reviewRequired",
            "additions": 1,
            "deletions": 1,
            "mergeability": "mergeable",
            "createdAt": "2026-10-17T00:00:00Z",
            "updatedAt": "2026-10-17T00:00:00Z",
        }))
        .unwrap()
    }

    #[test]
    fn attention_prs_dedupe_by_url_and_skip_quiet_ones() {
        let prs = HomePrs {
            mine: vec![pr("u/1", true, false, true), pr("u/2", true, false, false)],
            review: vec![pr("u/1", true, false, true), pr("u/3", false, true, false)],
            ..Default::default()
        };
        let flagged = attention_prs(&prs);
        let urls: Vec<&str> = flagged.iter().map(|(item, _)| item.url.as_str()).collect();
        assert_eq!(urls, ["u/1", "u/3"]);
        assert_eq!(attention_breakdown(&flagged), "1 to review · 1 failing CI");
    }

    #[test]
    fn tightest_window_is_the_most_used_active_window() {
        let snapshot: AgentAccountsSnapshot = serde_json::from_value(serde_json::json!({
            "accounts": [
                { "harness": "codex", "active": true, "switchable": true, "id": "c", "usageWindows": [
                    { "label": "Session", "usedFraction": 0.4, "resetsAt": null },
                    { "label": "Week", "usedFraction": 0.7, "resetsAt": null }
                ]},
                { "harness": "claude-code", "active": true, "switchable": true, "id": "a", "usageWindows": [
                    { "label": "Session", "usedFraction": 0.5, "resetsAt": null }
                ]},
                { "harness": "claude-code", "active": false, "switchable": true, "id": "b", "usageWindows": [
                    { "label": "Session", "usedFraction": 0.99, "resetsAt": null }
                ]}
            ],
            "warnings": []
        }))
        .unwrap();
        let (harness, window) = tightest_window(&snapshot).unwrap();
        assert_eq!((harness, window.label.as_str()), (HarnessId::Codex, "Week"));
        assert!(tightest_window(&AgentAccountsSnapshot::default()).is_none());
    }
}
