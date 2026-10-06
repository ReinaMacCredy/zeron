use std::collections::HashMap;
use std::time::{Duration, Instant};

use chrono::{DateTime, Datelike, Utc};
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, Hsla, InteractiveElement, IntoElement,
    ParentElement, Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Task,
    Window, div, prelude::FluentBuilder, px,
};
use zeron_proto::{
    ChangeRequestCheckStatus, ChangeRequestChecksState, ChangeRequestDetail, ChangeRequestListItem,
    ChangeRequestListing, ChangeRequestReviewDecision, Chat, ChatIndicator,
    GetChangeRequestRequest, GitActivity, GitActivityRequest, ListChangeRequestsRequest,
    ListeningServices, ListeningServicesRequest, SystemStats,
};
use zeron_rpc::methods;

use crate::icons::{self, icon};
use crate::popover;
use crate::settings::HomeWidgets as Toggles;
use crate::state::AppState;
use crate::theme::Theme;

const GRID_MAX_WIDTH: f32 = 880.0;
const COLUMN_GAP: f32 = 10.0;
const CARD_RADIUS: f32 = 10.0;
const ROW_HEIGHT: f32 = 28.0;
const STATS_EVERY: Duration = Duration::from_secs(3);
const SERVICES_EVERY: Duration = Duration::from_secs(5);
const SLOW_EVERY: Duration = Duration::from_secs(120);
const VISIBLE_GRACE: Duration = Duration::from_secs(2);
const FAILING_CHECK_LOOKUPS: usize = 3;

pub enum HomeEvent {
    OpenChat(String),
    OpenPullRequest(String),
    OpenSettings,
}

#[derive(Default)]
struct Source<T> {
    value: Option<Result<T, String>>,
    fetched: Option<Instant>,
    task: Option<Task<()>>,
}

impl<T> Source<T> {
    fn due(&self, every: Duration, now: Instant) -> bool {
        self.task.is_none()
            && self
                .fetched
                .is_none_or(|at| now.duration_since(at) >= every)
    }

    fn ready(&self) -> Option<&T> {
        self.value.as_ref().and_then(|value| value.as_ref().ok())
    }
}

pub struct HomeView {
    state: Entity<AppState>,
    git: Source<GitActivity>,
    git_filter: Option<String>,
    git_menu: popover::Popup<()>,
    git_hover: Option<usize>,
    customize: popover::Popup<()>,
    collapse_since: Option<Instant>,
    grid_height: std::rc::Rc<std::cell::Cell<f32>>,
    services: Source<ListeningServices>,
    services_all_ports: bool,
    stats: Source<SystemStats>,
    prs: Source<ChangeRequestListing>,
    failing_checks: HashMap<String, String>,
    failing_lookups: Vec<Task<()>>,
    last_render: Option<Instant>,
    projects_seen: Vec<String>,
    scroll: ScrollHandle,
    _ticker: Task<()>,
}

impl EventEmitter<HomeEvent> for HomeView {}

