use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, NaiveDate, TimeZone, Timelike, Utc};
use gpui::{
    AnyElement, ClickEvent, Context, Entity, EventEmitter, IntoElement, Render, SharedString,
    Subscription, Task, Window, div, prelude::*, px, relative,
};
use zeron_proto::change_request_assessment::{AttentionReason, Facts};
use zeron_proto::{
    AgentAccountsSnapshot, AgentUsageWindow, ChangeRequestListItem, Chat, ChatIndicator, HarnessId,
    SessionStatus, TodayRun, TodayRunStatus, TodayRuns, TodayRunsRequest,
};
use zeron_rpc::methods;

use crate::composer::{Composer, ComposerInput, ComposerInputEvent, MentionTab};
use crate::home::HomePrs;
use crate::icons::{self, icon};
use crate::provider_instances::InstanceIcon;
use crate::settings::accounts;
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
const PRS_EVERY: Duration = Duration::from_secs(120);
const RUNS_EVERY: Duration = Duration::from_secs(60);
const DAY_STARTS_AT_HOUR: u32 = 4;
const SLOTS: [(&str, u32, u32); 3] = [
    ("Morning", DAY_STARTS_AT_HOUR, 12),
    ("Afternoon", 12, 18),
    ("Evening", 18, 24 + DAY_STARTS_AT_HOUR),
];
const ADD_PLACEHOLDER: &str = "Add a task for today, or press Tab to hand it to an agent";

