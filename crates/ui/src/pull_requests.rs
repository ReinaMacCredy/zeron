mod actions;
mod activity;
mod code;
mod detail;
mod devices;
mod filters;
mod handoff;
mod paging;
mod review;
mod tabs;

pub use filters::PullRequestListPrefs;

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use chrono::{DateTime, Utc};
use gpui::{
    Animation, AnimationExt as _, AnyElement, App, Context, Entity, EventEmitter, Hsla,
    IntoElement, MouseButton, MouseMoveEvent, ParentElement, Render, ScrollHandle, SharedString,
    Styled, Subscription, Task, Window, div, percentage, prelude::*, px,
};
use zeron_proto::{
    ChangeRequestChecksState, ChangeRequestFailure, ChangeRequestInvolvement,
    ChangeRequestListItem, ChangeRequestListing, ChangeRequestMergeability, ChangeRequestRef,
    ChangeRequestReviewDecision, ChangeRequestState, ChangeRequestStats,
    ListChangeRequestStatsRequest, ListChangeRequestsRequest,
};
use zeron_rpc::methods;

use crate::composer::{ComposerInput, ComposerInputEvent};
use crate::icons::{self, icon};
use crate::popover::Loadable;
use crate::state::AppState;
use crate::theme::{Appearance, Theme, oklch};

const CONTAINER_MAX_WIDTH: f32 = 1152.0;
const CONTAINER_PAD_X: f32 = 24.0;
const GHOST_ROWS: usize = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PullRequestGroup {
    Authored,
    ReviewRequested,
    Others,
}

impl PullRequestGroup {
    fn label(self) -> &'static str {
        match self {
            Self::Authored => "Authored",
            Self::ReviewRequested => "Review requested",
            Self::Others => "Others",
        }
    }
}

fn authored_by_viewer(item: &ChangeRequestListItem, viewers: &BTreeMap<String, String>) -> bool {
    let (Some(author), Some(viewer)) = (
        item.author.as_ref(),
        viewers.get(&devices::viewer_key(item)),
    ) else {
        return false;
    };
    author.login.eq_ignore_ascii_case(viewer)
}

pub(crate) fn group_items<'a>(
    items: &[&'a ChangeRequestListItem],
    viewers: &BTreeMap<String, String>,
) -> Vec<(PullRequestGroup, Vec<&'a ChangeRequestListItem>)> {
    let mut authored = Vec::new();
    let mut review_requested = Vec::new();
    let mut others = Vec::new();
    for item in items {
        if authored_by_viewer(item, viewers) {
            authored.push(*item);
        } else if item.viewer_review_requested {
            review_requested.push(*item);
        } else {
            others.push(*item);
        }
    }
    [
        (PullRequestGroup::Authored, authored),
        (PullRequestGroup::ReviewRequested, review_requested),
        (PullRequestGroup::Others, others),
    ]
    .into_iter()
    .filter(|(_, items)| !items.is_empty())
    .collect()
}

fn readiness_tier(item: &ChangeRequestListItem) -> u8 {
    if item.mergeability == ChangeRequestMergeability::Conflicting {
        4
    } else if item.state != ChangeRequestState::Open {
        3
    } else if item.is_draft {
        2
    } else if item.checks_state == Some(ChangeRequestChecksState::Passing)
        && item.review_decision == Some(ChangeRequestReviewDecision::Approved)
    {
        0
    } else if item.checks_state == Some(ChangeRequestChecksState::Passing) {
        1
    } else {
        2
    }
}

pub(crate) fn change_size(item: &ChangeRequestListItem) -> u64 {
    item.additions + item.deletions
}

pub(crate) fn rank_by_merge_readiness(items: &mut [&ChangeRequestListItem]) {
    items.sort_by(|left, right| {
        let size = change_size;
        readiness_tier(left)
            .cmp(&readiness_tier(right))
            .then_with(|| (size(right) > 0).cmp(&(size(left) > 0)))
            .then_with(|| size(left).cmp(&size(right)))
            .then_with(|| right.updated_at.cmp(&left.updated_at))
    });
}

pub(crate) fn matches_query(item: &ChangeRequestListItem, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    let haystack = format!(
        "#{} {} {} {} {}",
        item.number,
        item.title,
        item.repository,
        item.head_ref,
        item.author
            .as_ref()
            .map(|author| author.login.as_str())
            .unwrap_or_default(),
    )
    .to_lowercase();
    haystack.contains(&query)
}

pub(crate) fn relative_time(now: DateTime<Utc>, at: DateTime<Utc>) -> String {
    let seconds = (now - at).num_seconds();
    if seconds < 60 {
        return "just now".into();
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m ago");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours}h ago");
    }
    format!("{}d ago", hours / 24)
}

pub(crate) fn failure_message(failure: ChangeRequestFailure) -> &'static str {
    match failure {
        ChangeRequestFailure::CliUnavailable => {
            "GitHub CLI (`gh`) is required to browse pull requests. Install it from https://cli.github.com/ and reload."
        }
        ChangeRequestFailure::Authentication => {
            "GitHub CLI is not authenticated. Run `gh auth login` and retry."
        }
        ChangeRequestFailure::RateLimited => "GitHub's rate limit was reached. Try again shortly.",
        ChangeRequestFailure::Timeout => "GitHub took too long to answer.",
        ChangeRequestFailure::Decode => "GitHub returned a response that could not be read.",
        ChangeRequestFailure::CommandFailed => "GitHub CLI could not list pull requests.",
        ChangeRequestFailure::RepositoryUnavailable => "This folder is not a Git checkout.",
        ChangeRequestFailure::UnsupportedRepository => {
            "Change requests cannot be browsed for this project's host yet."
        }
    }
}

fn shown_failure(failure: ChangeRequestFailure) -> bool {
    !matches!(
        failure,
        ChangeRequestFailure::RepositoryUnavailable | ChangeRequestFailure::UnsupportedRepository
    )
}

#[derive(Clone, Copy)]
struct Tones {
    open: Hsla,
    merged: Hsla,
    closed: Hsla,
    draft: Hsla,
    conflict: Hsla,
    pending: Hsla,
    approved_text: Hsla,
    changes_text: Hsla,
}