impl HomeView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let ticker = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this.update(cx, |this, cx| this.tick(cx)).is_err() {
                    break;
                }
            }
        });
        Self {
            state,
            git: Source::default(),
            git_filter: None,
            git_menu: popover::Popup::default(),
            git_hover: None,
            customize: popover::Popup::default(),
            collapse_since: None,
            grid_height: Default::default(),
            services: Source::default(),
            services_all_ports: false,
            stats: Source::default(),
            prs: Source::default(),
            failing_checks: HashMap::new(),
            failing_lookups: Vec::new(),
            last_render: None,
            projects_seen: Vec::new(),
            scroll: ScrollHandle::new(),
            _ticker: ticker,
        }
    }

    fn visible(&self) -> bool {
        self.last_render
            .is_some_and(|at| at.elapsed() < VISIBLE_GRACE)
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        if !self.visible() {
            return;
        }
        self.refresh(cx);
        if !running_agents(self.state.read(cx), Utc::now()).is_empty() {
            cx.notify();
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let toggles = crate::settings::home_widgets(cx);
        let now = Instant::now();
        let projects = local_git_projects(self.state.read(cx));
        if projects != self.projects_seen {
            self.projects_seen = projects;
            self.git.fetched = None;
            self.prs.fetched = None;
        }
        if toggles.services_all_ports != self.services_all_ports {
            self.services_all_ports = toggles.services_all_ports;
            self.services = Source::default();
        }
        if toggles.this_mac && self.stats.due(STATS_EVERY, now) {
            self.fetch_stats(cx);
        }
        if toggles.running_services && self.services.due(SERVICES_EVERY, now) {
            self.fetch_services(cx);
        }
        if toggles.git_activity && self.git.due(SLOW_EVERY, now) {
            self.fetch_git(cx);
        }
        if (toggles.pull_requests || toggles.ci_failures) && self.prs.due(SLOW_EVERY, now) {
            self.fetch_prs(cx);
        }
    }

    fn call<T: serde::de::DeserializeOwned + 'static>(
        &mut self,
        method: &'static str,
        params: serde_json::Value,
        slot: fn(&mut Self) -> &mut Source<T>,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let task = cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(method, params)
                .await
                .map_err(|error| error.to_string())
                .and_then(|value| serde_json::from_value::<T>(value).map_err(|e| e.to_string()));
            this.update(cx, |this, cx| {
                let source = slot(this);
                source.task = None;
                source.fetched = Some(Instant::now());
                source.value = Some(result);
                this.after_fetch(method, cx);
                cx.notify();
            })
            .ok();
        });
        slot(self).task = Some(task);
    }

    fn after_fetch(&mut self, method: &'static str, cx: &mut Context<Self>) {
        if method == methods::LIST_CHANGE_REQUESTS {
            self.lookup_failing_checks(cx);
        }
    }

    fn fetch_stats(&mut self, cx: &mut Context<Self>) {
        self.call(
            methods::GET_SYSTEM_STATS,
            serde_json::json!({}),
            |this| &mut this.stats,
            cx,
        );
    }

    fn fetch_services(&mut self, cx: &mut Context<Self>) {
        let params = serde_json::to_value(ListeningServicesRequest {
            all_ports: self.services_all_ports,
        })
        .unwrap_or_default();
        self.call(
            methods::LIST_LISTENING_SERVICES,
            params,
            |this| &mut this.services,
            cx,
        );
    }

    fn fetch_git(&mut self, cx: &mut Context<Self>) {
        let params = serde_json::to_value(GitActivityRequest {
            space_id: self.git_filter.clone(),
            days: Some(14),
        })
        .unwrap_or_default();
        self.call(methods::GET_GIT_ACTIVITY, params, |this| &mut this.git, cx);
    }

    fn fetch_prs(&mut self, cx: &mut Context<Self>) {
        let params = serde_json::to_value(ListChangeRequestsRequest::default()).unwrap_or_default();
        self.call(
            methods::LIST_CHANGE_REQUESTS,
            params,
            |this| &mut this.prs,
            cx,
        );
    }

    fn lookup_failing_checks(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let failing: Vec<ChangeRequestListItem> = self
            .prs
            .ready()
            .map(|listing| {
                failing_prs(listing)
                    .into_iter()
                    .filter(|item| !self.failing_checks.contains_key(&item.url))
                    .take(FAILING_CHECK_LOOKUPS)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        self.failing_lookups = failing
            .into_iter()
            .map(|item| {
                let engine = engine.clone();
                let params = serde_json::to_value(GetChangeRequestRequest {
                    host: item.host.clone(),
                    repository: item.repository.clone(),
                    number: item.number,
                })
                .unwrap_or_default();
                cx.spawn(async move |this, cx| {
                    let Ok(value) = engine
                        .client()
                        .call(methods::GET_CHANGE_REQUEST, params)
                        .await
                    else {
                        return;
                    };
                    let Ok(detail) = serde_json::from_value::<ChangeRequestDetail>(value) else {
                        return;
                    };
                    let Some(check) = detail.checks.iter().find(|check| {
                        matches!(
                            check.status,
                            ChangeRequestCheckStatus::Failure | ChangeRequestCheckStatus::Cancelled
                        )
                    }) else {
                        return;
                    };
                    let name = check.name.clone();
                    this.update(cx, |this, cx| {
                        this.failing_checks.insert(item.url.clone(), name);
                        cx.notify();
                    })
                    .ok();
                })
            })
            .collect();
    }

    fn select_git_project(&mut self, space_id: Option<String>, cx: &mut Context<Self>) {
        self.close_git_menu(cx);
        if self.git_filter == space_id {
            return;
        }
        self.git_filter = space_id;
        self.git.task = None;
        self.fetch_git(cx);
    }

    fn close_customize(&mut self, cx: &mut Context<Self>) {
        if self.customize.begin_close() {
            popover::reap_popup(cx, |this: &mut Self| &mut this.customize);
            cx.notify();
        }
    }

    fn toggle_collapsed(&mut self, cx: &mut Context<Self>) {
        self.collapse_since = Some(Instant::now());
        crate::settings::update_home_widgets(cx, |widgets| widgets.collapsed = !widgets.collapsed);
        cx.notify();
    }

    fn summary(&self, cx: &App) -> String {
        let mut parts = Vec::new();
        let running = running_agents(self.state.read(cx), Utc::now()).len();
        if running > 0 {
            parts.push(format!("{running} running"));
        }
        if let Some(listing) = self.prs.ready() {
            let review = split_prs(listing).1.len();
            if review > 0 {
                parts.push(format!("{review} to review"));
            }
            let failing = failing_prs(listing).len();
            if failing > 0 {
                parts.push(format!("{failing} failing"));
            }
        }
        if let Some(activity) = self.git.ready() {
            parts.push(format!("{} commits", activity.commits));
        }
        if parts.is_empty() {
            "Home".into()
        } else {
            parts.join(" · ")
        }
    }

    fn grid_reveal(&self, collapsed: bool, window: &mut Window, cx: &App) -> f32 {
        let target = if collapsed { 0.0 } else { 1.0 };
        let Some(since) = self.collapse_since else {
            return target;
        };
        if crate::motion::reduced_motion(cx) {
            return target;
        }
        let total =
            crate::motion::SIDECAR_LAYOUT.total().as_secs_f32() * crate::motion::speed_scale();
        let raw = since.elapsed().as_secs_f32() / total.max(f32::EPSILON);
        if raw < 1.0 {
            window.request_animation_frame();
        }
        let progress = crate::motion::SIDECAR_LAYOUT.progress(raw);
        if collapsed { 1.0 - progress } else { progress }
    }

    fn render_customize_menu(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let theme = &theme.for_popup();
        let mut toggles = crate::settings::home_widgets(cx);
        let all_ports = toggles.services_all_ports;
        let rows = crate::settings::home::WIDGETS
            .into_iter()
            .map(|(id, title, _, field)| {
                let on = *field(&mut toggles);
                let row_id: SharedString = format!("{id}-quick").into();
                popover::menu_row(theme, false, row_id.clone())
                    .id(row_id.clone())
                    .debug_selector(move || row_id.to_string())
                    .on_click(cx.listener(move |_, _, _, cx| {
                        crate::settings::update_home_widgets(cx, |widgets| {
                            let value = field(widgets);
                            *value = !*value;
                        });
                        cx.notify();
                    }))
                    .child(div().flex_1().min_w_0().child(title))
                    .child(crate::settings::widgets::toggle_switch(
                        theme,
                        on,
                        format!("{id}-quick"),
                    ))
            });
        let rows: Vec<_> = rows.collect();
        let mode = |id: &'static str, label: &'static str, value: bool| {
            crate::settings::permission_guide::choice_button(theme, id, label, all_ports == value)
                .debug_selector(move || id.to_string())
                .on_click(cx.listener(move |_, _, _, cx| {
                    crate::settings::update_home_widgets(cx, |widgets| {
                        widgets.services_all_ports = value
                    });
                    cx.notify();
                }))
        };
        popover::popover_card(theme)
            .w(px(260.0))
            .p(px(4.0))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_customize(cx)))
            .child(popover::menu_heading(theme, "Widgets"))
            .children(rows)
            .child(popover::menu_separator())
            .child(popover::menu_heading(theme, "Running services"))
            .child(
                div()
                    .px(px(8.0))
                    .pb(px(6.0))
                    .flex()
                    .gap(px(6.0))
                    .child(mode("home-quick-by-zeron", "By Zeron", false))
                    .child(mode("home-quick-all-ports", "All ports", true)),
            )
            .child(popover::menu_separator())
            .child(
                popover::menu_row(theme, false, "home-quick-settings")
                    .id("home-quick-settings")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.close_customize(cx);
                        cx.emit(HomeEvent::OpenSettings);
                    }))
                    .child(
                        icon(icons::SETTINGS_MINIMALISTIC)
                            .size(px(14.0))
                            .text_color(theme.text_muted),
                    )
                    .child("All Home settings"),
            )
            .into_any_element()
    }

    fn close_git_menu(&mut self, cx: &mut Context<Self>) {
        if self.git_menu.begin_close() {
            popover::reap_popup(cx, |this: &mut Self| &mut this.git_menu);
            cx.notify();
        }
    }
}