pub enum InboxEvent {
    OpenChat(String),
    OpenPullRequests,
    OpenTasks,
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
    accounts: Option<AgentAccountsSnapshot>,
    asks: HashMap<String, bool>,
    prs: Option<HomePrs>,
    prs_fetched: Option<Instant>,
    prs_task: Option<Task<()>>,
    projects_seen: Vec<String>,
    runs: Option<Result<TodayRuns, String>>,
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
    pub fn new(
        state: Entity<AppState>,
        composer: Entity<Composer>,
        cx: &mut Context<Self>,
    ) -> Self {
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
        let mut page = Self {
            state,
            add_input,
            card,
            open_first_card: std::env::var("ZERON_OPEN_DIALOG").as_deref() == Ok("task-card"),
            accounts: None,
            asks,
            prs: None,
            prs_fetched: None,
            prs_task: None,
            projects_seen: Vec::new(),
            runs: None,
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
            due_on: Local::now().date_naive().format("%Y-%m-%d").to_string(),
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
                now.date_naive(),
                local_midnight(now).with_timezone(&Utc),
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
            since_ms: work_day_start(Local::now()).timestamp_millis(),
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
                if result.is_ok() {
                    this.runs_updated = Some(Local::now());
                }
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

fn local_at(day: NaiveDate, hour: u32) -> Option<DateTime<Local>> {
    let date = day.checked_add_days(chrono::Days::new(u64::from(hour / 24)))?;
    let at = date.and_hms_opt(hour % 24, 0, 0)?;
    Local.from_local_datetime(&at).earliest()
}

fn work_day_start(now: DateTime<Local>) -> DateTime<Local> {
    local_at(work_day(now), DAY_STARTS_AT_HOUR).unwrap_or(now)
}

pub(crate) fn greeting(now: DateTime<Local>) -> &'static str {
    match now.hour() {
        h if (DAY_STARTS_AT_HOUR..12).contains(&h) => "Good morning",
        12..=17 => "Good afternoon",
        _ => "Good evening",
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

pub(crate) fn header_line(now: DateTime<Local>, needs: usize, open_tasks: usize) -> String {
    let date = work_day(now).format("%A, %B %-d");
    let tasks = match open_tasks {
        0 => String::new(),
        n => format!(" {} to do.", plural(n, "task", "tasks")),
    };
    let rest = match (needs, open_tasks) {
        (0, 0) => " You’re all caught up.".to_owned(),
        (0, _) => tasks,
        (1, _) => format!(" 1 thing needs you.{tasks}"),
        (n, _) => format!(" {n} things need you.{tasks}"),
    };
    format!("{}. {date}.{rest}", greeting(now))
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

pub(crate) fn today_tasks<'a>(
    tasks: &'a [zeron_proto::Task],
    chats: &[zeron_proto::Chat],
    today: NaiveDate,
    day_start: DateTime<Utc>,
) -> Vec<&'a zeron_proto::Task> {
    let today = today.format("%Y-%m-%d").to_string();
    let mut open: Vec<&zeron_proto::Task> = tasks
        .iter()
        .filter(|task| !task.deleted && task.completed_at.is_none())
        .filter(|task| {
            crate::tasks::linked_chat(chats, task).is_some()
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

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Slot {
    label: &'static str,
    now: bool,
    prompts: usize,
    work_ms: i64,
    hours: Vec<(usize, bool)>,
}

pub(crate) fn day_slots(runs: &[TodayRun], now: DateTime<Local>) -> Vec<Slot> {
    let day = work_day(now);
    let now_ms = now.timestamp_millis();
    SLOTS
        .iter()
        .filter_map(|&(label, from, to)| {
            let start = local_at(day, from)?.timestamp_millis();
            let end = local_at(day, to)?.timestamp_millis();
            if start > now_ms {
                return None;
            }
            let inside: Vec<&TodayRun> = runs
                .iter()
                .filter(|run| (start..end).contains(&run.started_at_ms))
                .collect();
            let hours = (from..to)
                .filter_map(|hour| {
                    let at = local_at(day, hour)?.timestamp_millis();
                    let count = inside
                        .iter()
                        .filter(|run| (at..at + 3_600_000).contains(&run.started_at_ms))
                        .count();
                    Some((count, at > now_ms))
                })
                .collect();
            let slot = Slot {
                label,
                now: now_ms < end,
                prompts: inside.len(),
                work_ms: inside.iter().map(|run| run_work_ms(run, now_ms)).sum(),
                hours,
            };
            (slot.now || slot.prompts > 0).then_some(slot)
        })
        .collect()
}

fn run_work_ms(run: &TodayRun, now_ms: i64) -> i64 {
    match (run.status, run.duration_ms) {
        (_, Some(ms)) => ms.max(0),
        (TodayRunStatus::Running, None) => (now_ms - run.started_at_ms).max(0),
        _ => 0,
    }
}

fn clock_duration(ms: i64) -> String {
    let seconds = (ms / 1000).max(1);
    match seconds {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s => match (s % 3600) / 60 {
            0 => format!("{}h", s / 3600),
            m => format!("{}h {m}m", s / 3600),
        },
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Part {
    Text(String),
    Strong(String),
    Bad(String),
}

pub(crate) fn digest(
    runs: &[TodayRun],
    slots: &[Slot],
    open: usize,
    done: usize,
    overdue: usize,
) -> Vec<Vec<Part>> {
    let text = |s: &str| Part::Text(s.to_owned());
    let mut sentences = Vec::new();
    if !runs.is_empty() {
        let chats: HashSet<&str> = runs.iter().map(|run| run.chat_id.as_str()).collect();
        sentences.push(vec![
            text("You sent "),
            Part::Strong(plural(runs.len(), "prompt", "prompts")),
            text(" across "),
            Part::Strong(plural(chats.len(), "chat", "chats")),
            text("."),
        ]);
    }
    let work: i64 = slots.iter().map(|slot| slot.work_ms).sum();
    if work >= 1_000 {
        let mut parts = vec![
            text("Your agents ran for "),
            Part::Strong(clock_duration(work)),
        ];
        if slots.len() > 1
            && let Some(busiest) = slots.iter().max_by_key(|slot| slot.work_ms)
        {
            parts.push(Part::Text(format!(
                ", mostly in the {}",
                busiest.label.to_lowercase()
            )));
        }
        parts.push(text("."));
        sentences.push(parts);
    }
    if open > 0 || done > 0 {
        let mut parts = if open == 0 {
            vec![
                text(if done == 1 {
                    "You finished your "
                } else {
                    "You finished all "
                }),
                Part::Strong(plural(done, "task", "tasks")),
                text(" for today"),
            ]
        } else if done > 0 {
            vec![
                text("You finished "),
                Part::Strong(plural(done, "task", "tasks")),
                text(" today, "),
                Part::Strong(format!("{open} to go")),
            ]
        } else {
            vec![
                text("You have "),
                Part::Strong(plural(open, "task", "tasks")),
                text(" to do today"),
            ]
        };
        if overdue > 0 && open > 0 {
            parts.push(text(", "));
            parts.push(Part::Bad(format!("{overdue} overdue")));
        }
        parts.push(text("."));
        sentences.push(parts);
    }
    sentences
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

fn brand_mark(theme: &Theme, mark: Option<InstanceIcon>, size: f32) -> AnyElement {
    mark.unwrap_or(InstanceIcon::Brand(icons::CHAT_ROUND_LINE, None))
        .svg(theme.text_muted)
        .size(px(size))
        .flex_none()
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
    ) -> Option<AnyElement> {
        let total = chats.len() + prs.len();
        if total == 0 {
            return None;
        }
        let (_, _, attention) = crate::shell::chat_status_tone(ChatIndicator::AwaitingInput, theme);
        let state = self.state.read(cx);
        let mut rows: Vec<AnyElement> = chats
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
                task_row(
                    theme,
                    SharedString::from(format!("inbox-needs-{}", chat.id)),
                )
                .child(
                    div().mt(px(2.0)).child(
                        icon(icons::LUCIDE_CIRCLE_ALERT)
                            .size(px(16.0))
                            .text_color(attention),
                    ),
                )
                .child(row_text(theme, chat_title(chat), meta))
                .child(div().mt(px(3.0)).child(brand_mark(
                    theme,
                    crate::pickers::chat_mark(chat, cx),
                    14.0,
                )))
                .on_click(
                    cx.listener(move |_, _, _, cx| cx.emit(InboxEvent::OpenChat(chat_id.clone()))),
                )
                .into_any_element()
            })
            .collect();
        if !prs.is_empty() {
            let reviews = prs
                .iter()
                .all(|(_, why)| why.contains(&AttentionReason::ReviewRequested));
            let count = plural(prs.len(), "pull request", "pull requests");
            let title = if reviews {
                format!("{count} to review")
            } else if prs.len() == 1 {
                format!("{count} needs you")
            } else {
                format!("{count} need you")
            };
            let meta = if reviews {
                "GitHub".to_owned()
            } else {
                format!("GitHub, {}", attention_breakdown(prs))
            };
            rows.push(
                task_row(theme, "inbox-needs-prs")
                    .child(
                        div().mt(px(2.0)).child(
                            icon(icons::LUCIDE_GIT_PULL_REQUEST)
                                .size(px(16.0))
                                .text_color(attention),
                        ),
                    )
                    .child(row_text(theme, title, meta))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(InboxEvent::OpenPullRequests)))
                    .into_any_element(),
            );
        }
        Some(
            tile(theme, "inbox-needs-you")
                .px(px(8.0))
                .py(px(12.0))
                .flex()
                .flex_col()
                .child(group_heading(theme, "Needs you", total, None))
                .children(rows)
                .into_any_element(),
        )
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
                let meta = if done {
                    None
                } else if let Some(chat) = crate::tasks::linked_chat(&state.chats, task) {
                    Some(format!("In {}", chat_title(chat)))
                } else {
                    match task.due_on.as_deref() {
                        Some(due) if due < today_key.as_str() => Some("Overdue".to_owned()),
                        Some(_) => Some("Today".to_owned()),
                        None => None,
                    }
                };
                let overdue = meta.as_deref() == Some("Overdue");
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
                        page.open_task(open_id.clone(), window, cx)
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
        let runs: &[TodayRun] = match &self.runs {
            Some(Ok(runs)) => &runs.runs,
            _ => &[],
        };
        let slots = day_slots(runs, now);
        let (open, done, overdue) = {
            let state = self.state.read(cx);
            let today_key = now.date_naive().format("%Y-%m-%d").to_string();
            let tasks = today_tasks(
                &state.tasks,
                &state.chats,
                now.date_naive(),
                local_midnight(now).with_timezone(&Utc),
            );
            let open: Vec<_> = tasks.iter().filter(|t| t.completed_at.is_none()).collect();
            let overdue = open
                .iter()
                .filter(|t| {
                    t.due_on
                        .as_deref()
                        .is_some_and(|due| due < today_key.as_str())
                })
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
            .children(self.runs_updated.map(|at| {
                div().opacity(0.8).child(SharedString::from(format!(
                    "Updated {}",
                    at.format("%I:%M %p")
                )))
            }));
        let body = if sentences.is_empty() {
            let copy = match &self.runs {
                Some(Err(_)) => "Today’s runs did not load.",
                _ => "Nothing ran yet today. This fills in as your agents work.",
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
            tile(theme, "inbox-usage")
                .min_h(px(112.0))
                .px(px(16.0))
                .py(px(14.0))
                .flex()
                .flex_col()
                .justify_between()
                .gap(px(12.0))
                .child(
                    div()
                        .truncate()
                        .text_size(ui_rems(12.5))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(lead)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(brand_mark(
                            theme,
                            Some(InstanceIcon::harness(harness)),
                            18.0,
                        ))
                        .child(
                            div()
                                .text_size(ui_rems(20.0))
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
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .flex_none()
                                .w(px(56.0))
                                .h(px(4.0))
                                .rounded_full()
                                .bg(theme.wash(0.08))
                                .child(
                                    div()
                                        .h_full()
                                        .w(relative(remaining))
                                        .rounded_full()
                                        .bg(fill),
                                ),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(ui_rems(12.5))
                                .text_color(theme.text_muted)
                                .child(SharedString::from(detail)),
                        ),
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
        let open_tasks = {
            let state = self.state.read(cx);
            today_tasks(
                &state.tasks,
                &state.chats,
                now.date_naive(),
                local_midnight(now).with_timezone(&Utc),
            )
            .iter()
            .filter(|task| task.completed_at.is_none())
            .count()
        };
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
                    .child(SharedString::from(header_line(now, needs, open_tasks))),
            );
        let needs_card = self.render_needs_you(&theme, &chats, &prs, cx);
        let tasks_card = self.render_tasks(&theme, now, cx);
        let day_card = self.render_day(&theme, now, cx);
        let usage_card = self.render_usage(&theme);
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
                            .children(needs_card)
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
                            .child(day_card)
                            .children(usage_card),
                    ),
            );
        let card = self.card.read(cx).is_open().then(|| {
            div()
                .absolute()
                .top(px(Theme::TITLEBAR_HEIGHT + 16.0))
                .right(px(16.0))
                .bottom(px(16.0))
                .w(px(CARD_WIDTH))
                .flex()
                .flex_col()
                .occlude()
                .child(crate::frost::frosted(
                    24.0,
                    crate::frost::MENU_BLUR,
                    self.card.clone(),
                ))
        });
        div()
            .id("inbox-root")
            .relative()
            .size_full()
            .on_key_down(cx.listener(|page, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" && page.card.read(cx).is_open() {
                    cx.stop_propagation();
                    page.card.update(cx, |card, cx| card.close(cx));
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
            header_line(local(2026, 10, 17, 2, 30), 0, 0),
            "Good evening. Friday, October 16. You’re all caught up."
        );
        assert_eq!(
            header_line(local(2026, 10, 17, 9, 0), 1, 0),
            "Good morning. Saturday, October 17. 1 thing needs you."
        );
        assert_eq!(
            header_line(local(2026, 10, 17, 9, 0), 5, 1),
            "Good morning. Saturday, October 17. 5 things need you. 1 task to do."
        );
        assert_eq!(
            header_line(local(2026, 10, 17, 13, 0), 0, 3),
            "Good afternoon. Saturday, October 17. 3 tasks to do."
        );
    }

    fn run(chat: &str, at: DateTime<Local>, ms: Option<i64>) -> TodayRun {
        TodayRun {
            chat_id: chat.into(),
            title: None,
            harness: None,
            started_at_ms: at.timestamp_millis(),
            duration_ms: ms,
            status: TodayRunStatus::Complete,
        }
    }

    #[test]
    fn the_day_counts_prompts_per_slot_from_four_am() {
        let now = local(2026, 10, 17, 14, 30);
        let runs = vec![
            run("a", local(2026, 10, 17, 3, 59), Some(60_000)),
            run("a", local(2026, 10, 17, 9, 10), Some(120_000)),
            run("b", local(2026, 10, 17, 9, 40), Some(60_000)),
            run("b", local(2026, 10, 17, 13, 5), Some(30_000)),
        ];
        let slots = day_slots(&runs, now);
        let summary: Vec<(&str, bool, usize)> = slots
            .iter()
            .map(|slot| (slot.label, slot.now, slot.prompts))
            .collect();
        assert_eq!(summary, [("Morning", false, 2), ("Afternoon", true, 1)]);
        assert_eq!(slots[0].hours.len(), 8);
        assert_eq!(slots[0].hours[5], (2, false));
        assert_eq!(slots[1].hours[1], (1, false));
        assert_eq!(slots[1].hours[3], (0, true));
        let sentences = digest(&runs[1..], &slots, 1, 0, 0);
        assert_eq!(
            sentences.last().unwrap(),
            &vec![
                Part::Text("You have ".into()),
                Part::Strong("1 task".into()),
                Part::Text(" to do today".into()),
                Part::Text(".".into()),
            ]
        );
        assert_eq!(
            sentences[1],
            vec![
                Part::Text("Your agents ran for ".into()),
                Part::Strong("3m".into()),
                Part::Text(", mostly in the morning".into()),
                Part::Text(".".into()),
            ]
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
            task("e-linked-dead", None, Some("deleted-chat"), None),
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
        let chats: Vec<zeron_proto::Chat> = vec![
            serde_json::from_value(serde_json::json!({
                "id": "chat", "deviceId": "local", "archived": false, "createdAt": day_start,
            }))
            .unwrap(),
        ];
        let ids: Vec<&str> = today_tasks(&tasks, &chats, today, day_start)
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