impl Tones {
    fn of(theme: &Theme) -> Self {
        let dark = theme.appearance == Appearance::Dark;
        let alpha = |color: Hsla, a: f32| Hsla { a, ..color };
        if dark {
            Self {
                open: alpha(oklch(0.845, 0.143, 164.978), 0.9),
                merged: alpha(oklch(0.811, 0.111, 293.571), 0.9),
                closed: alpha(oklch(0.808, 0.114, 19.571), 0.9),
                draft: alpha(oklch(0.705, 0.015, 286.067), 0.8),
                conflict: theme.danger,
                pending: alpha(oklch(0.828, 0.189, 84.429), 0.9),
                approved_text: alpha(oklch(0.765, 0.177, 163.223), 0.8),
                changes_text: alpha(oklch(0.828, 0.189, 84.429), 0.8),
            }
        } else {
            Self {
                open: oklch(0.596, 0.145, 163.225),
                merged: oklch(0.541, 0.281, 293.009),
                closed: oklch(0.577, 0.245, 27.325),
                draft: oklch(0.552, 0.016, 285.938),
                conflict: theme.danger,
                pending: oklch(0.666, 0.179, 58.318),
                approved_text: alpha(oklch(0.596, 0.145, 163.225), 0.9),
                changes_text: alpha(oklch(0.666, 0.179, 58.318), 0.9),
            }
        }
    }

    fn glyph(self, item: &ChangeRequestListItem) -> (&'static str, Hsla, String) {
        match item.state {
            ChangeRequestState::Merged => (icons::LUCIDE_GIT_MERGE, self.merged, "Merged".into()),
            ChangeRequestState::Closed => (
                icons::LUCIDE_GIT_PULL_REQUEST_CLOSED,
                self.closed,
                "Closed".into(),
            ),
            ChangeRequestState::Open if item.is_draft => (
                icons::LUCIDE_GIT_PULL_REQUEST_DRAFT,
                self.draft,
                "Draft".into(),
            ),
            ChangeRequestState::Open
                if item.mergeability == ChangeRequestMergeability::Conflicting =>
            {
                (
                    icons::LUCIDE_TRIANGLE_ALERT,
                    self.conflict,
                    format!("Conflicts with {}", item.base_ref),
                )
            }
            ChangeRequestState::Open => (icons::LUCIDE_GIT_PULL_REQUEST, self.open, "Open".into()),
        }
    }