pub(crate) fn running_agents(state: &AppState, now: DateTime<Utc>) -> Vec<(&Chat, DateTime<Utc>)> {
    state
        .overview_chats(now)
        .into_iter()
        .filter(|(indicator, _)| *indicator == ChatIndicator::Working)
        .map(|(_, chat)| {
            let started = state
                .session_for(&chat.id)
                .and_then(|session| session.started_at)
                .into_iter()
                .chain(state.pending_send_started(&chat.id, now))
                .max()
                .unwrap_or(now);
            (chat, started)
        })
        .collect()
}

fn local_git_projects(state: &AppState) -> Vec<String> {
    state
        .spaces
        .iter()
        .filter(|space| {
            space.git_detected && state.local_device_id.as_ref() == Some(&space.device_id)
        })
        .map(|space| space.id.clone())
        .collect()
}

pub(crate) fn chat_title(chat: &Chat) -> String {
    chat.title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or("New session")
        .to_owned()
}

fn failing_prs(listing: &ChangeRequestListing) -> Vec<&ChangeRequestListItem> {
    let (mine, _) = split_prs(listing);
    mine.into_iter()
        .filter(|item| item.checks_state == Some(ChangeRequestChecksState::Failing))
        .collect()
}

fn split_prs(
    listing: &ChangeRequestListing,
) -> (Vec<&ChangeRequestListItem>, Vec<&ChangeRequestListItem>) {
    let items: Vec<&ChangeRequestListItem> = listing.items.iter().collect();
    let mut mine = Vec::new();
    let mut review = Vec::new();
    for (group, rows) in crate::pull_requests::group_items(&items, &listing.viewers) {
        match group {
            crate::pull_requests::PullRequestGroup::Authored => mine = rows,
            crate::pull_requests::PullRequestGroup::ReviewRequested => review = rows,
            _ => {}
        }
    }
    (mine, review)
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RecentArea {
    pub label: String,
    pub chat_id: String,
    pub at: DateTime<Utc>,
}

pub(crate) fn recent_areas(state: &AppState, limit: usize) -> Vec<RecentArea> {
    let mut areas: Vec<RecentArea> = Vec::new();
    let mut keys: Vec<(Option<String>, Option<String>)> = Vec::new();
    let mut chats: Vec<&Chat> = state.visible_chats().collect();
    chats.sort_by_key(|chat| std::cmp::Reverse(chat.last_message_at.unwrap_or(chat.created_at)));
    for chat in chats {
        let key = (chat.space_id.clone(), chat.cwd.clone());
        if keys.contains(&key) {
            continue;
        }
        let space = chat
            .space_id
            .as_deref()
            .and_then(|id| state.space_row(id))
            .map(|space| state.representative_space(space));
        let Some(space) = space else {
            continue;
        };
        let area = chat
            .branch
            .clone()
            .filter(|branch| !branch.is_empty())
            .or_else(|| {
                chat.cwd
                    .as_deref()
                    .filter(|cwd| *cwd != space.path)
                    .and_then(|cwd| std::path::Path::new(cwd).file_name())
                    .map(|name| name.to_string_lossy().into_owned())
            });
        keys.push(key);
        let label = match area {
            Some(area) => format!("{} / {area}", space.display_name()),
            None => space.display_name().to_owned(),
        };
        if areas.iter().any(|existing| existing.label == label) {
            continue;
        }
        areas.push(RecentArea {
            label,
            chat_id: chat.id.clone(),
            at: chat.last_message_at.unwrap_or(chat.created_at),
        });
        if areas.len() == limit {
            break;
        }
    }
    areas
}

fn short_age(now: DateTime<Utc>, at: DateTime<Utc>) -> String {
    let minutes = (now - at).num_minutes();
    if minutes < 1 {
        "now".into()
    } else if minutes < 60 {
        format!("{minutes}m")
    } else if minutes < 60 * 24 {
        format!("{}h", minutes / 60)
    } else {
        format!("{}d", minutes / (60 * 24))
    }
}

fn gigabytes(bytes: u64) -> String {
    let gb = bytes as f64 / 1_000_000_000.0;
    if gb >= 1000.0 {
        format!("{:.1} TB", gb / 1000.0)
    } else {
        format!("{gb:.0} GB")
    }
}

fn line_count(count: u64) -> String {
    if count >= 1000 {
        format!("{:.1}k", count as f64 / 1000.0)
    } else {
        count.to_string()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Widget {
    WorkingNow,
    PullRequests,
    GitActivity,
    ThisMac,
    RunningServices,
    RecentAreas,
    CiFailures,
}

impl Widget {
    const ORDER: [Widget; 7] = [
        Widget::WorkingNow,
        Widget::PullRequests,
        Widget::GitActivity,
        Widget::RunningServices,
        Widget::ThisMac,
        Widget::CiFailures,
        Widget::RecentAreas,
    ];

    fn enabled(self, toggles: &Toggles) -> bool {
        match self {
            Widget::WorkingNow => toggles.working_now,
            Widget::PullRequests => toggles.pull_requests,
            Widget::GitActivity => toggles.git_activity,
            Widget::ThisMac => toggles.this_mac,
            Widget::RunningServices => toggles.running_services,
            Widget::RecentAreas => toggles.recent_areas,
            Widget::CiFailures => toggles.ci_failures,
        }
    }
}

fn columns(widgets: &[(Widget, f32)]) -> [Vec<Widget>; 2] {
    let mut columns = [Vec::new(), Vec::new()];
    let mut heights = [0.0_f32; 2];
    for (widget, height) in widgets {
        let column = if heights[1] < heights[0] { 1 } else { 0 };
        columns[column].push(*widget);
        heights[column] += height + COLUMN_GAP;
    }
    columns
}

impl HomeView {
    fn estimated_rows(&self, widget: Widget, cx: &App) -> usize {
        let state = self.state.read(cx);
        match widget {
            Widget::WorkingNow => running_agents(state, Utc::now()).len().clamp(1, 4) * 2,
            Widget::PullRequests => self
                .prs
                .ready()
                .map(|listing| {
                    let (mine, review) = split_prs(listing);
                    mine.len().min(5) + review.len().min(4) + 2
                })
                .unwrap_or(2),
            Widget::GitActivity => 4,
            Widget::ThisMac => 2,
            Widget::RunningServices => self
                .services
                .ready()
                .map(|list| list.services.len().clamp(1, 6))
                .unwrap_or(1),
            Widget::RecentAreas => 4,
            Widget::CiFailures => self
                .prs
                .ready()
                .map(|listing| failing_prs(listing).len().clamp(1, 4))
                .unwrap_or(1),
        }
    }

    fn render_widget(
        &mut self,
        widget: Widget,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match widget {
            Widget::WorkingNow => self.render_working(theme, cx),
            Widget::PullRequests => self.render_prs(theme, cx),
            Widget::GitActivity => self.render_git(theme, cx),
            Widget::ThisMac => self.render_this_mac(theme, cx),
            Widget::RunningServices => self.render_services(theme),
            Widget::RecentAreas => self.render_recent(theme, cx),
            Widget::CiFailures => self.render_ci(theme, cx),
        }
    }

    fn render_working(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let now = Utc::now();
        let rows: Vec<(
            String,
            String,
            Option<String>,
            Option<(&'static str, Option<Hsla>)>,
            DateTime<Utc>,
        )> = {
            let state = self.state.read(cx);
            running_agents(state, now)
                .into_iter()
                .map(|(chat, started)| {
                    let shell =
                        crate::composer::background::shell_tasks(state.session_for(&chat.id))
                            .into_iter()
                            .next()
                            .map(|task| task.label);
                    let detail = shell.or_else(|| zeron_proto::view::chat_location(chat));
                    let mark = chat
                        .config
                        .as_ref()
                        .map(|config| crate::pickers::harness_brand_icon(config.harness));
                    (chat.id.clone(), chat_title(chat), detail, mark, started)
                })
                .collect()
        };
        let count = rows.len();
        let entity = cx.entity_id();
        let body: Vec<AnyElement> = if rows.is_empty() {
            vec![empty_line(theme, "No agents running")]
        } else {
            rows.into_iter()
                .take(4)
                .map(|(chat_id, title, detail, mark, started)| {
                    let elapsed =
                        crate::composer::background::elapsed_label((now - started).num_seconds());
                    let id: SharedString = format!("home-working-{chat_id}").into();
                    div()
                        .id(id.clone())
                        .flex()
                        .flex_col()
                        .px(px(6.0))
                        .mx(px(-6.0))
                        .py(px(4.0))
                        .rounded(px(6.0))
                        .cursor_pointer()
                        .hover(|style| style.bg(theme.wash(0.05)))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(HomeEvent::OpenChat(chat_id.clone()))
                        }))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .child(crate::loaders::mini_glyph_spinner(
                                    format!("{id}-spin"),
                                    2.0,
                                    theme.glyph,
                                    entity,
                                    cx,
                                ))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .whitespace_nowrap()
                                        .text_color(theme.text)
                                        .child(title),
                                )
                                .when_some(mark, |row, (path, tint)| {
                                    row.child(
                                        icon(path)
                                            .size(px(12.0))
                                            .flex_none()
                                            .text_color(tint.unwrap_or(theme.text_muted)),
                                    )
                                })
                                .child(mono(theme, theme.text_muted, elapsed)),
                        )
                        .when_some(detail, |el, detail| {
                            el.child(
                                div()
                                    .pl(px(20.0))
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .font_family(theme.font_mono.clone())
                                    .text_size(crate::typography::ui_rems(11.0))
                                    .text_color(theme.text_faint)
                                    .child(detail),
                            )
                        })
                        .into_any_element()
                })
                .collect()
        };
        card(
            theme,
            icons::SPEEDOMETER,
            "Working now",
            Some(count.to_string()),
            None,
            body,
        )
    }

    fn render_prs(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let body: Vec<AnyElement> = match &self.prs.value {
            None => vec![empty_line(theme, "Loading…")],
            Some(Err(error)) => vec![empty_line(theme, error)],
            Some(Ok(listing)) => {
                let (mine, review) = split_prs(listing);
                let mut body = Vec::new();
                if !review.is_empty() {
                    body.push(sub_heading(
                        theme,
                        theme.warning,
                        format!("Waiting on your review · {}", review.len()),
                    ));
                    for item in review.iter().take(4) {
                        let right = item
                            .author
                            .as_ref()
                            .map(|author| author.login.clone())
                            .unwrap_or_default();
                        body.push(pr_row(theme, item, right.into(), cx));
                    }
                }
                if !mine.is_empty() {
                    body.push(sub_heading(
                        theme,
                        theme.text_faint,
                        format!("Yours · {}", mine.len()),
                    ));
                    for item in mine.iter().take(5) {
                        body.push(pr_row(theme, item, review_label(item).into(), cx));
                    }
                }
                if body.is_empty() {
                    body.push(empty_line(
                        theme,
                        listing
                            .errors
                            .first()
                            .map(|error| crate::pull_requests::failure_message(error.failure))
                            .unwrap_or("Nothing waiting on you"),
                    ));
                }
                body
            }
        };
        let count = self
            .prs
            .ready()
            .map(|listing| split_prs(listing).1.len())
            .filter(|count| *count > 0)
            .map(|count| count.to_string());
        card(
            theme,
            icons::LUCIDE_GIT_PULL_REQUEST,
            "Pull requests",
            count,
            None,
            body,
        )
    }

    fn render_git(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let label = match (&self.git_filter, self.git.ready()) {
            (None, _) => "All projects".to_owned(),
            (Some(id), Some(activity)) => activity
                .projects
                .iter()
                .find(|project| &project.space_id == id)
                .map(|project| project.name.clone())
                .unwrap_or_else(|| "Project".into()),
            (Some(_), None) => "Project".into(),
        };
        let open = self.git_menu.is_open();
        let trigger = div()
            .id("home-git-menu")
            .flex()
            .items_center()
            .gap(px(4.0))
            .px(px(6.0))
            .h(px(20.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(theme.border)
            .text_size(crate::typography::ui_rems(11.0))
            .text_color(theme.text_muted)
            .cursor_pointer()
            .when(open, |el| el.bg(theme.wash(0.06)))
            .hover(|style| style.bg(theme.wash(0.06)))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, _| this.git_menu.note_trigger_press()),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                if this.git_menu.take_press_was_open() {
                    this.close_git_menu(cx);
                } else {
                    this.git_menu.open(());
                    cx.notify();
                }
            }))
            .child(label)
            .child(
                icon(icons::ALT_ARROW_DOWN)
                    .size(px(10.0))
                    .text_color(theme.text_faint),
            );
        let trigger = if self.git_menu.get().is_some() {
            let closing = self.git_menu.closing_since();
            let menu = self.render_git_menu(theme, cx);
            trigger.relative().child(popover::anchored_menu_below(
                "home-git-menu-card",
                menu,
                closing,
            ))
        } else {
            trigger
        };
        let body: Vec<AnyElement> = match &self.git.value {
            None => vec![empty_line(theme, "Loading…")],
            Some(Err(error)) => vec![empty_line(theme, error)],
            Some(Ok(activity)) => {
                let activity = activity.clone();
                vec![git_chart(theme, &activity, self.git_hover, cx)]
            }
        };
        card(
            theme,
            icons::LUCIDE_GIT_COMMIT_HORIZONTAL,
            "Git activity",
            None,
            Some(trigger.into_any_element()),
            body,
        )
    }

    fn render_git_menu(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let theme = &theme.for_popup();
        let projects = self
            .git
            .ready()
            .map(|activity| activity.projects.clone())
            .unwrap_or_default();
        let mut rows = vec![(None, "All projects".to_owned(), None)];
        rows.extend(
            projects
                .into_iter()
                .map(|project| (Some(project.space_id), project.name, Some(project.commits))),
        );
        popover::popover_card(theme)
            .w(px(220.0))
            .p(px(4.0))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_git_menu(cx)))
            .children(
                rows.into_iter()
                    .enumerate()
                    .map(|(index, (space_id, name, commits))| {
                        let selected = self.git_filter == space_id;
                        let id: SharedString = format!("home-git-project-{index}").into();
                        popover::menu_row(theme, selected, id.clone())
                            .id(id)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_git_project(space_id.clone(), cx)
                            }))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(name),
                            )
                            .when_some(commits, |row, commits| {
                                row.child(mono(theme, theme.text_faint, commits.to_string()))
                            })
                    }),
            )
            .into_any_element()
    }

    fn render_this_mac(&mut self, theme: &Theme, cx: &App) -> AnyElement {
        let name = {
            let state = self.state.read(cx);
            state
                .local_device_id
                .as_deref()
                .and_then(|id| state.device_name(id))
                .unwrap_or("This Mac")
                .to_owned()
        };
        let body = match &self.stats.value {
            None => vec![empty_line(theme, "Loading…")],
            Some(Err(error)) => vec![empty_line(theme, error)],
            Some(Ok(stats)) if !stats.available => {
                vec![empty_line(theme, "Unavailable on this system")]
            }
            Some(Ok(stats)) => {
                let share = |used: u64, total: u64| {
                    if total == 0 {
                        0.0
                    } else {
                        used as f32 / total as f32
                    }
                };
                vec![
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(dot(theme.success))
                        .child(div().text_color(theme.text).child(name))
                        .child(
                            div()
                                .text_size(crate::typography::ui_rems(11.0))
                                .text_color(theme.text_faint)
                                .child(format!(
                                    "{} CPU · {} · {} disk",
                                    stats.cores,
                                    gigabytes(stats.memory_total),
                                    gigabytes(stats.disk_total)
                                )),
                        )
                        .into_any_element(),
                    div()
                        .flex()
                        .gap(px(10.0))
                        .pt(px(6.0))
                        .child(meter(theme, "CPU", stats.cpu_percent / 100.0))
                        .child(meter(
                            theme,
                            "Mem",
                            share(stats.memory_used, stats.memory_total),
                        ))
                        .child(meter(
                            theme,
                            "Disk",
                            share(stats.disk_used, stats.disk_total),
                        ))
                        .into_any_element(),
                ]
            }
        };
        card(theme, icons::LAPTOP, "This Mac", None, None, body)
    }

    fn render_services(&mut self, theme: &Theme) -> AnyElement {
        let mode = if self.services_all_ports {
            "All ports"
        } else {
            "By Zeron"
        };
        let body: Vec<AnyElement> = match &self.services.value {
            None => vec![empty_line(theme, "Loading…")],
            Some(Err(error)) => vec![empty_line(theme, error)],
            Some(Ok(list)) if list.services.is_empty() => {
                vec![empty_line(theme, "Nothing listening")]
            }
            Some(Ok(list)) => {
                let services = &list.services;
                let mut rows: Vec<AnyElement> = services
                    .iter()
                    .take(5)
                    .map(|service| {
                        let owned = service.project.is_some();
                        let label = match (&service.project, &service.worktree) {
                            (Some(project), Some(worktree)) => format!("{project} / {worktree}"),
                            (Some(project), None) => project.clone(),
                            (None, _) => service.command.clone(),
                        };
                        let right = service
                            .framework
                            .clone()
                            .unwrap_or_else(|| service.command.clone());
                        let url = format!("http://localhost:{}", service.port);
                        div()
                            .id(SharedString::from(format!("home-service-{}", service.port)))
                            .h(px(ROW_HEIGHT))
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .px(px(6.0))
                            .mx(px(-6.0))
                            .rounded(px(6.0))
                            .cursor_pointer()
                            .hover(|style| style.bg(theme.wash(0.05)))
                            .on_click(move |_, _, cx| cx.open_url(&url))
                            .child(dot(if owned {
                                theme.success
                            } else {
                                theme.text_faint
                            }))
                            .child(div().w(px(48.0)).flex_none().child(mono(
                                theme,
                                theme.text_muted,
                                format!(":{}", service.port),
                            )))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .text_color(if owned { theme.text } else { theme.text_muted })
                                    .child(label),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(crate::typography::ui_rems(12.0))
                                    .text_color(theme.text_faint)
                                    .child(right),
                            )
                            .into_any_element()
                    })
                    .collect();
                if services.len() > 5 {
                    rows.push(empty_line(
                        theme,
                        &format!("+{} more listening", services.len() - 5),
                    ));
                }
                rows
            }
        };
        card(
            theme,
            icons::GLOBAL,
            "Running services",
            Some(mode.into()),
            None,
            body,
        )
    }

    fn render_recent(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let now = Utc::now();
        let areas = recent_areas(self.state.read(cx), 4);
        let body: Vec<AnyElement> = if areas.is_empty() {
            vec![empty_line(theme, "No recent work")]
        } else {
            areas
                .into_iter()
                .enumerate()
                .map(|(index, area)| {
                    let chat_id = area.chat_id.clone();
                    div()
                        .id(SharedString::from(format!("home-recent-{index}")))
                        .h(px(ROW_HEIGHT))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .px(px(6.0))
                        .mx(px(-6.0))
                        .rounded(px(6.0))
                        .cursor_pointer()
                        .hover(|style| style.bg(theme.wash(0.05)))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(HomeEvent::OpenChat(chat_id.clone()))
                        }))
                        .child(
                            icon(icons::LUCIDE_GIT_BRANCH)
                                .size(px(12.0))
                                .flex_none()
                                .text_color(theme.text_faint),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .text_color(theme.text)
                                .child(area.label),
                        )
                        .child(mono(theme, theme.text_faint, short_age(now, area.at)))
                        .into_any_element()
                })
                .collect()
        };
        card(theme, icons::WORKTREE, "Recent areas", None, None, body)
    }

    fn render_ci(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let now = Utc::now();
        let body: Vec<AnyElement> = match &self.prs.value {
            None => vec![empty_line(theme, "Loading…")],
            Some(Err(error)) => vec![empty_line(theme, error)],
            Some(Ok(listing)) => {
                let failing = failing_prs(listing);
                if failing.is_empty() {
                    vec![empty_line(theme, "All checks passing")]
                } else {
                    failing
                        .into_iter()
                        .take(4)
                        .map(|item| {
                            let check = self
                                .failing_checks
                                .get(&item.url)
                                .cloned()
                                .unwrap_or_else(|| "Checks failing".into());
                            let repo = item
                                .repository
                                .rsplit('/')
                                .next()
                                .unwrap_or(&item.repository)
                                .to_owned();
                            let url = item.url.clone();
                            div()
                                .id(SharedString::from(format!("home-ci-{}", item.url)))
                                .h(px(ROW_HEIGHT))
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .px(px(6.0))
                                .mx(px(-6.0))
                                .rounded(px(6.0))
                                .cursor_pointer()
                                .hover(|style| style.bg(theme.wash(0.05)))
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(HomeEvent::OpenPullRequest(url.clone()))
                                }))
                                .child(
                                    icon(icons::LUCIDE_CIRCLE_X)
                                        .size(px(13.0))
                                        .flex_none()
                                        .text_color(theme.danger),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .whitespace_nowrap()
                                        .text_color(theme.text)
                                        .child(check)
                                        .child(
                                            div()
                                                .ml(px(6.0))
                                                .text_color(theme.text_faint)
                                                .child(format!("· {repo} / {}", item.head_ref)),
                                        )
                                        .flex()
                                        .flex_row(),
                                )
                                .child(mono(
                                    theme,
                                    theme.text_faint,
                                    short_age(now, item.updated_at),
                                ))
                                .into_any_element()
                        })
                        .collect()
                }
            }
        };
        card(
            theme,
            icons::LUCIDE_CIRCLE_X,
            "CI failures",
            None,
            None,
            body,
        )
    }
}