    fn checks(self, state: ChangeRequestChecksState) -> (&'static str, Hsla) {
        match state {
            ChangeRequestChecksState::Passing => (icons::LUCIDE_CIRCLE_CHECK, self.open),
            ChangeRequestChecksState::Failing => (icons::LUCIDE_CIRCLE_X, self.conflict),
            ChangeRequestChecksState::Pending => (icons::LUCIDE_CIRCLE_DOT, self.pending),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PullRequestKey {
    host: String,
    repository: String,
    number: u64,
    device: Option<String>,
}

impl PullRequestKey {
    fn of(item: &ChangeRequestListItem) -> Self {
        Self {
            host: item.host.clone(),
            repository: item.repository.clone(),
            number: item.number,
            device: item.device_id.clone(),
        }
    }

    fn route(&self, params: serde_json::Value) -> serde_json::Value {
        routed(params, self.device.as_deref())
    }
}

fn routed(mut params: serde_json::Value, device: Option<&str>) -> serde_json::Value {
    if let (Some(device), Some(map)) = (device, params.as_object_mut()) {
        map.insert("targetDeviceId".into(), device.into());
    }
    params
}

pub use handoff::ChatHandoff;

pub enum PullRequestsEvent {
    Close,
    OpenChat(ChatHandoff),
}

pub struct PullRequestsPage {
    state: Entity<AppState>,
    search: Entity<ComposerInput>,
    _search_sub: Subscription,
    listing: Loadable<ChangeRequestListing>,
    refreshing: bool,
    load_task: Option<Task<()>>,
    scroll: ScrollHandle,
    tabs: Vec<PullRequestKey>,
    active: Option<PullRequestKey>,
    details: HashMap<PullRequestKey, detail::DetailState>,
    hovered_tab: Option<PullRequestKey>,
    panel_width: Option<f32>,
    resize_from: Option<(f32, f32)>,
    stats_task: Option<Task<()>>,
    checks_popover: Option<detail::ChecksPopover>,
    prefs: PullRequestListPrefs,
    menu: crate::popover::Popup<filters::ListMenu>,
    author_search: Entity<ComposerInput>,
    _author_search_sub: Subscription,
    sent_query: String,
    sent: (String, zeron_proto::ChangeRequestListFilters),
    debounce_task: Option<Task<()>>,
    toasts: Vec<(u64, SharedString, Option<SharedString>)>,
    confirm: Option<actions::Confirm>,
    handoff: Option<(PullRequestKey, handoff::Handoff)>,
    limit: u32,
    paged_key: String,
    last_read: std::time::Instant,
    last_interaction: std::time::Instant,
    window_active: bool,
    visible: bool,
    poll_task: Option<Task<()>>,
    checkout_menu: Option<(PullRequestKey, crate::popover::Popup<()>)>,
    toast_seq: u64,
    tab_menu: crate::popover::Popup<(PullRequestKey, gpui::Point<gpui::Pixels>)>,
}

impl EventEmitter<PullRequestsEvent> for PullRequestsPage {}

impl PullRequestsPage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| {
            ComposerInput::with_context("Search pull requests, or label:bug", "PaletteSearch", cx)
        });
        let search_sub = cx.subscribe(&search, |page: &mut Self, _, event, cx| {
            if matches!(event, ComposerInputEvent::Edited) {
                page.schedule_search(cx);
                cx.notify();
            }
        });
        let author_search =
            cx.new(|cx| ComposerInput::with_context("Search authors", "PaletteSearch", cx));
        let author_search_sub = cx.subscribe(&author_search, |_, _, event, cx| {
            if matches!(event, ComposerInputEvent::Edited) {
                cx.notify();
            }
        });
        let prefs = crate::settings::current(cx).pull_requests_list.clone();
        let mut page = Self {
            state,
            search,
            _search_sub: search_sub,
            listing: Loadable::Idle,
            refreshing: false,
            load_task: None,
            scroll: ScrollHandle::new(),
            tabs: Vec::new(),
            active: None,
            details: HashMap::new(),
            hovered_tab: None,
            panel_width: None,
            resize_from: None,
            stats_task: None,
            checks_popover: None,
            prefs,
            menu: Default::default(),
            author_search,
            _author_search_sub: author_search_sub,
            sent_query: String::new(),
            sent: Default::default(),
            debounce_task: None,
            toasts: Vec::new(),
            confirm: None,
            handoff: None,
            limit: paging::PAGE_SIZE,
            paged_key: String::new(),
            last_read: std::time::Instant::now(),
            last_interaction: std::time::Instant::now(),
            window_active: true,
            visible: true,
            poll_task: None,
            checkout_menu: None,
            toast_seq: 0,
            tab_menu: Default::default(),
        };
        if let Some(listing) = paging::snapshot_dir(cx).and_then(|dir| paging::load_list(&dir)) {
            page.listing = Loadable::Ready(listing);
        }
        page.refresh(cx);
        page.start_live_refresh(cx);
        page
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        if matches!(self.listing, Loadable::Ready(_)) {
            self.refreshing = true;
        } else {
            self.listing = Loadable::Loading;
        }
        let (text, typed) = &self.sent;
        let mut request = ListChangeRequestsRequest {
            state: self.prefs.state,
            involvement: self.prefs.involvement,
            limit: None,
            query: (!text.trim().is_empty()).then(|| text.trim().to_string()),
            filters: filters::effective_filters(&self.prefs.filters, typed),
        };
        let paged_key = serde_json::to_string(&request).unwrap_or_default();
        if paged_key != self.paged_key {
            self.paged_key = paged_key;
            self.limit = paging::PAGE_SIZE;
        }
        request.limit = Some(self.limit);
        let unsearched = request.query.is_none();
        self.last_read = std::time::Instant::now();
        let params = serde_json::to_value(request).unwrap_or(serde_json::Value::Null);
        let devices: Vec<devices::ListDevice> = self
            .list_devices(cx)
            .into_iter()
            .filter(|device| {
                self.prefs
                    .device
                    .as_ref()
                    .is_none_or(|wanted| *wanted == device.id)
            })
            .collect();
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let answers = futures::future::join_all(devices.iter().map(|device| {
                let params = routed(params.clone(), device.target().as_deref());
                let engine = engine.clone();
                async move {
                    engine
                        .client()
                        .call(methods::LIST_CHANGE_REQUESTS, params)
                        .await
                        .map_err(|error| error.to_string())
                        .and_then(|value| {
                            serde_json::from_value::<ChangeRequestListing>(value)
                                .map_err(|error| error.to_string())
                        })
                }
            }))
            .await;
            let parsed = devices::merge_listings(devices.iter().zip(answers).collect());
            this.update(cx, |page, cx| {
                page.refreshing = false;
                match parsed {
                    Ok(mut listing) => {
                        if let Loadable::Ready(previous) = &page.listing {
                            for item in &mut listing.items {
                                if let Some(known) = previous.items.iter().find(|known| {
                                    PullRequestKey::of(known) == PullRequestKey::of(item)
                                }) && change_size(item) == 0
                                {
                                    item.additions = known.additions;
                                    item.deletions = known.deletions;
                                }
                            }
                        }
                        if unsearched
                            && listing.errors.is_empty()
                            && let Some(dir) = paging::snapshot_dir(cx)
                        {
                            paging::save_list(&dir, &listing);
                        }
                        page.listing = Loadable::Ready(listing);
                        page.load_stats(cx);
                    }
                    Err(error) if !matches!(page.listing, Loadable::Ready(_)) => {
                        page.listing = Loadable::Error(error)
                    }
                    Err(_) => {}
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn load_stats(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let Loadable::Ready(listing) = &self.listing else {
            return;
        };
        let mut by_device: BTreeMap<Option<String>, Vec<ChangeRequestRef>> = BTreeMap::new();
        for item in listing
            .items
            .iter()
            .filter(|item| item.additions + item.deletions == 0)
        {
            by_device
                .entry(item.device_id.clone())
                .or_default()
                .push(ChangeRequestRef {
                    host: item.host.clone(),
                    repository: item.repository.clone(),
                    number: item.number,
                });
        }
        if by_device.is_empty() {
            return;
        }
        self.stats_task = Some(cx.spawn(async move |this, cx| {
            let answers = futures::future::join_all(by_device.into_iter().map(|(device, refs)| {
                let params = routed(
                    serde_json::to_value(ListChangeRequestStatsRequest { refs })
                        .unwrap_or(serde_json::Value::Null),
                    device.as_deref(),
                );
                let engine = engine.clone();
                async move {
                    let stats = engine
                        .client()
                        .call(methods::LIST_CHANGE_REQUEST_STATS, params)
                        .await
                        .ok()
                        .and_then(|value| serde_json::from_value::<ChangeRequestStats>(value).ok());
                    (device, stats)
                }
            }))
            .await;
            this.update(cx, |page, cx| {
                if let Loadable::Ready(listing) = &mut page.listing {
                    for (device, stats) in answers {
                        for stat in stats.map(|stats| stats.stats).unwrap_or_default() {
                            if let Some(item) = listing.items.iter_mut().find(|item| {
                                item.number == stat.reference.number
                                    && item.repository == stat.reference.repository
                                    && item.host == stat.reference.host
                                    && item.device_id == device
                            }) {
                                item.additions = stat.additions;
                                item.deletions = stat.deletions;
                            }
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn list_devices(&self, cx: &App) -> Vec<devices::ListDevice> {
        let state = self.state.read(cx);
        devices::list_devices(
            state.local_device_id.as_deref(),
            &state.devices,
            &state.spaces,
            Utc::now(),
        )
    }

    fn query(&self, cx: &App) -> String {
        self.search.read(cx).text().to_string()
    }

    fn schedule_search(&mut self, cx: &mut Context<Self>) {
        self.debounce_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            this.update(cx, |page, cx| {
                let query = page.query(cx).trim().to_string();
                let parsed = filters::parse_query(&query);
                page.sent_query = query;
                if parsed != page.sent {
                    page.sent = parsed;
                    page.refresh(cx);
                }
                cx.notify();
            })
            .ok();
        }));
    }

    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.scroll.set_offset(gpui::point(px(0.0), px(0.0)));
        let handle = gpui::Focusable::focus_handle(self.search.read(cx), cx);
        window.focus(&handle, cx);
        self.search
            .update(cx, |input, cx| input.select_all_text(cx));
        cx.notify();
    }

    fn search_busy(&self, cx: &App) -> bool {
        self.query(cx).trim() != self.sent_query || (self.refreshing && !self.sent_query.is_empty())
    }

    fn render_topbar(&self, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let spacer = crate::shell::titlebar_spacer_width(
            cfg!(target_os = "macos"),
            window.is_fullscreen(),
            0.0,
        );
        div()
            .h(px(Theme::TITLEBAR_HEIGHT))
            .pt(px(Theme::TITLEBAR_TOP_PAD))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(12.0))
            .child(div().w(px(spacer)).flex_none())
            .child(
                div()
                    .id("pull-requests-back")
                    .role(gpui::Role::Button)
                    .aria_label("Back")
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .h(px(28.0))
                    .px(px(8.0))
                    .rounded(px(8.0))
                    .text_size(px(13.0))
                    .text_color(theme.text_muted)
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.ink(0.06)).text_color(theme.text))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(PullRequestsEvent::Close)))
                    .child(
                        icon(icons::LUCIDE_ARROW_LEFT)
                            .size(px(14.0))
                            .text_color(theme.text_muted),
                    )
                    .child("Back"),
            )
            .child(
                div()
                    .text_size(px(14.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child("Pull Requests"),
            )
            .when(self.list_condensed(), |el| {
                let compact = |id: &'static str, label: &'static str| {
                    div()
                        .id(id)
                        .role(gpui::Role::Button)
                        .aria_label(label)
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .h(px(28.0))
                        .px(px(6.0))
                        .rounded(px(8.0))
                        .text_size(px(14.0))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(theme.text_muted)
                        .cursor_pointer()
                        .hover(|style| style.bg(theme.ink(0.06)).text_color(theme.text))
                        .child(label)
                        .child(
                            icon(icons::LUCIDE_CHEVRON_DOWN)
                                .size(px(12.0))
                                .text_color(theme.text_muted.opacity(0.7)),
                        )
                };
                el.child(div().text_color(theme.text_muted.opacity(0.5)).child("/"))
                    .child(self.menu_trigger(
                        filters::MenuKind::State,
                        compact(
                            "pull-requests-compact-state",
                            filters::state_label(self.prefs.state),
                        ),
                        theme,
                        cx,
                    ))
                    .child(self.menu_trigger(
                        filters::MenuKind::Involvement,
                        compact(
                            "pull-requests-compact-involvement",
                            filters::involvement_label(self.prefs.involvement),
                        ),
                        theme,
                        cx,
                    ))
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("pull-requests-compact-search")
                            .role(gpui::Role::Button)
                            .aria_label("Search pull requests")
                            .size(px(28.0))
                            .mr(px(12.0))
                            .rounded(px(8.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|style| style.bg(theme.ink(0.06)))
                            .on_click(
                                cx.listener(|page, _, window, cx| page.focus_search(window, cx)),
                            )
                            .child(
                                icon(icons::LUCIDE_SEARCH)
                                    .size(px(16.0))
                                    .text_color(theme.text_muted),
                            ),
                    )
            })
            .into_any_element()
    }

    fn list_condensed(&self) -> bool {
        -f32::from(self.scroll.offset().y) > 64.0
    }

    fn render_controls(
        &self,
        theme: &Theme,
        search_busy: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let busy = self.refreshing || matches!(self.listing, Loadable::Loading);
        let refresh_icon = icon(icons::LUCIDE_REFRESH_CW)
            .size(px(16.0))
            .text_color(theme.text);
        let refresh_icon: AnyElement = if busy {
            refresh_icon
                .with_animation(
                    "pull-requests-refresh-spin",
                    Animation::new(Duration::from_millis(800)).repeat(),
                    |svg, delta| {
                        svg.with_transformation(gpui::Transformation::rotate(percentage(delta)))
                    },
                )
                .into_any_element()
        } else {
            refresh_icon.into_any_element()
        };
        let outline = |el: gpui::Stateful<gpui::Div>| {
            el.h(px(32.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(theme.hairline(0.09))
                .bg(theme.surface_raised)
                .hover(|style| style.bg(theme.ink(0.06)))
        };
        let menu_button = |id: &'static str, label: &'static str| {
            outline(div().id(id))
                .flex()
                .items_center()
                .gap(px(6.0))
                .px(px(11.0))
                .role(gpui::Role::Button)
                .aria_label(label)
                .cursor_pointer()
                .text_size(px(14.0))
                .text_color(theme.text)
        };
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.0))
            .child(
                outline(div().id("pull-requests-search"))
                    .flex_1()
                    .min_w(px(160.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(10.0))
                    .child(if search_busy {
                        icon(icons::LUCIDE_REFRESH_CW)
                            .size(px(16.0))
                            .text_color(theme.text_muted)
                            .with_animation(
                                "pull-requests-search-spin",
                                Animation::new(Duration::from_millis(800)).repeat(),
                                |svg, delta| {
                                    svg.with_transformation(gpui::Transformation::rotate(
                                        percentage(delta),
                                    ))
                                },
                            )
                            .into_any_element()
                    } else {
                        icon(icons::LUCIDE_SEARCH)
                            .size(px(16.0))
                            .text_color(theme.text_muted)
                            .into_any_element()
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(14.0))
                            .child(self.search.clone()),
                    ),
            )
            .child(
                self.menu_trigger(
                    filters::MenuKind::Sort,
                    menu_button("pull-requests-sort", "Sort pull requests")
                        .child(
                            icon(icons::LUCIDE_ARROW_DOWN_UP)
                                .size(px(16.0))
                                .text_color(theme.text),
                        )
                        .child("Sort"),
                    theme,
                    cx,
                ),
            )
            .child({
                let count = filters::filter_count(&self.prefs);
                self.menu_trigger(
                    filters::MenuKind::Filters,
                    menu_button("pull-requests-filters", "Filters")
                        .child(icon(icons::LUCIDE_LIST_FILTER).size(px(16.0)).text_color(
                            if count > 0 {
                                theme.text
                            } else {
                                theme.text_muted
                            },
                        ))
                        .child("Filters")
                        .when(count > 0, |el| {
                            el.child(
                                div()
                                    .rounded_full()
                                    .bg(theme.ink(0.06))
                                    .px(px(6.0))
                                    .text_size(px(12.0))
                                    .text_color(theme.text_muted)
                                    .child(count.to_string()),
                            )
                        }),
                    theme,
                    cx,
                )
            })
            .child(
                outline(div().id("pull-requests-refresh"))
                    .w(px(32.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .role(gpui::Role::Button)
                    .aria_label("Refresh pull requests")
                    .cursor_pointer()
                    .on_click(cx.listener(|page, _, _, cx| {
                        if !page.refreshing {
                            page.hard_refresh(cx);
                        }
                    }))
                    .child(refresh_icon),
            )
            .into_any_element()
    }

    fn render_ghost(&self, theme: &Theme) -> AnyElement {
        self.render_ghost_rows(theme, GHOST_ROWS, None)
    }

    fn render_ghost_rows(&self, theme: &Theme, rows: usize, caption: Option<String>) -> AnyElement {
        let title = [0.6, 0.4, 0.5, 0.66, 0.4, 0.6, 0.5];
        let meta = [0.4, 0.33, 0.4, 0.25, 0.33, 0.4, 0.33];
        let bar = theme.text_muted.opacity(0.15);
        div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .children(caption.map(|caption| {
                div()
                    .px(px(12.0))
                    .pb(px(4.0))
                    .text_size(px(12.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text_muted.opacity(0.7))
                    .child(caption)
            }))
            .children((0..rows).map(|index| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .px(px(12.0))
                    .py(px(8.0))
                    .child(div().size(px(16.0)).rounded_full().bg(bar).flex_none())
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .h(px(14.0))
                                    .w(gpui::relative(title[index % 7]))
                                    .rounded(px(4.0))
                                    .bg(bar),
                            )
                            .child(
                                div()
                                    .h(px(12.0))
                                    .w(gpui::relative(meta[index % 7]))
                                    .rounded(px(4.0))
                                    .bg(bar),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_end()
                            .gap(px(8.0))
                            .child(div().h(px(12.0)).w(px(48.0)).rounded(px(4.0)).bg(bar))
                            .child(div().h(px(12.0)).w(px(64.0)).rounded(px(4.0)).bg(bar)),
                    )
            }))
            .with_animation(
                "pull-requests-ghost-pulse",
                Animation::new(Duration::from_millis(1600)).repeat(),
                |el, delta| el.opacity(1.0 - 0.45 * (delta * std::f32::consts::PI).sin()),
            )
            .into_any_element()
    }

    fn render_state(
        &self,
        theme: &Theme,
        title: String,
        description: String,
        with_mark: bool,
        clear_search: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mark: AnyElement = if with_mark {
            branch_mark(theme)
        } else {
            div()
                .size(px(36.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(theme.hairline(0.09))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    icon(icons::LUCIDE_GIT_PULL_REQUEST)
                        .size(px(16.0))
                        .text_color(theme.text),
                )
                .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(24.0))
            .px(px(16.0))
            .py(px(64.0))
            .child(mark)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(6.0))
                    .max_w(px(420.0))
                    .child(
                        div()
                            .text_size(px(20.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(14.0))
                            .text_color(theme.text_muted)
                            .text_center()
                            .child(description),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .when(clear_search, |row| {
                        row.child(
                            state_button(
                                "pull-requests-clear-search",
                                icons::LUCIDE_SEARCH,
                                "Clear search",
                                theme,
                            )
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.search.update(cx, |input, cx| input.set_text("", cx));
                                cx.notify();
                            })),
                        )
                    })
                    .child(
                        state_button(
                            "pull-requests-retry",
                            icons::LUCIDE_REFRESH_CW,
                            if with_mark { "Check again" } else { "Retry" },
                            theme,
                        )
                        .on_click(cx.listener(|page, _, _, cx| page.refresh(cx))),
                    ),
            )
            .into_any_element()
    }

    fn render_row(
        &self,
        item: &ChangeRequestListItem,
        theme: &Theme,
        tones: Tones,
        row_width: f32,
        now: DateTime<Utc>,
        search: &str,
        device_label: Option<String>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (glyph, glyph_tone, _) = tones.glyph(item);
        let matched_elsewhere =
            !search.trim().is_empty() && filters::score_match(item, search) <= 10;
        let selected = self.active.as_ref() == Some(&PullRequestKey::of(item));
        let muted70 = theme.text_muted.opacity(0.7);
        let muted50 = theme.text_muted.opacity(0.5);
        let meta_width = (row_width - 24.0 - 28.0 - 160.0).max(0.0);
        let separator = || div().flex_none().text_color(muted50).child("·");
        let mut meta = div()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .flex()
            .items_center()
            .gap(px(6.0))
            .text_size(px(12.0))
            .text_color(muted70)
            .whitespace_nowrap()
            .when(matched_elsewhere, |el| {
                el.child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(2.0))
                        .rounded_full()
                        .border_1()
                        .border_color(theme.hairline(0.06))
                        .px(px(4.0))
                        .text_size(px(10.0))
                        .child(
                            icon(icons::LUCIDE_SEARCH)
                                .size(px(12.0))
                                .text_color(muted70),
                        )
                        .when(meta_width >= 320.0, |el| {
                            el.child("matched in the description")
                        }),
                )
                .child(separator())
            })
            .child(div().flex_none().child(format!("#{}", item.number)))
            .child(separator())
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(item.repository.clone()),
            );
        if let Some(device) = device_label {
            meta = meta.child(separator()).child(
                div()
                    .max_w(px(128.0))
                    .overflow_hidden()
                    .text_ellipsis()
                    .flex_none()
                    .child(device),
            );
        }
        if let Some(author) = &item.author {
            meta = meta.child(separator()).child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .flex_none()
                    .child(avatar(&author.login, theme))
                    .when(meta_width >= 320.0, |el| el.child(author.login.clone())),
            );
        }
        let visible_labels = if meta_width >= 768.0 {
            3
        } else if meta_width >= 576.0 {
            2
        } else {
            1
        }
        .min(item.labels.len());
        if visible_labels > 0 {
            meta = meta.child(separator());
            for (index, label) in item.labels.iter().take(visible_labels).enumerate() {
                let rest = item.labels.len() - visible_labels;
                let suffix = if index + 1 == visible_labels && rest > 0 {
                    format!(" +{rest}")
                } else {
                    String::new()
                };
                meta = meta.child(label_pill(
                    &label.name,
                    label.color.as_deref(),
                    &suffix,
                    theme,
                ));
            }
        }

        let mut right_top = div()
            .flex()
            .items_center()
            .justify_end()
            .gap(px(8.0))
            .max_w(px(144.0))
            .text_size(px(12.0));
        if row_width >= 448.0 {
            match item.review_decision {
                Some(ChangeRequestReviewDecision::Approved) => {
                    right_top = right_top.child(
                        div()
                            .text_color(tones.approved_text)
                            .whitespace_nowrap()
                            .child("Approved"),
                    )
                }
                Some(ChangeRequestReviewDecision::ChangesRequested) => {
                    right_top = right_top.child(
                        div()
                            .text_color(tones.changes_text)
                            .whitespace_nowrap()
                            .child("Changes requested"),
                    )
                }
                _ => {}
            }
        }
        if let Some(checks) = item.checks_state {
            let (glyph, tone) = tones.checks(checks);
            let popover_item = item.clone();
            right_top = right_top.child(
                div()
                    .id(SharedString::from(format!(
                        "pull-request-checks-{}-{}-{}-{}",
                        item.device_id.as_deref().unwrap_or_default(),
                        item.host,
                        item.repository,
                        item.number
                    )))
                    .role(gpui::Role::Button)
                    .aria_label(SharedString::from(format!(
                        "Checks: {}",
                        detail::checks_verdict_label(checks)
                    )))
                    .rounded(px(4.0))
                    .hover(|style| style.bg(theme.ink(0.06)))
                    .on_click(cx.listener(move |page, event: &gpui::ClickEvent, _, cx| {
                        cx.stop_propagation();
                        page.toggle_checks_popover(&popover_item, event.position(), false, cx);
                    }))
                    .child(icon(glyph).size(px(16.0)).text_color(tone)),
            );
        }

        let mut right_bottom = div()
            .flex()
            .items_center()
            .justify_end()
            .gap(px(12.0))
            .whitespace_nowrap()
            .text_size(px(11.0))
            .text_color(muted70);
        if item.additions + item.deletions > 0 {
            right_bottom =
                right_bottom.child(diff_stat(item.additions, item.deletions, tones, theme));
        }
        if row_width >= 384.0 {
            right_bottom = right_bottom.child(relative_time(now, item.updated_at));
        }

        let url = item.url.clone();
        let opened = item.clone();
        div()
            .id(SharedString::from(format!(
                "pull-request-row-{}-{}-{}-{}",
                item.device_id.as_deref().unwrap_or_default(),
                item.host,
                item.repository,
                item.number
            )))
            .role(gpui::Role::Button)
            .aria_label(SharedString::from(item.title.clone()))
            .w_full()
            .flex()
            .items_center()
            .gap(px(12.0))
            .p(px(12.0))
            .rounded(px(10.0))
            .cursor_pointer()
            .when(selected, |el| el.bg(theme.ink(0.055)))
            .when(!selected, |el| el.hover(|style| style.bg(theme.ink(0.035))))
            .on_click(cx.listener(move |page, event: &gpui::ClickEvent, _, cx| {
                if event.modifiers().platform {
                    cx.open_url(&url);
                } else {
                    page.open_pull_request(&opened, cx);
                }
            }))
            .child(
                icon(glyph)
                    .size(px(16.0))
                    .text_color(glyph_tone)
                    .flex_none(),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.0))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .text_size(px(14.0))
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(theme.text)
                                    .child(item.title.clone()),
                            )
                            .child(right_top),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.0))
                            .child(meta)
                            .child(right_bottom),
                    ),
            )
            .into_any_element()
    }

    fn render_body(
        &self,
        theme: &Theme,
        row_width: f32,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let listing = match &self.listing {
            Loadable::Idle | Loadable::Loading => return self.render_ghost(theme),
            Loadable::Error(error) => {
                return self.render_state(
                    theme,
                    "Could not load pull requests".into(),
                    error.clone(),
                    false,
                    false,
                    cx,
                );
            }
            Loadable::Ready(listing) => listing,
        };
        let shown_errors: Vec<_> = listing
            .errors
            .iter()
            .filter(|error| shown_failure(error.failure))
            .collect();
        if listing.items.is_empty() && !shown_errors.is_empty() {
            return self.render_state(
                theme,
                "Could not load pull requests".into(),
                failure_message(shown_errors[0].failure).into(),
                false,
                false,
                cx,
            );
        }
        let query = self.query(cx);
        let trimmed = query.trim();
        let search_text = if busy {
            filters::parse_query(trimmed).0
        } else {
            self.sent.0.clone()
        };
        let items: Vec<&ChangeRequestListItem> = listing
            .items
            .iter()
            .filter(|item| {
                self.prefs
                    .repository
                    .as_deref()
                    .is_none_or(|repository| item.repository == repository)
                    && self
                        .prefs
                        .host
                        .as_deref()
                        .is_none_or(|host| item.host == host)
            })
            .filter(|item| !busy || matches_query(item, &search_text))
            .collect();
        let shown = crate::rail::truncate_preview(trimmed, 49);
        if items.is_empty() && busy && !search_text.trim().is_empty() {
            return self.render_ghost_rows(
                theme,
                5,
                Some(format!("Searching every host for “{shown}”")),
            );
        }
        if items.is_empty() {
            let (title, description) = if !trimmed.is_empty() {
                (
                    format!("Nothing matches “{shown}”"),
                    "The hosts were searched for it. Try fewer words, or search by number, author or branch."
                        .to_string(),
                )
            } else if filters::filter_count(&self.prefs) > 0 {
                (
                    "Nothing under these filters".to_string(),
                    "Widen the state, involvement or project filter to see more.".to_string(),
                )
            } else {
                (
                    "No pull requests".to_string(),
                    "Pull requests from every project in this workspace appear here.".to_string(),
                )
            };
            return self.render_state(theme, title, description, true, !trimmed.is_empty(), cx);
        }
        let tones = Tones::of(theme);
        let now = Utc::now();
        let list_devices = self.list_devices(cx);
        let device_label = |item: &ChangeRequestListItem| {
            (list_devices.len() >= 2)
                .then(|| {
                    list_devices
                        .iter()
                        .find(|device| device.target() == item.device_id)
                        .map(|device| device.name.clone())
                })
                .flatten()
        };
        let groups: Vec<(Option<PullRequestGroup>, Vec<&ChangeRequestListItem>)> =
            if self.prefs.involvement == ChangeRequestInvolvement::All {
                group_items(&items, &listing.viewers)
                    .into_iter()
                    .map(|(group, items)| (Some(group), items))
                    .collect()
            } else {
                vec![(None, items)]
            };
        let mut column = div().flex().flex_col().gap(px(12.0));
        for (group, mut group_items) in groups {
            filters::sort_items(&mut group_items, self.prefs.sort, &search_text);
            column = column.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .children(group.map(|group| {
                        div()
                            .px(px(12.0))
                            .pb(px(2.0))
                            .text_size(px(12.0))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(theme.text_muted.opacity(0.7))
                            .child(group.label())
                    }))
                    .children(group_items.into_iter().map(|item| {
                        self.render_row(
                            item,
                            theme,
                            tones,
                            row_width,
                            now,
                            &search_text,
                            device_label(item),
                            cx,
                        )
                    })),
            );
        }
        if let Some(error) = shown_errors.first() {
            column = column.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.0))
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(tones.pending.opacity(0.3))
                    .bg(tones.pending.opacity(0.05))
                    .px(px(12.0))
                    .py(px(8.0))
                    .text_size(px(12.0))
                    .text_color(theme.text)
                    .child(format!(
                        "{} Showing the last pull requests loaded.",
                        failure_message(error.failure)
                    )),
            );
        }
        column = column.children(self.render_more(listing.truncated, theme, cx));
        column.into_any_element()
    }
}