fn review_label(item: &ChangeRequestListItem) -> &'static str {
    if item.is_draft {
        "Draft"
    } else {
        match item.review_decision {
            Some(ChangeRequestReviewDecision::Approved) => "Approved",
            Some(ChangeRequestReviewDecision::ChangesRequested) => "Changes",
            _ => "In review",
        }
    }
}

fn pr_row(
    theme: &Theme,
    item: &ChangeRequestListItem,
    right: SharedString,
    cx: &mut Context<HomeView>,
) -> AnyElement {
    let url = item.url.clone();
    let repo = item
        .repository
        .rsplit('/')
        .next()
        .unwrap_or(&item.repository)
        .to_owned();
    let check = match item.checks_state {
        Some(ChangeRequestChecksState::Passing) => Some((icons::LUCIDE_CHECK, theme.success)),
        Some(ChangeRequestChecksState::Failing) => Some((icons::LUCIDE_CIRCLE_X, theme.danger)),
        Some(ChangeRequestChecksState::Pending) => {
            Some((icons::LUCIDE_CIRCLE_DASHED, theme.text_faint))
        }
        None => None,
    };
    div()
        .id(SharedString::from(format!("home-pr-{}", item.url)))
        .h(px(ROW_HEIGHT))
        .flex()
        .items_center()
        .gap(px(8.0))
        .px(px(6.0))
        .mx(px(-6.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .hover(|style| style.bg(theme.wash(0.05)))
        .on_click(cx.listener(move |_, _, _, cx| cx.emit(HomeEvent::OpenPullRequest(url.clone()))))
        .child(
            icon(icons::LUCIDE_GIT_PULL_REQUEST)
                .size(px(12.0))
                .flex_none()
                .text_color(if item.is_draft {
                    theme.text_faint
                } else {
                    theme.success
                }),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_color(theme.text)
                .child(item.title.clone()),
        )
        .child(mono(
            theme,
            theme.text_faint,
            format!("{repo}#{}", item.number),
        ))
        .child(
            div()
                .w(px(14.0))
                .flex_none()
                .when_some(check, |el, (path, color)| {
                    el.child(icon(path).size(px(12.0)).text_color(color))
                }),
        )
        .child(
            div()
                .w(px(68.0))
                .flex_none()
                .flex()
                .justify_end()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_size(crate::typography::ui_rems(12.0))
                .text_color(theme.text_muted)
                .child(right),
        )
        .into_any_element()
}

fn git_chart(
    theme: &Theme,
    activity: &GitActivity,
    hovered: Option<usize>,
    cx: &mut Context<HomeView>,
) -> AnyElement {
    let max = activity
        .days
        .iter()
        .map(|day| day.commits)
        .max()
        .unwrap_or(0)
        .max(1);
    let last = activity.days.len().saturating_sub(1);
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_end()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .items_end()
                        .gap(px(6.0))
                        .child(
                            div()
                                .text_size(crate::typography::ui_rems(26.0))
                                .line_height(px(30.0))
                                .text_color(theme.text)
                                .child(activity.commits.to_string()),
                        )
                        .child(
                            div()
                                .pb(px(3.0))
                                .text_size(crate::typography::ui_rems(12.0))
                                .text_color(theme.text_muted)
                                .child(format!("commits · {} days", activity.days.len())),
                        ),
                )
                .child(
                    div()
                        .pb(px(3.0))
                        .flex()
                        .gap(px(6.0))
                        .child(mono(
                            theme,
                            theme.success,
                            format!("+{}", line_count(activity.additions)),
                        ))
                        .child(mono(
                            theme,
                            theme.danger,
                            format!("-{}", line_count(activity.deletions)),
                        )),
                ),
        )
        .child(
            div()
                .mt(px(10.0))
                .h(px(56.0))
                .relative()
                .flex()
                .items_end()
                .gap(px(5.0))
                .children(activity.days.iter().enumerate().map(|(index, day)| {
                    let height = if day.commits == 0 {
                        2.0
                    } else {
                        (day.commits as f32 / max as f32 * 56.0).max(4.0)
                    };
                    let color = if day.commits == 0 {
                        theme.wash(0.08)
                    } else if hovered == Some(index) || (hovered.is_none() && index == last) {
                        theme.accent
                    } else {
                        theme.accent.opacity(0.7)
                    };
                    div()
                        .id(("home-git-day", index))
                        .flex_1()
                        .h_full()
                        .flex()
                        .items_end()
                        .on_hover(cx.listener(move |this, hovering: &bool, _, cx| {
                            let next = if *hovering {
                                Some(index)
                            } else if this.git_hover == Some(index) {
                                None
                            } else {
                                this.git_hover
                            };
                            if next != this.git_hover {
                                this.git_hover = next;
                                cx.notify();
                            }
                        }))
                        .child(div().w_full().h(px(height)).rounded_t(px(2.0)).bg(color))
                }))
                .when_some(
                    hovered.and_then(|index| activity.days.get(index).map(|day| (index, day))),
                    |el, (index, day)| el.child(day_label(theme, index, activity.days.len(), day)),
                ),
        )
        .child(
            div()
                .mt(px(3.0))
                .flex()
                .gap(px(5.0))
                .children(activity.days.iter().map(|day| {
                    let weekday = day.date.weekday().to_string();
                    div()
                        .flex_1()
                        .flex()
                        .justify_center()
                        .text_size(crate::typography::ui_rems(9.0))
                        .text_color(theme.text_faint)
                        .child(weekday.chars().next().unwrap_or(' ').to_string())
                })),
        )
        .into_any_element()
}

fn day_label(
    theme: &Theme,
    index: usize,
    count: usize,
    day: &zeron_proto::GitActivityDay,
) -> AnyElement {
    let commits = match day.commits {
        1 => "1 commit".to_owned(),
        n => format!("{n} commits"),
    };
    let chip = div()
        .px(px(8.0))
        .py(px(3.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(theme.border)
        .bg(popover::surface_bg(theme))
        .whitespace_nowrap()
        .text_size(crate::typography::ui_rems(11.0))
        .flex()
        .gap(px(6.0))
        .child(
            div()
                .text_color(theme.text_muted)
                .child(day.date.format("%a, %b %-d").to_string()),
        )
        .child(div().text_color(theme.text).child(commits));
    let slot = 1.0 / count.max(1) as f32;
    let anchor = div().absolute().top(px(-26.0));
    let anchor = if index * 2 < count {
        anchor.left(gpui::relative(slot * index as f32))
    } else {
        anchor.right(gpui::relative(slot * (count - 1 - index) as f32))
    };
    anchor
        .child(crate::frost::frosted(6.0, 16.0, chip))
        .into_any_element()
}

fn card(
    theme: &Theme,
    glyph: &'static str,
    title: &'static str,
    count: Option<String>,
    accessory: Option<AnyElement>,
    body: Vec<AnyElement>,
) -> AnyElement {
    div()
        .w_full()
        .flex()
        .flex_col()
        .rounded(px(CARD_RADIUS))
        .border_1()
        .border_color(theme.border)
        .bg(theme.wash(0.03))
        .px(px(12.0))
        .pt(px(10.0))
        .pb(px(10.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .pb(px(6.0))
                .text_size(crate::typography::ui_rems(12.0))
                .child(icon(glyph).size(px(13.0)).text_color(theme.text_muted))
                .child(div().text_color(theme.text_muted).child(title))
                .when_some(count, |el, count| {
                    el.child(div().text_color(theme.text_faint).child(count))
                })
                .when_some(accessory, |el, accessory| {
                    el.child(div().flex_1()).child(accessory)
                }),
        )
        .children(body)
        .into_any_element()
}

fn sub_heading(theme: &Theme, color: Hsla, text: String) -> AnyElement {
    let _ = theme;
    div()
        .pt(px(4.0))
        .pb(px(2.0))
        .text_size(crate::typography::ui_rems(11.0))
        .text_color(color)
        .child(text)
        .into_any_element()
}

fn empty_line(theme: &Theme, text: &str) -> AnyElement {
    div()
        .py(px(4.0))
        .text_size(crate::typography::ui_rems(12.0))
        .text_color(theme.text_faint)
        .child(text.to_owned())
        .into_any_element()
}

fn mono(theme: &Theme, color: Hsla, text: String) -> gpui::Div {
    div()
        .flex_none()
        .font_family(theme.font_mono.clone())
        .text_size(crate::typography::ui_rems(11.0))
        .text_color(color)
        .child(text)
}

fn dot(color: Hsla) -> gpui::Div {
    div().size(px(6.0)).flex_none().rounded_full().bg(color)
}

fn meter(theme: &Theme, label: &'static str, share: f32) -> gpui::Div {
    let share = share.clamp(0.0, 1.0);
    div()
        .flex_1()
        .flex()
        .flex_col()
        .gap(px(3.0))
        .child(
            div()
                .flex()
                .justify_between()
                .text_size(crate::typography::ui_rems(10.0))
                .text_color(theme.text_faint)
                .child(label)
                .child(format!("{:.0}%", share * 100.0)),
        )
        .child(
            div().h(px(3.0)).rounded_full().bg(theme.wash(0.1)).child(
                div()
                    .h_full()
                    .w(gpui::relative(share))
                    .rounded_full()
                    .bg(theme.text_muted),
            ),
        )
}

impl Render for HomeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let first = !self.visible();
        self.last_render = Some(Instant::now());
        if first {
            self.refresh(cx);
        }
        let theme = Theme::of(cx).clone();
        let toggles = crate::settings::home_widgets(cx);
        let enabled: Vec<(Widget, f32)> = Widget::ORDER
            .into_iter()
            .filter(|widget| widget.enabled(&toggles))
            .map(|widget| {
                (
                    widget,
                    self.estimated_rows(widget, cx) as f32 * ROW_HEIGHT + 40.0,
                )
            })
            .collect();
        let [left, right] = columns(&enabled);
        let left: Vec<AnyElement> = left
            .into_iter()
            .map(|widget| self.render_widget(widget, &theme, cx))
            .collect();
        let right: Vec<AnyElement> = right
            .into_iter()
            .map(|widget| self.render_widget(widget, &theme, cx))
            .collect();
        let column = |cards: Vec<AnyElement>| {
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(COLUMN_GAP))
                .children(cards)
        };
        let reveal = self.grid_reveal(toggles.collapsed, window, cx);
        let customize_open = self.customize.is_open();
        let customize = div()
            .id("home-customize")
            .debug_selector(|| "home-customize".into())
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(6.0))
            .h(px(24.0))
            .rounded(px(6.0))
            .text_size(crate::typography::ui_rems(12.0))
            .text_color(theme.text_muted)
            .cursor_pointer()
            .when(customize_open, |el| {
                el.bg(theme.wash(0.06)).text_color(theme.text)
            })
            .hover(|style| style.bg(theme.wash(0.05)).text_color(theme.text))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, _| this.customize.note_trigger_press()),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                if this.customize.take_press_was_open() {
                    this.close_customize(cx);
                } else {
                    this.customize.open(());
                    cx.notify();
                }
            }))
            .child(
                icon(icons::WIDGET)
                    .size(px(13.0))
                    .text_color(theme.text_muted),
            )
            .child("Customize");
        let customize = if self.customize.get().is_some() {
            let closing = self.customize.closing_since();
            let menu = self.render_customize_menu(&theme, cx);
            customize.relative().child(popover::anchored_menu_below_end(
                "home-customize-menu",
                menu,
                closing,
            ))
        } else {
            customize
        };
        let summary = self.summary(cx);
        let pill = div()
            .id("home-collapse")
            .debug_selector(|| "home-collapse".into())
            .h(px(32.0))
            .pl(px(12.0))
            .pr(px(6.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .rounded_full()
            .border_1()
            .border_color(theme.border)
            .bg(theme.wash(0.03))
            .cursor_pointer()
            .hover(|style| style.bg(theme.wash(0.06)))
            .role(gpui::Role::Button)
            .aria_label(if toggles.collapsed {
                "Show widgets"
            } else {
                "Hide widgets"
            })
            .on_click(cx.listener(|this, _, _, cx| this.toggle_collapsed(cx)))
            .child(
                icon(icons::HOME)
                    .size(px(13.0))
                    .flex_none()
                    .text_color(theme.text_muted),
            )
            .child(
                div()
                    .text_size(crate::typography::ui_rems(12.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .whitespace_nowrap()
                    .child(summary),
            )
            .child(
                div().relative().size(px(20.0)).flex_none().child(
                    icon(icons::ALT_ARROW_DOWN)
                        .absolute()
                        .left(px(3.0))
                        .top(px(3.0))
                        .size(px(14.0))
                        .text_color(theme.text_muted)
                        .with_transformation(gpui::Transformation::rotate(gpui::radians(
                            std::f32::consts::PI * (1.0 - reveal),
                        ))),
                ),
            );
        let header = div()
            .relative()
            .h(px(32.0))
            .flex()
            .justify_center()
            .child(pill)
            .child(div().absolute().right_0().top(px(4.0)).child(customize));
        let measured = self.grid_height.clone();
        let natural = measured.get();
        let grid = div()
            .relative()
            .top(px(-8.0 * (1.0 - reveal)))
            .opacity(reveal)
            .flex()
            .items_start()
            .gap(px(COLUMN_GAP))
            .when(!left.is_empty(), |el| el.child(column(left)))
            .when(!right.is_empty(), |el| el.child(column(right)))
            .child(
                gpui::canvas(
                    move |bounds, _, _| measured.set(f32::from(bounds.size.height)),
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            );
        crate::edge_fade::edge_faded(
            16.0,
            true,
            true,
            div()
                .id("home-widgets")
                .size_full()
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .text_size(crate::typography::ui_rems(13.0))
                .child(
                    div()
                        .w_full()
                        .max_w(px(GRID_MAX_WIDTH))
                        .mx_auto()
                        .px(px(24.0))
                        .pb(px(48.0))
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .child(header)
                        .when(reveal > 0.0, |el| {
                            el.child(
                                div()
                                    .when(reveal < 1.0 && natural > 0.0, |el| {
                                        el.h(px(natural * reveal)).overflow_hidden()
                                    })
                                    .child(grid),
                            )
                        }),
                ),
        )
        .fade_overflow_y(&self.scroll)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_balance_by_estimated_height() {
        let placed = columns(&[
            (Widget::WorkingNow, 100.0),
            (Widget::PullRequests, 300.0),
            (Widget::GitActivity, 150.0),
            (Widget::RunningServices, 120.0),
            (Widget::ThisMac, 80.0),
        ]);
        assert_eq!(
            placed,
            [
                vec![
                    Widget::WorkingNow,
                    Widget::GitActivity,
                    Widget::RunningServices
                ],
                vec![Widget::PullRequests, Widget::ThisMac],
            ]
        );
    }

    #[test]
    fn ages_and_sizes_read_short() {
        let now = Utc::now();
        assert_eq!(short_age(now, now), "now");
        assert_eq!(short_age(now, now - chrono::Duration::hours(2)), "2h");
        assert_eq!(short_age(now, now - chrono::Duration::days(3)), "3d");
        assert_eq!(gigabytes(34_359_738_368), "34 GB");
        assert_eq!(line_count(8_712), "8.7k");
    }
}