fn avatar(login: &str, theme: &Theme) -> AnyElement {
    avatar_sized(login, 16.0, theme)
}

fn avatar_sized(login: &str, size: f32, theme: &Theme) -> AnyElement {
    let initial = login
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default();
    div()
        .size(px(size))
        .flex_none()
        .rounded_full()
        .bg(theme.ink(0.08))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px((size / 2.0).max(8.0).min(10.0)))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(theme.text_muted)
        .child(initial)
        .into_any_element()
}

fn label_color(color: Option<&str>, theme: &Theme) -> Hsla {
    color
        .filter(|hex| hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()))
        .and_then(|hex| u32::from_str_radix(hex, 16).ok())
        .map(|rgb| gpui::rgb(rgb).into())
        .unwrap_or(theme.text_muted)
}

fn label_pill(name: &str, color: Option<&str>, suffix: &str, theme: &Theme) -> AnyElement {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(4.0))
        .max_w(px(160.0))
        .rounded_full()
        .border_1()
        .border_color(theme.hairline(0.08))
        .bg(theme.ink(0.025))
        .pl(px(4.0))
        .pr(px(6.0))
        .text_size(px(10.0))
        .line_height(px(14.0))
        .text_color(theme.text_muted)
        .child(
            div()
                .size(px(8.0))
                .flex_none()
                .rounded_full()
                .bg(label_color(color, theme)),
        )
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .child(format!("{name}{suffix}")),
        )
        .into_any_element()
}

fn diff_stat(additions: u64, deletions: u64, tones: Tones, theme: &Theme) -> AnyElement {
    div()
        .flex()
        .items_baseline()
        .gap(px(4.0))
        .child(
            div()
                .text_color(tones.open)
                .child(format!("+{}", group_thousands(additions))),
        )
        .child(
            div()
                .text_color(theme.danger)
                .child(format!("-{}", group_thousands(deletions))),
        )
        .into_any_element()
}

pub(crate) fn group_thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

fn state_button(
    id: &'static str,
    glyph: &'static str,
    label: &'static str,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .role(gpui::Role::Button)
        .aria_label(label)
        .flex()
        .items_center()
        .gap(px(6.0))
        .h(px(28.0))
        .px(px(9.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(theme.hairline(0.09))
        .text_size(px(13.0))
        .text_color(theme.text)
        .cursor_pointer()
        .hover(|style| style.bg(theme.ink(0.06)))
        .child(icon(glyph).size(px(14.0)).text_color(theme.text))
        .child(label)
}

fn branch_mark(theme: &Theme) -> AnyElement {
    icon(icons::PR_BRANCH_MARK)
        .w(px(128.0))
        .h(px(80.0))
        .text_color(theme.text_muted.opacity(0.6))
        .into_any_element()
}

const TOAST_FOR: Duration = Duration::from_secs(4);

impl PullRequestsPage {
    /// t3code's error toasts, kept on the page because no app-wide toast surface exists.
    fn toast(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.push_toast(text.into(), None, cx);
    }

    fn toast_with(
        &mut self,
        title: impl Into<SharedString>,
        description: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.push_toast(title.into(), Some(description.into()), cx);
    }

    fn push_toast(
        &mut self,
        title: SharedString,
        description: Option<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.toast_seq += 1;
        let id = self.toast_seq;
        self.toasts.push((id, title, description));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TOAST_FOR).await;
            this.update(cx, |page, cx| {
                page.toasts.retain(|(toast, _, _)| *toast != id);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn render_toasts(&self, theme: &Theme) -> Option<AnyElement> {
        (!self.toasts.is_empty()).then(|| {
            let popup = theme.for_popup();
            let stack = div()
                .absolute()
                .right(px(16.0))
                .bottom(px(16.0))
                .flex()
                .flex_col()
                .items_end()
                .gap(px(8.0))
                .children(self.toasts.iter().map(|(id, text, description)| {
                    crate::popover::popover_card(&popup)
                        .bg(popup.surface_dialog)
                        .id(("pull-requests-toast", *id as usize))
                        .role(gpui::Role::Alert)
                        .aria_label(text.clone())
                        .max_w(px(360.0))
                        .px(px(12.0))
                        .py(px(8.0))
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .text_size(px(12.0))
                        .text_color(popup.text)
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .child(text.clone()),
                        )
                        .children(description.clone().map(|description| {
                            div().text_color(popup.text_muted).child(description)
                        }))
                }));
            gpui::deferred(stack).with_priority(3).into_any_element()
        })
    }
}

impl Render for PullRequestsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if matches!(self.listing, Loadable::Idle) {
            self.refresh(cx);
        }
        self.note_focus(window, cx);
        let theme = Theme::of(cx).clone();
        let viewport = f32::from(window.viewport_size().width);
        let panel_width = self
            .active
            .is_some()
            .then(|| self.panel_width(viewport, cx));
        let list_width = viewport - panel_width.map_or(0.0, |width| width + 1.0);
        let container = list_width.min(CONTAINER_MAX_WIDTH);
        let row_width = (container - 2.0 * CONTAINER_PAD_X).max(0.0);
        let topbar = self.render_topbar(&theme, window, cx);
        let search_busy = self.search_busy(cx);
        let controls = self.render_controls(&theme, search_busy, cx);
        let body = self.render_body(&theme, row_width, search_busy, cx);
        let list = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(topbar)
            .child(
                div()
                    .id("pull-requests-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(
                        div()
                            .mx_auto()
                            .w_full()
                            .max_w(px(CONTAINER_MAX_WIDTH))
                            .px(px(CONTAINER_PAD_X))
                            .pt(px(24.0))
                            .pb(px(48.0))
                            .flex()
                            .flex_col()
                            .gap(px(16.0))
                            .child(controls)
                            .child(body),
                    ),
            );
        let panel = panel_width.map(|width| {
            let resizer = div()
                .id("pull-requests-panel-resizer")
                .w(px(1.0))
                .h_full()
                .flex_none()
                .relative()
                .bg(if self.resize_from.is_some() {
                    theme.hairline(0.22)
                } else {
                    theme.hairline(0.07)
                })
                .child(
                    div()
                        .id("pull-requests-panel-resize-handle")
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(px(-4.0))
                        .w(px(9.0))
                        .cursor_col_resize()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |page, event: &gpui::MouseDownEvent, _, cx| {
                                page.resize_from = Some((f32::from(event.position.x), width));
                                cx.notify();
                            }),
                        ),
                );
            let panel = self.render_panel(&theme, width, window, cx);
            (resizer, panel)
        });
        div()
            .id("pull-requests-page")
            .on_mouse_move(cx.listener(|page, _: &MouseMoveEvent, _, _| {
                page.last_interaction = std::time::Instant::now();
            }))
            .on_key_down(cx.listener(|page, _: &gpui::KeyDownEvent, _, _| {
                page.last_interaction = std::time::Instant::now();
            }))
            .role(gpui::Role::Group)
            .aria_label("Pull Requests")
            .size_full()
            .flex()
            .flex_row()
            .bg(theme.bg)
            .on_mouse_move(cx.listener(move |page, event: &MouseMoveEvent, _, cx| {
                if let Some((start_x, start_width)) = page.resize_from {
                    let width = detail::clamp_panel_width(
                        start_width - (f32::from(event.position.x) - start_x),
                        viewport,
                    );
                    page.panel_width = Some(width);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|page, _, _, cx| {
                    if page.resize_from.take().is_some() {
                        if let Some(width) = page.panel_width {
                            crate::settings::update(
                                crate::settings::SavePolicy::Debounced,
                                cx,
                                |settings| settings.pull_requests_panel_width = Some(width),
                            );
                        }
                        cx.notify();
                    }
                }),
            )
            .relative()
            .child(list)
            .children(self.render_checks_popover(&theme, Tones::of(&theme), cx))
            .children(panel.map(|(resizer, panel)| {
                div()
                    .h_full()
                    .flex()
                    .flex_row()
                    .flex_none()
                    .child(resizer)
                    .child(panel)
            }))
            .children(self.render_confirm(&theme, cx))
            .children(self.render_toasts(&theme))
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use zeron_proto::ChangeRequestActor;

    use super::*;

    fn item(number: u64, author: &str) -> ChangeRequestListItem {
        ChangeRequestListItem {
            provider: "github".into(),
            host: "github.com".into(),
            repository: "acme/zeron".into(),
            number,
            title: format!("Change {number}"),
            url: format!("https://github.com/acme/zeron/pull/{number}"),
            author: Some(ChangeRequestActor {
                login: author.into(),
                avatar_url: None,
            }),
            head_ref: format!("feature/{number}"),
            base_ref: "main".into(),
            state: ChangeRequestState::Open,
            is_draft: false,
            mergeability: ChangeRequestMergeability::Mergeable,
            additions: 0,
            deletions: 0,
            created_at: Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 9, 2, 0, 0, 0).unwrap(),
            viewer_review_requested: false,
            labels: Vec::new(),
            review_decision: None,
            checks_state: None,
            device_id: None,
        }
    }

    #[test]
    fn groups_put_authored_before_review_requested_and_drop_empty_groups() {
        let mine = item(1, "Alice");
        let mut asked = item(2, "bob");
        asked.viewer_review_requested = true;
        let mut mine_and_asked = item(3, "alice");
        mine_and_asked.viewer_review_requested = true;
        let viewers = BTreeMap::from([("github.com".to_string(), "alice".to_string())]);
        let all = [&mine, &asked, &mine_and_asked];
        let groups = group_items(&all, &viewers);
        let shape: Vec<(PullRequestGroup, Vec<u64>)> = groups
            .into_iter()
            .map(|(group, items)| (group, items.iter().map(|item| item.number).collect()))
            .collect();
        assert_eq!(
            shape,
            [
                (PullRequestGroup::Authored, vec![1, 3]),
                (PullRequestGroup::ReviewRequested, vec![2]),
            ]
        );
        assert!(
            group_items(&all, &BTreeMap::new())
                .iter()
                .all(|(group, _)| *group != PullRequestGroup::Authored)
        );
    }

    #[test]
    fn merge_readiness_orders_tiers_then_measured_size_then_recency() {
        let mut ready = item(1, "a");
        ready.checks_state = Some(ChangeRequestChecksState::Passing);
        ready.review_decision = Some(ChangeRequestReviewDecision::Approved);
        let mut green = item(2, "a");
        green.checks_state = Some(ChangeRequestChecksState::Passing);
        let mut draft = item(3, "a");
        draft.is_draft = true;
        draft.checks_state = Some(ChangeRequestChecksState::Passing);
        draft.review_decision = Some(ChangeRequestReviewDecision::Approved);
        let mut merged = item(4, "a");
        merged.state = ChangeRequestState::Merged;
        let mut conflicting = item(5, "a");
        conflicting.mergeability = ChangeRequestMergeability::Conflicting;
        conflicting.checks_state = Some(ChangeRequestChecksState::Passing);
        conflicting.review_decision = Some(ChangeRequestReviewDecision::Approved);
        let mut small = item(6, "a");
        small.additions = 3;
        let mut large = item(7, "a");
        large.additions = 300;
        let mut newer_unmeasured = item(8, "a");
        newer_unmeasured.updated_at = Utc.with_ymd_and_hms(2026, 9, 9, 0, 0, 0).unwrap();

        let mut items = vec![
            &conflicting,
            &merged,
            &newer_unmeasured,
            &draft,
            &large,
            &green,
            &small,
            &ready,
        ];
        rank_by_merge_readiness(&mut items);
        let order: Vec<u64> = items.iter().map(|item| item.number).collect();
        assert_eq!(order, [1, 2, 6, 7, 8, 3, 4, 5]);
    }

    #[test]
    fn search_matches_number_title_repository_branch_and_author() {
        let row = item(42, "Carol");
        for query in [
            "#42",
            "change 42",
            "ACME/zeron",
            "feature/42",
            "carol",
            "  ",
        ] {
            assert!(matches_query(&row, query), "{query}");
        }
        assert!(!matches_query(&row, "dave"));
    }

    #[test]
    fn relative_time_matches_t3code_buckets() {
        let now = Utc.with_ymd_and_hms(2026, 10, 4, 12, 0, 0).unwrap();
        let ago = |seconds: i64| relative_time(now, now - chrono::Duration::seconds(seconds));
        assert_eq!(ago(-30), "just now");
        assert_eq!(ago(59), "just now");
        assert_eq!(ago(60), "1m ago");
        assert_eq!(ago(3599), "59m ago");
        assert_eq!(ago(3600), "1h ago");
        assert_eq!(ago(86_399), "23h ago");
        assert_eq!(ago(86_400 * 40), "40d ago");
    }

    #[test]
    fn thousands_are_grouped_like_to_locale_string() {
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(1240), "1,240");
        assert_eq!(group_thousands(1_234_567), "1,234,567");
    }
}
