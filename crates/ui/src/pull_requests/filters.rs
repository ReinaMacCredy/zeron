use gpui::{AnyElement, Bounds, Context, Pixels, SharedString, div, prelude::*, px};
use serde::{Deserialize, Serialize};
use zeron_proto::{
    ChangeRequestChecksState, ChangeRequestDraftFilter, ChangeRequestInvolvement,
    ChangeRequestListFilters, ChangeRequestListItem, ChangeRequestListState, ChangeRequestListing,
    ChangeRequestReviewFilter, ChangeRequestState, ListChangeRequestsRequest,
};

use super::{PageTab, PullRequestsPage, avatar, change_size, label_color, rank_by_merge_readiness};
use crate::icons::{self, icon};
use crate::popover;
use crate::theme::Theme;

const MAX_QUALIFIER_VALUES: usize = 10;
const MAX_QUALIFIER_LENGTH: usize = 200;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ListSort {
    #[default]
    Ready,
    Updated,
    Newest,
    Oldest,
    Largest,
    Smallest,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PullRequestListPrefs {
    pub state: ChangeRequestListState,
    pub involvement: ChangeRequestInvolvement,
    pub sort: ListSort,
    pub filters: ChangeRequestListFilters,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    pub tab: PageTab,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_space_id: Option<String>,
}

fn qualifier_value(raw: &str) -> String {
    raw.replace('"', "").trim().to_string()
}

fn clip_qualifier(value: &str) -> String {
    value
        .chars()
        .take(MAX_QUALIFIER_LENGTH)
        .collect::<String>()
        .trim()
        .to_string()
}

fn split_qualifier_list(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed.len() >= 2
        && trimmed.starts_with('"')
        && trimmed.ends_with('"')
        && !trimmed[1..trimmed.len() - 1].contains('"')
    {
        let whole = qualifier_value(trimmed);
        return if whole.is_empty() {
            Vec::new()
        } else {
            vec![whole]
        };
    }
    raw.split(',')
        .map(qualifier_value)
        .filter(|part| !part.is_empty())
        .collect()
}

fn bounded(names: Vec<String>) -> Vec<String> {
    names
        .iter()
        .take(MAX_QUALIFIER_VALUES)
        .map(|name| clip_qualifier(name))
        .filter(|name| !name.is_empty())
        .collect()
}

fn query_tokens(raw: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else if c == '"' {
            if chars.clone().any(|next| next == '"') {
                current.push('"');
                current.extend(chars.by_ref().take_while(|&next| next != '"'));
                current.push('"');
            }
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn push_labels(filters: &mut ChangeRequestListFilters, negated: bool, names: Vec<String>) -> bool {
    if names.is_empty() {
        return false;
    }
    if negated {
        filters.excluded_labels.extend(names);
    } else {
        filters.labels.push(names);
    }
    true
}

pub(crate) fn parse_query(raw: &str) -> (String, ChangeRequestListFilters) {
    let mut text = Vec::new();
    let mut filters = ChangeRequestListFilters::default();
    for token in query_tokens(raw) {
        let qualifier = token.split_once(':').and_then(|(key, value)| {
            let (negated, key) = match key.strip_prefix('-') {
                Some(key) => (true, key),
                None => (false, key),
            };
            let valid_key = key.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                && key
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            valid_key.then(|| (negated, key.to_ascii_lowercase(), value.to_string()))
        });
        let Some((negated, key, raw_value)) = qualifier else {
            text.push(token);
            continue;
        };
        let value = qualifier_value(&raw_value);
        let lower = value.to_ascii_lowercase();
        let consumed = !value.is_empty()
            && match key.as_str() {
                "author" if !negated => {
                    filters.author = Some(clip_qualifier(&value));
                    true
                }
                "draft" if !negated => {
                    let draft = match lower.as_str() {
                        "true" => Some(ChangeRequestDraftFilter::Only),
                        "false" => Some(ChangeRequestDraftFilter::Hide),
                        _ => None,
                    };
                    filters.draft = draft.or(filters.draft);
                    draft.is_some()
                }
                "review" if !negated => {
                    let review = match lower.as_str() {
                        "approved" => Some(ChangeRequestReviewFilter::Approved),
                        "changes_requested" | "changes-requested" => {
                            Some(ChangeRequestReviewFilter::ChangesRequested)
                        }
                        "required" | "review-required" => {
                            Some(ChangeRequestReviewFilter::ReviewRequired)
                        }
                        "none" => Some(ChangeRequestReviewFilter::None),
                        _ => None,
                    };
                    filters.review = review.or(filters.review);
                    review.is_some()
                }
                "status" | "checks" if !negated => {
                    let checks = match lower.as_str() {
                        "success" | "passing" => Some(ChangeRequestChecksState::Passing),
                        "failure" | "failing" => Some(ChangeRequestChecksState::Failing),
                        _ => None,
                    };
                    filters.checks = checks.or(filters.checks);
                    checks.is_some()
                }
                "repo" if !negated => {
                    filters.repository = Some(clip_qualifier(&value));
                    true
                }
                "author" | "draft" | "review" | "status" | "checks" | "repo" => false,
                "label" => push_labels(
                    &mut filters,
                    negated,
                    bounded(split_qualifier_list(&raw_value)),
                ),
                _ if value.starts_with('/') => false,
                _ => {
                    let names = split_qualifier_list(&raw_value)
                        .into_iter()
                        .map(|name| {
                            if name.contains(':') {
                                name
                            } else {
                                format!("{key}:{name}")
                            }
                        })
                        .collect();
                    push_labels(&mut filters, negated, bounded(names))
                }
            };
        if !consumed {
            text.push(token);
        }
    }
    filters.labels.truncate(MAX_QUALIFIER_VALUES);
    filters.excluded_labels.truncate(MAX_QUALIFIER_VALUES);
    (text.join(" "), filters)
}

pub(crate) fn effective_filters(
    prefs: &PullRequestListPrefs,
    typed: &ChangeRequestListFilters,
) -> ChangeRequestListFilters {
    let menu = &prefs.filters;
    ChangeRequestListFilters {
        draft: typed.draft.or(menu.draft),
        review: typed.review.or(menu.review),
        checks: typed.checks.or(menu.checks),
        author: typed.author.clone().or_else(|| menu.author.clone()),
        labels: if typed.labels.is_empty() {
            menu.labels.clone()
        } else {
            typed.labels.clone()
        },
        excluded_labels: if typed.excluded_labels.is_empty() {
            menu.excluded_labels.clone()
        } else {
            typed.excluded_labels.clone()
        },
        repository: typed
            .repository
            .clone()
            .or_else(|| prefs.repository.clone()),
    }
}

pub(crate) fn list_request(
    prefs: &PullRequestListPrefs,
    text: &str,
    typed: &ChangeRequestListFilters,
) -> ListChangeRequestsRequest {
    ListChangeRequestsRequest {
        state: prefs.state,
        involvement: prefs.involvement,
        limit: None,
        query: (!text.trim().is_empty()).then(|| text.trim().to_string()),
        filters: effective_filters(prefs, typed),
    }
}

pub(crate) fn project_options(
    listing: Option<&ChangeRequestListing>,
    current: Option<&str>,
) -> Vec<String> {
    let mut options: Vec<String> = Vec::new();
    let known = listing.into_iter().flat_map(|listing| {
        listing
            .repositories
            .iter()
            .chain(listing.items.iter().map(|item| &item.repository))
    });
    for repository in current.into_iter().chain(known.map(String::as_str)) {
        if !options
            .iter()
            .any(|option| option.eq_ignore_ascii_case(repository))
        {
            options.push(repository.to_string());
        }
    }
    options.sort_by_key(|option| option.to_ascii_lowercase());
    options
}

pub(crate) fn score_match(item: &ChangeRequestListItem, query: &str) -> u32 {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return 0;
    }
    let number = needle.trim_start_matches('#');
    if !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()) {
        return if item.number.to_string() == number {
            100
        } else {
            0
        };
    }
    let title = item.title.to_lowercase();
    let terms: Vec<&str> = needle.split_whitespace().collect();
    if title == needle {
        90
    } else if title.contains(&needle) {
        80
    } else if terms.len() > 1 && terms.iter().all(|term| title.contains(term)) {
        70
    } else if item.head_ref.to_lowercase().contains(&needle) {
        60
    } else if item
        .author
        .as_ref()
        .is_some_and(|author| author.login.to_lowercase().contains(&needle))
    {
        50
    } else if item.repository.to_lowercase().contains(&needle) {
        40
    } else if terms.iter().any(|term| title.contains(term)) {
        30
    } else {
        10
    }
}

pub(crate) fn sort_items(items: &mut Vec<&ChangeRequestListItem>, sort: ListSort, search: &str) {
    use std::cmp::{Ordering, Reverse};
    if !search.trim().is_empty() {
        items.sort_by_cached_key(|item| {
            (Reverse(score_match(item, search)), Reverse(item.updated_at))
        });
        return;
    }
    if sort == ListSort::Ready {
        rank_by_merge_readiness(items);
        return;
    }
    let primary = |left: &ChangeRequestListItem, right: &ChangeRequestListItem| match sort {
        ListSort::Newest => right.created_at.cmp(&left.created_at),
        ListSort::Oldest => left.created_at.cmp(&right.created_at),
        ListSort::Largest | ListSort::Smallest => {
            let measured = (change_size(right) > 0).cmp(&(change_size(left) > 0));
            let sized = if sort == ListSort::Largest {
                change_size(right).cmp(&change_size(left))
            } else {
                change_size(left).cmp(&change_size(right))
            };
            measured.then(sized)
        }
        ListSort::Ready | ListSort::Updated => Ordering::Equal,
    };
    items.sort_by(|left, right| {
        primary(left, right).then_with(|| right.updated_at.cmp(&left.updated_at))
    });
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuthorFacet {
    pub login: String,
    pub merged: usize,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LabelFacet {
    pub name: String,
    pub color: Option<String>,
    pub count: usize,
}

pub(crate) fn facets(
    items: &[ChangeRequestListItem],
    state: ChangeRequestListState,
) -> (Vec<AuthorFacet>, Vec<LabelFacet>) {
    let mut authors: Vec<AuthorFacet> = Vec::new();
    let mut labels: Vec<LabelFacet> = Vec::new();
    for item in items {
        let in_state = match state {
            ChangeRequestListState::All => true,
            ChangeRequestListState::Open => item.state == ChangeRequestState::Open,
            ChangeRequestListState::Closed => item.state == ChangeRequestState::Closed,
            ChangeRequestListState::Merged => item.state == ChangeRequestState::Merged,
        };
        if let Some(author) = &item.author {
            let merged = usize::from(item.state == ChangeRequestState::Merged);
            match authors
                .iter_mut()
                .find(|known| known.login.eq_ignore_ascii_case(&author.login))
            {
                Some(known) => {
                    known.count += usize::from(in_state);
                    known.merged += merged;
                }
                None => authors.push(AuthorFacet {
                    login: author.login.clone(),
                    merged,
                    count: usize::from(in_state),
                }),
            }
        }
        if !in_state {
            continue;
        }
        for label in &item.labels {
            match labels
                .iter_mut()
                .find(|known| known.name.eq_ignore_ascii_case(&label.name))
            {
                Some(known) => known.count += 1,
                None => labels.push(LabelFacet {
                    name: label.name.clone(),
                    color: label.color.clone(),
                    count: 1,
                }),
            }
        }
    }
    authors.retain(|author| author.count > 0);
    authors.sort_by(|left, right| {
        right
            .merged
            .cmp(&left.merged)
            .then(right.count.cmp(&left.count))
            .then_with(|| left.login.cmp(&right.login))
    });
    labels.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.name.cmp(&right.name))
    });
    (authors, labels)
}

pub(crate) fn filter_count(prefs: &PullRequestListPrefs) -> usize {
    usize::from(prefs.state != ChangeRequestListState::Open)
        + usize::from(prefs.involvement != ChangeRequestInvolvement::All)
        + usize::from(prefs.repository.is_some())
        + usize::from(prefs.host.is_some())
        + usize::from(prefs.device.is_some())
        + usize::from(prefs.filters.draft.is_some())
        + usize::from(prefs.filters.review.is_some())
        + usize::from(prefs.filters.checks.is_some())
        + usize::from(prefs.filters.author.is_some())
        + prefs.filters.labels.iter().map(Vec::len).sum::<usize>()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MenuKind {
    Sort,
    Filters,
    State,
    Involvement,
    Project,
    GitProject,
    GitCheckout,
    GitPush,
    GitPull,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FilterSection {
    State,
    Involvement,
    Author,
    Labels,
    Draft,
    Review,
    Checks,
    Project,
    Host,
    Device,
}

pub(super) struct ListMenu {
    kind: MenuKind,
    submenu: Option<FilterSection>,
    submenu_bounds: Option<Bounds<Pixels>>,
}

type Options<T> = [(T, &'static str, &'static str)];

const SORTS: [(ListSort, &str, &str); 6] = [
    (
        ListSort::Ready,
        "Merge readiness",
        icons::LUCIDE_LIST_CHECKS,
    ),
    (ListSort::Updated, "Recently updated", icons::LUCIDE_CLOCK),
    (
        ListSort::Newest,
        "Newest shown",
        icons::LUCIDE_CALENDAR_ARROW_DOWN,
    ),
    (
        ListSort::Oldest,
        "Oldest shown",
        icons::LUCIDE_CALENDAR_ARROW_UP,
    ),
    (ListSort::Largest, "Largest shown", icons::LUCIDE_MAXIMIZE_2),
    (
        ListSort::Smallest,
        "Smallest shown",
        icons::LUCIDE_MINIMIZE_2,
    ),
];

const STATES: [(ChangeRequestListState, &str, &str); 4] = [
    (ChangeRequestListState::All, "All", icons::LUCIDE_LAYERS),
    (
        ChangeRequestListState::Open,
        "Open",
        icons::LUCIDE_GIT_PULL_REQUEST,
    ),
    (
        ChangeRequestListState::Closed,
        "Closed",
        icons::LUCIDE_GIT_PULL_REQUEST_CLOSED,
    ),
    (
        ChangeRequestListState::Merged,
        "Merged",
        icons::LUCIDE_GIT_MERGE,
    ),
];

const INVOLVEMENTS: [(ChangeRequestInvolvement, &str, &str); 3] = [
    (ChangeRequestInvolvement::All, "All", icons::LUCIDE_LAYERS),
    (
        ChangeRequestInvolvement::Reviewing,
        "Reviewing",
        icons::LUCIDE_EYE,
    ),
    (
        ChangeRequestInvolvement::Authored,
        "Authored",
        icons::LUCIDE_PEN_LINE,
    ),
];

const DRAFTS: [(Option<ChangeRequestDraftFilter>, &str, &str); 3] = [
    (None, "All", icons::LUCIDE_LAYERS),
    (
        Some(ChangeRequestDraftFilter::Only),
        "Drafts only",
        icons::LUCIDE_GIT_PULL_REQUEST_DRAFT,
    ),
    (
        Some(ChangeRequestDraftFilter::Hide),
        "Hide drafts",
        icons::LUCIDE_EYE_OFF,
    ),
];

const REVIEWS: [(Option<ChangeRequestReviewFilter>, &str, &str); 5] = [
    (None, "All", icons::LUCIDE_LAYERS),
    (
        Some(ChangeRequestReviewFilter::Approved),
        "Approved",
        icons::LUCIDE_CIRCLE_CHECK,
    ),
    (
        Some(ChangeRequestReviewFilter::ChangesRequested),
        "Changes requested",
        icons::LUCIDE_CIRCLE_X,
    ),
    (
        Some(ChangeRequestReviewFilter::ReviewRequired),
        "Review required",
        icons::LUCIDE_CIRCLE_DASHED,
    ),
    (
        Some(ChangeRequestReviewFilter::None),
        "No reviews",
        icons::LUCIDE_CIRCLE_SLASH,
    ),
];

const CHECKS: [(Option<ChangeRequestChecksState>, &str, &str); 3] = [
    (None, "All", icons::LUCIDE_LAYERS),
    (
        Some(ChangeRequestChecksState::Passing),
        "Passing",
        icons::LUCIDE_CIRCLE_CHECK,
    ),
    (
        Some(ChangeRequestChecksState::Failing),
        "Failing",
        icons::LUCIDE_CIRCLE_X,
    ),
];

fn label_of<T: PartialEq + Copy>(options: &Options<T>, value: T) -> &'static str {
    options
        .iter()
        .find(|(option, _, _)| *option == value)
        .map(|(_, label, _)| *label)
        .unwrap_or("All")
}

pub(crate) fn host_options(
    items: &[ChangeRequestListItem],
    errors: &[zeron_proto::ChangeRequestListError],
) -> Vec<(String, bool)> {
    let read: std::collections::BTreeSet<&str> =
        items.iter().map(|item| item.host.as_str()).collect();
    let mut hosts: Vec<(String, bool)> = read.iter().map(|host| (host.to_string(), true)).collect();
    for host in errors.iter().filter_map(|error| error.host.as_deref()) {
        if !hosts.iter().any(|(known, _)| known == host) {
            hosts.push((host.to_string(), false));
        }
    }
    hosts.sort();
    hosts
}

pub(super) fn state_label(state: ChangeRequestListState) -> &'static str {
    label_of(&STATES, state)
}

pub(super) fn involvement_label(involvement: ChangeRequestInvolvement) -> &'static str {
    label_of(&INVOLVEMENTS, involvement)
}

fn row(theme: &Theme, active: bool, id: impl Into<SharedString>) -> gpui::Stateful<gpui::Div> {
    let id = id.into();
    popover::menu_row(theme, active, id.clone()).id(id)
}

fn radio_dot(selected: bool, theme: &Theme) -> AnyElement {
    div()
        .ml_auto()
        .size(px(14.0))
        .flex()
        .items_center()
        .justify_center()
        .when(selected, |el| {
            el.child(div().size(px(6.0)).rounded_full().bg(theme.text))
        })
        .into_any_element()
}

fn section_of(kind: MenuKind) -> Option<FilterSection> {
    match kind {
        MenuKind::State => Some(FilterSection::State),
        MenuKind::Involvement => Some(FilterSection::Involvement),
        MenuKind::Project => Some(FilterSection::Project),
        MenuKind::Sort
        | MenuKind::Filters
        | MenuKind::GitProject
        | MenuKind::GitCheckout
        | MenuKind::GitPush
        | MenuKind::GitPull => None,
    }
}

impl PullRequestsPage {
    pub(super) fn menu_trigger(
        &self,
        kind: MenuKind,
        trigger: gpui::Stateful<gpui::Div>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        self.menu_trigger_with(kind, trigger, |_, _| {}, theme, cx)
    }

    pub(super) fn menu_trigger_with(
        &self,
        kind: MenuKind,
        trigger: gpui::Stateful<gpui::Div>,
        on_open: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let trigger = trigger
            .relative()
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |page, _, _, _| {
                    page.menu
                        .note_trigger_press_matching(|menu| menu.kind == kind)
                }),
            )
            .on_click(cx.listener(move |page, _, _, cx| {
                if page.menu.take_press_was_open() {
                    page.close_menu(cx);
                } else {
                    page.menu.open(ListMenu {
                        kind,
                        submenu: None,
                        submenu_bounds: None,
                    });
                    on_open(page, cx);
                    cx.notify();
                }
            }));
        if self.menu.get().map(|menu| menu.kind) != Some(kind) {
            return trigger;
        }
        let closing = self.menu.closing_since();
        let content = self.render_menu(kind, theme, cx);
        let id = format!("pull-requests-menu-{kind:?}");
        trigger.child(if kind == MenuKind::Filters {
            popover::anchored_menu_below_end(id, content, closing)
        } else {
            popover::anchored_menu_below(id, content, closing)
        })
    }

    pub(super) fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu.begin_close() {
            popover::reap_popup(cx, |page: &mut Self| &mut page.menu);
            cx.notify();
        }
    }

    pub(super) fn update_prefs(
        &mut self,
        refetch: bool,
        cx: &mut Context<Self>,
        apply: impl FnOnce(&mut PullRequestListPrefs),
    ) {
        let before = self.prefs.clone();
        apply(&mut self.prefs);
        if self.prefs.repository != before.repository {
            self.follow_repository_into_git();
        }
        if self.prefs == before {
            return;
        }
        let prefs = self.prefs.clone();
        crate::settings::update(crate::settings::SavePolicy::Debounced, cx, |settings| {
            settings.pull_requests_list = prefs;
        });
        if refetch {
            self.refresh(cx);
        }
        cx.notify();
    }

    pub(super) fn choose(
        &mut self,
        refetch: bool,
        cx: &mut Context<Self>,
        apply: impl FnOnce(&mut PullRequestListPrefs),
    ) {
        self.close_menu(cx);
        self.update_prefs(refetch, cx, apply);
    }

    #[allow(clippy::too_many_arguments)]
    fn radio_menu<T: PartialEq + Clone + 'static>(
        &self,
        id: &'static str,
        options: Vec<(T, SharedString, &'static str)>,
        current: &T,
        refetch: bool,
        apply: fn(&mut PullRequestListPrefs, T),
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        div().flex().flex_col().gap(px(popover::MENU_GAP)).children(
            options
                .into_iter()
                .enumerate()
                .map(|(index, (value, label, glyph))| {
                    let selected = value == *current;
                    let selector = format!("{id}-{index}");
                    row(theme, false, selector.clone())
                        .debug_selector(move || selector.clone())
                        .role(gpui::Role::MenuItemRadio)
                        .aria_label(label.clone())
                        .on_click(cx.listener(move |page, _, _, cx| {
                            let value = value.clone();
                            page.choose(refetch, cx, |prefs| apply(prefs, value));
                        }))
                        .child(icon(glyph).size(px(14.0)).text_color(theme.text_muted))
                        .child(label)
                        .child(radio_dot(selected, theme))
                }),
        )
    }

    fn table_menu<T: PartialEq + Copy + 'static>(
        &self,
        id: &'static str,
        options: &Options<T>,
        current: T,
        apply: fn(&mut PullRequestListPrefs, T),
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let options = options
            .iter()
            .map(|(value, label, glyph)| (*value, SharedString::from(*label), *glyph))
            .collect();
        self.radio_menu(id, options, &current, true, apply, theme, cx)
            .into_any_element()
    }

    fn render_menu(&self, kind: MenuKind, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let theme = &theme.for_popup();
        let body: AnyElement = match (kind, section_of(kind)) {
            (MenuKind::Sort, _) => {
                let options = SORTS
                    .iter()
                    .map(|(sort, label, glyph)| (*sort, SharedString::from(*label), *glyph))
                    .collect();
                self.radio_menu(
                    "pull-requests-sort",
                    options,
                    &self.prefs.sort,
                    false,
                    |prefs, sort| prefs.sort = sort,
                    theme,
                    cx,
                )
                .min_w(px(180.0))
                .into_any_element()
            }
            (MenuKind::GitProject, _) => self.render_git_project_menu(theme, cx),
            (MenuKind::GitCheckout, _) => self.render_git_checkout_menu(theme, cx),
            (MenuKind::GitPush | MenuKind::GitPull, _) => {
                self.render_git_sync_menu(kind, theme, cx)
            }
            (_, Some(section)) => div()
                .min_w(px(160.0))
                .child(self.render_filter_section(section, theme, cx))
                .into_any_element(),
            _ => self.render_filter_rows(theme, cx),
        };
        popover::popover_card(theme)
            .flex()
            .flex_col()
            .on_mouse_down_out(cx.listener(|page, event: &gpui::MouseDownEvent, _, cx| {
                let in_submenu = page
                    .menu
                    .get()
                    .and_then(|menu| menu.submenu_bounds)
                    .is_some_and(|bounds| bounds.contains(&event.position));
                if !in_submenu {
                    page.close_menu(cx);
                }
            }))
            .child(body)
            .into_any_element()
    }

    fn render_filter_rows(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let filters = &self.prefs.filters;
        let labels_value = match filters.labels.iter().map(Vec::len).sum::<usize>() {
            0 => "Any".to_string(),
            count => format!("{count} selected"),
        };
        let mut rows: Vec<(FilterSection, &str, &str, String)> = vec![
            (
                FilterSection::State,
                icons::LUCIDE_GIT_PULL_REQUEST,
                "State",
                state_label(self.prefs.state).into(),
            ),
            (
                FilterSection::Involvement,
                icons::LUCIDE_LAYERS,
                "Involvement",
                involvement_label(self.prefs.involvement).into(),
            ),
            (
                FilterSection::Author,
                icons::LUCIDE_USER_ROUND,
                "Author",
                filters.author.clone().unwrap_or_else(|| "Anyone".into()),
            ),
            (
                FilterSection::Labels,
                icons::LUCIDE_TAG,
                "Labels",
                labels_value,
            ),
            (
                FilterSection::Draft,
                icons::LUCIDE_GIT_PULL_REQUEST_DRAFT,
                "Draft",
                label_of(&DRAFTS, filters.draft).into(),
            ),
            (
                FilterSection::Review,
                icons::LUCIDE_CIRCLE_CHECK,
                "Review",
                label_of(&REVIEWS, filters.review).into(),
            ),
            (
                FilterSection::Checks,
                icons::LUCIDE_CIRCLE_DOT,
                "Checks",
                label_of(&CHECKS, filters.checks).into(),
            ),
            (
                FilterSection::Project,
                icons::LUCIDE_FOLDER,
                "Project",
                self.prefs
                    .repository
                    .clone()
                    .unwrap_or_else(|| "All projects".into()),
            ),
        ];
        if self.host_choices().len() >= 2 || self.prefs.host.is_some() {
            rows.push((
                FilterSection::Host,
                icons::LUCIDE_GIT_PULL_REQUEST,
                "Host",
                self.prefs
                    .host
                    .clone()
                    .unwrap_or_else(|| "All hosts".into()),
            ));
        }
        let devices = self.list_devices(cx);
        if devices.len() >= 2 || self.prefs.device.is_some() {
            rows.push((
                FilterSection::Device,
                icons::LAPTOP,
                "Device",
                self.prefs
                    .device
                    .as_ref()
                    .and_then(|id| devices.iter().find(|device| device.id == *id))
                    .map(|device| device.name.clone())
                    .unwrap_or_else(|| "All devices".into()),
            ));
        }
        let open = self.menu.get().and_then(|menu| menu.submenu);
        let mut card = div()
            .w(px(224.0))
            .flex()
            .flex_col()
            .gap(px(popover::MENU_GAP));
        for (section, glyph, label, value) in rows {
            if matches!(
                section,
                FilterSection::Author
                    | FilterSection::Project
                    | FilterSection::Host
                    | FilterSection::Device
            ) {
                card = card.child(popover::menu_separator());
            }
            let mut entry = div()
                .id(SharedString::from(format!(
                    "pull-requests-filter-hover-{label}"
                )))
                .relative()
                .on_hover(cx.listener(move |page, hovered: &bool, _, cx| {
                    if *hovered
                        && let Some(menu) = page.menu.open_mut()
                        && menu.submenu != Some(section)
                    {
                        menu.submenu = Some(section);
                        menu.submenu_bounds = None;
                        cx.notify();
                    }
                }));
            let trigger = row(
                theme,
                open == Some(section),
                format!("pull-requests-filter-{label}"),
            )
            .role(gpui::Role::MenuItem)
            .aria_label(label)
            .child(icon(glyph).size(px(14.0)).text_color(theme.text_muted))
            .child(div().flex_1().child(label))
            .child(
                div()
                    .max_w(px(128.0))
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_size(px(12.0))
                    .text_color(theme.text_muted)
                    .child(value),
            )
            .child(
                icon(icons::LUCIDE_CHEVRON_RIGHT)
                    .size(px(12.0))
                    .text_color(theme.text_muted),
            );
            entry = entry.child(trigger);
            if open == Some(section) {
                let entity = cx.entity().downgrade();
                let submenu = popover::popover_card(theme)
                    .relative()
                    .min_w(px(224.0))
                    .flex()
                    .flex_col()
                    .child(self.render_filter_section(section, theme, cx))
                    .child(
                        gpui::canvas(
                            move |bounds, _, cx| {
                                let _ = entity.update(cx, |page, _| {
                                    if let Some(menu) = page.menu.open_mut() {
                                        menu.submenu_bounds = Some(bounds);
                                    }
                                });
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    );
                entry = entry.child(popover::nested_menu(
                    format!("pull-requests-submenu-{label}"),
                    submenu.into_any_element(),
                    true,
                ));
            }
            card = card.child(entry);
        }
        card.into_any_element()
    }

    fn render_filter_section(
        &self,
        section: FilterSection,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let filters = &self.prefs.filters;
        match section {
            FilterSection::State => self.table_menu(
                "pull-requests-state",
                &STATES,
                self.prefs.state,
                |prefs, state| prefs.state = state,
                theme,
                cx,
            ),
            FilterSection::Involvement => self.table_menu(
                "pull-requests-involvement",
                &INVOLVEMENTS,
                self.prefs.involvement,
                |prefs, involvement| prefs.involvement = involvement,
                theme,
                cx,
            ),
            FilterSection::Draft => self.table_menu(
                "pull-requests-draft",
                &DRAFTS,
                filters.draft,
                |prefs, draft| prefs.filters.draft = draft,
                theme,
                cx,
            ),
            FilterSection::Review => self.table_menu(
                "pull-requests-review",
                &REVIEWS,
                filters.review,
                |prefs, review| prefs.filters.review = review,
                theme,
                cx,
            ),
            FilterSection::Checks => self.table_menu(
                "pull-requests-checks",
                &CHECKS,
                filters.checks,
                |prefs, checks| prefs.filters.checks = checks,
                theme,
                cx,
            ),
            FilterSection::Author => self.render_author_menu(theme, cx),
            FilterSection::Labels => self.render_labels_menu(theme, cx),
            FilterSection::Project => self.render_project_menu(theme, cx),
            FilterSection::Host => self.render_host_menu(theme, cx),
            FilterSection::Device => self.render_device_menu(theme, cx),
        }
    }

    fn host_choices(&self) -> Vec<(String, bool)> {
        match &self.listing {
            crate::popover::Loadable::Ready(listing) => {
                host_options(&listing.items, &listing.errors)
            }
            _ => Vec::new(),
        }
    }

    fn render_host_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let current = self.prefs.host.clone();
        let choices = std::iter::once((None, "All hosts".to_string(), true)).chain(
            self.host_choices()
                .into_iter()
                .map(|(host, available)| (Some(host.clone()), host, available)),
        );
        div()
            .flex()
            .flex_col()
            .gap(px(popover::MENU_GAP))
            .children(
                choices
                    .enumerate()
                    .map(|(index, (value, label, available))| {
                        let selected = value == current;
                        let glyph = if value.is_none() {
                            icons::LUCIDE_LAYERS
                        } else {
                            icons::LUCIDE_GIT_PULL_REQUEST
                        };
                        let text = if available {
                            label.clone()
                        } else {
                            format!("{label} · Unavailable")
                        };
                        row(theme, false, format!("pull-requests-host-{index}"))
                            .role(gpui::Role::MenuItemRadio)
                            .aria_label(SharedString::from(text.clone()))
                            .when(!available, |el| {
                                el.opacity(0.5).cursor_default().tooltip(
                                    crate::settings::widgets::text_tooltip(
                                        "This host could not be read.",
                                    ),
                                )
                            })
                            .when(available, |el| {
                                el.on_click(cx.listener(move |page, _, _, cx| {
                                    let value = value.clone();
                                    page.choose(false, cx, |prefs| prefs.host = value);
                                }))
                            })
                            .child(icon(glyph).size(px(14.0)).text_color(theme.text_muted))
                            .child(text)
                            .child(radio_dot(selected, theme))
                    }),
            )
            .into_any_element()
    }

    fn render_device_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let options = std::iter::once((
            None,
            SharedString::from("All devices"),
            icons::LUCIDE_LAYERS,
        ))
        .chain(self.list_devices(cx).into_iter().map(|device| {
            (
                Some(device.id),
                SharedString::from(device.name),
                icons::LAPTOP,
            )
        }))
        .collect();
        self.radio_menu(
            "pull-requests-device",
            options,
            &self.prefs.device,
            true,
            |prefs, device| prefs.device = device,
            theme,
            cx,
        )
        .into_any_element()
    }

    fn listed_items(&self) -> &[ChangeRequestListItem] {
        self.listing
            .ready()
            .map_or(&[][..], |listing| &listing.items)
    }

    fn render_author_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let (authors, _) = facets(self.listed_items(), self.prefs.state);
        let query = self.author_search.read(cx).text().to_string();
        let current = self.prefs.filters.author.clone();
        let mut shown: Vec<AuthorFacet> = authors
            .into_iter()
            .filter(|author| popover::match_rank(&query, &author.login).is_some())
            .collect();
        if let Some(selected) = &current
            && let Some(index) = shown
                .iter()
                .position(|author| author.login.eq_ignore_ascii_case(selected))
        {
            let selected = shown.remove(index);
            shown.insert(0, selected);
        }
        shown.truncate(10);
        let empty = shown.is_empty();
        div()
            .w(px(320.0))
            .flex()
            .flex_col()
            .gap(px(popover::MENU_GAP))
            .child(popover::search_input_frame(
                theme,
                self.author_search.clone().into_any_element(),
            ))
            .child(
                row(theme, false, "pull-requests-author-anyone")
                    .role(gpui::Role::MenuItemRadio)
                    .aria_label("Anyone")
                    .on_click(cx.listener(|page, _, _, cx| {
                        page.choose(true, cx, |prefs| prefs.filters.author = None);
                    }))
                    .child(
                        icon(icons::LUCIDE_LAYERS)
                            .size(px(14.0))
                            .text_color(theme.text_muted),
                    )
                    .child("Anyone")
                    .child(radio_dot(current.is_none(), theme)),
            )
            .children(shown.into_iter().enumerate().map(|(index, author)| {
                let login = author.login.clone();
                let selected = current
                    .as_deref()
                    .is_some_and(|selected| selected.eq_ignore_ascii_case(&author.login));
                row(theme, selected, format!("pull-requests-author-{index}"))
                    .role(gpui::Role::MenuItemRadio)
                    .aria_label(SharedString::from(author.login.clone()))
                    .on_click(cx.listener(move |page, _, _, cx| {
                        let login = login.clone();
                        page.choose(true, cx, |prefs| prefs.filters.author = Some(login));
                    }))
                    .child(avatar(&author.login, theme))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(author.login),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(theme.text_muted)
                            .child(format!("{} merges loaded", author.merged)),
                    )
            }))
            .when(empty, |el| {
                el.child(
                    div()
                        .px(px(8.0))
                        .py(px(6.0))
                        .text_size(px(12.0))
                        .text_color(theme.text_muted)
                        .child("No authors found"),
                )
            })
            .into_any_element()
    }

    fn render_labels_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let (_, mut labels) = facets(self.listed_items(), self.prefs.state);
        let selected: Vec<String> = self
            .prefs
            .filters
            .labels
            .iter()
            .flatten()
            .cloned()
            .collect();
        for name in selected.iter().rev() {
            if !labels
                .iter()
                .any(|label| label.name.eq_ignore_ascii_case(name))
            {
                labels.insert(
                    0,
                    LabelFacet {
                        name: name.clone(),
                        color: None,
                        count: 0,
                    },
                );
            }
        }
        if labels.is_empty() {
            return div()
                .w(px(288.0))
                .px(px(8.0))
                .py(px(6.0))
                .text_size(px(12.0))
                .text_color(theme.text_muted)
                .child("No labels in this view")
                .into_any_element();
        }
        div()
            .w(px(288.0))
            .flex()
            .flex_col()
            .gap(px(popover::MENU_GAP))
            .children(labels.into_iter().enumerate().map(|(index, label)| {
                let checked = selected
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(&label.name));
                let name = label.name.clone();
                row(theme, false, format!("pull-requests-label-{index}"))
                    .role(gpui::Role::MenuItemCheckBox)
                    .aria_label(SharedString::from(label.name.clone()))
                    .on_click(cx.listener(move |page, _, _, cx| {
                        let name = name.clone();
                        page.update_prefs(true, cx, |prefs| {
                            let mut held: Vec<String> =
                                prefs.filters.labels.iter().flatten().cloned().collect();
                            match held
                                .iter()
                                .position(|label| label.eq_ignore_ascii_case(&name))
                            {
                                Some(index) => {
                                    held.remove(index);
                                }
                                None if held.len() < MAX_QUALIFIER_VALUES => held.push(name),
                                None => {}
                            }
                            prefs.filters.labels =
                                held.into_iter().map(|label| vec![label]).collect();
                        });
                    }))
                    .child(
                        div()
                            .size(px(14.0))
                            .flex_none()
                            .rounded(px(4.0))
                            .border_1()
                            .border_color(theme.hairline(0.2))
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(checked, |el| {
                                el.child(
                                    icon(icons::LUCIDE_CHECK)
                                        .size(px(10.0))
                                        .text_color(theme.text),
                                )
                            }),
                    )
                    .child(
                        div()
                            .size(px(10.0))
                            .flex_none()
                            .rounded_full()
                            .bg(label_color(label.color.as_deref(), theme)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(label.name),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(theme.text_muted)
                            .child(label.count.to_string()),
                    )
            }))
            .into_any_element()
    }

    fn render_project_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let repositories = project_options(self.listing.ready(), self.prefs.repository.as_deref());
        let options = std::iter::once((
            None,
            SharedString::from("All projects"),
            icons::LUCIDE_LAYERS,
        ))
        .chain(repositories.into_iter().map(|repository| {
            (
                Some(repository.clone()),
                SharedString::from(repository),
                icons::LUCIDE_FOLDER,
            )
        }))
        .collect();
        self.radio_menu(
            "pull-requests-project",
            options,
            &self.prefs.repository,
            true,
            |prefs, repository| prefs.repository = repository,
            theme,
            cx,
        )
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use zeron_proto::{ChangeRequestActor, ChangeRequestLabel, ChangeRequestMergeability};

    use super::*;

    fn item(number: u64, title: &str, author: &str) -> ChangeRequestListItem {
        ChangeRequestListItem {
            provider: "github".into(),
            host: "github.com".into(),
            repository: "acme/zeron".into(),
            number,
            title: title.into(),
            url: String::new(),
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
    fn query_grammar_matches_t3code() {
        let (text, filters) = parse_query(
            r#"fix login label:bug,"needs triage" label:"needs design" -label:wontfix author:@me draft:true review:changes_requested status:success size:S,XS -area:web https://x.dev/a status:maybe "size:XXL""#,
        );
        assert_eq!(text, r#"fix login https://x.dev/a status:maybe "size:XXL""#);
        assert_eq!(
            filters.labels,
            [
                vec!["bug".to_string(), "needs triage".to_string()],
                vec!["needs design".to_string()],
                vec!["size:S".to_string(), "size:XS".to_string()],
            ]
        );
        assert_eq!(
            filters.excluded_labels,
            ["wontfix".to_string(), "area:web".to_string()]
        );
        assert_eq!(filters.author.as_deref(), Some("@me"));
        assert_eq!(filters.draft, Some(ChangeRequestDraftFilter::Only));
        assert_eq!(
            filters.review,
            Some(ChangeRequestReviewFilter::ChangesRequested)
        );
        assert_eq!(filters.checks, Some(ChangeRequestChecksState::Passing));
        let (_, quoted) = parse_query(r#"label:"a,b" draft:maybe -author:x"#);
        assert_eq!(quoted.labels, [vec!["a,b".to_string()]]);
        assert_eq!(quoted.draft, None);
        assert_eq!(quoted.author, None);
    }

    #[test]
    fn typed_qualifiers_override_the_same_menu_filter() {
        let prefs = PullRequestListPrefs {
            filters: ChangeRequestListFilters {
                draft: Some(ChangeRequestDraftFilter::Hide),
                author: Some("bob".into()),
                labels: vec![vec!["ui".into()]],
                ..Default::default()
            },
            ..Default::default()
        };
        let typed = ChangeRequestListFilters {
            author: Some("carol".into()),
            ..Default::default()
        };
        let merged = effective_filters(&prefs, &typed);
        assert_eq!(merged.author.as_deref(), Some("carol"));
        assert_eq!(merged.draft, Some(ChangeRequestDraftFilter::Hide));
        assert_eq!(merged.labels, [vec!["ui".to_string()]]);
    }

    #[test]
    fn a_repo_qualifier_is_the_project_filter_not_a_label() {
        let (text, filters) = parse_query("fix repo:ReinaMacCredy/maestro label:bug");
        assert_eq!(text, "fix");
        assert_eq!(filters.repository.as_deref(), Some("ReinaMacCredy/maestro"));
        assert_eq!(filters.labels, [vec!["bug".to_string()]]);
        let (text, negated) = parse_query("-repo:acme/zeron repo:");
        assert_eq!(text, "-repo:acme/zeron repo:");
        assert_eq!(negated, ChangeRequestListFilters::default());
    }

    #[test]
    fn the_list_request_carries_the_project_and_a_typed_repo_wins() {
        let prefs = PullRequestListPrefs {
            repository: Some("ReinaMacCredy/maestro".into()),
            ..Default::default()
        };
        let (text, typed) = parse_query("");
        let request = list_request(&prefs, &text, &typed);
        assert_eq!(request.query, None);
        assert_eq!(
            serde_json::to_value(&request).unwrap()["filters"],
            serde_json::json!({"repository": "ReinaMacCredy/maestro"})
        );
        let (text, typed) = parse_query(" login repo:getpaseo/paseo ");
        let request = list_request(&prefs, &text, &typed);
        assert_eq!(request.query.as_deref(), Some("login"));
        assert_eq!(
            request.filters.repository.as_deref(),
            Some("getpaseo/paseo")
        );
        let request = list_request(&PullRequestListPrefs::default(), "", &typed);
        assert_eq!(
            request.filters.repository.as_deref(),
            Some("getpaseo/paseo")
        );
    }

    #[test]
    fn the_project_pref_persists_under_its_own_key() {
        let prefs = PullRequestListPrefs {
            repository: Some("ReinaMacCredy/maestro".into()),
            ..Default::default()
        };
        let json = serde_json::to_value(&prefs).unwrap();
        assert_eq!(json["repository"], "ReinaMacCredy/maestro");
        assert!(json["filters"].get("repository").is_none());
        let back: PullRequestListPrefs = serde_json::from_value(json).unwrap();
        assert_eq!(back, prefs);
        let older: PullRequestListPrefs =
            serde_json::from_value(serde_json::json!({"repository": "acme/zeron"})).unwrap();
        assert_eq!(older.repository.as_deref(), Some("acme/zeron"));
    }

    #[test]
    fn the_page_tab_and_git_project_persist_and_default_for_older_prefs() {
        let prefs = PullRequestListPrefs {
            tab: PageTab::Git,
            git_space_id: Some("space-1".into()),
            ..Default::default()
        };
        let json = serde_json::to_value(&prefs).unwrap();
        assert_eq!(json["tab"], "git");
        assert_eq!(json["gitSpaceId"], "space-1");
        let back: PullRequestListPrefs = serde_json::from_value(json).unwrap();
        assert_eq!(back, prefs);
        let older: PullRequestListPrefs =
            serde_json::from_value(serde_json::json!({"repository": "acme/zeron"})).unwrap();
        assert_eq!(older.tab, PageTab::PullRequests);
        assert_eq!(older.git_space_id, None);
        assert_eq!(filter_count(&prefs), 0);
    }

    #[test]
    fn project_options_list_every_known_project_once_and_keep_the_pick() {
        let mut paseo = item(1, "a", "x");
        paseo.repository = "getpaseo/paseo".into();
        let listing = ChangeRequestListing {
            items: vec![paseo, item(2, "b", "x")],
            repositories: vec![
                "ReinaMacCredy/maestro".into(),
                "GetPaseo/Paseo".into(),
                "acme/zeron".into(),
            ],
            ..Default::default()
        };
        assert_eq!(
            project_options(Some(&listing), None),
            ["acme/zeron", "GetPaseo/Paseo", "ReinaMacCredy/maestro"]
        );
        assert_eq!(project_options(None, Some("old/project")), ["old/project"]);
        let older: ChangeRequestListing = serde_json::from_value(serde_json::json!({
            "viewers": {}, "items": [], "errors": [], "truncated": false
        }))
        .unwrap();
        assert!(older.repositories.is_empty());
    }

    #[test]
    fn relevance_scores_follow_t3code_tiers() {
        let row = item(42, "Welcome wizard polish", "Carol");
        assert_eq!(score_match(&row, "#42"), 100);
        assert_eq!(score_match(&row, "43"), 0);
        assert_eq!(score_match(&row, "welcome wizard polish"), 90);
        assert_eq!(score_match(&row, "wizard"), 80);
        assert_eq!(score_match(&row, "polish welcome"), 70);
        assert_eq!(score_match(&row, "feature/4"), 60);
        assert_eq!(score_match(&row, "caro"), 50);
        assert_eq!(score_match(&row, "acme"), 40);
        assert_eq!(score_match(&row, "wizard nope"), 30);
        assert_eq!(score_match(&row, "body text"), 10);
    }

    #[test]
    fn hosts_list_every_read_host_and_flag_only_hosts_that_never_answered() {
        let mut ghe = item(2, "b", "x");
        ghe.host = "ghe.acme.io".into();
        let error = |host: &str| zeron_proto::ChangeRequestListError {
            host: Some(host.into()),
            repository: None,
            path: "/repo".into(),
            failure: zeron_proto::ChangeRequestFailure::Authentication,
        };
        let options = host_options(
            &[item(1, "a", "x"), ghe],
            &[error("github.com"), error("gitlab.acme.io")],
        );
        assert_eq!(
            options,
            [
                ("ghe.acme.io".to_string(), true),
                ("github.com".to_string(), true),
                ("gitlab.acme.io".to_string(), false),
            ]
        );
        let mut prefs = PullRequestListPrefs::default();
        prefs.host = Some("github.com".into());
        prefs.device = Some("studio".into());
        assert_eq!(filter_count(&prefs), 2);
    }

    #[test]
    fn size_sorts_put_measured_rows_first() {
        let mut small = item(1, "a", "x");
        small.additions = 3;
        let mut large = item(2, "b", "x");
        large.additions = 300;
        let unmeasured = item(3, "c", "x");
        let mut rows = vec![&unmeasured, &small, &large];
        sort_items(&mut rows, ListSort::Largest, "");
        assert_eq!(
            rows.iter().map(|row| row.number).collect::<Vec<_>>(),
            [2, 1, 3]
        );
        sort_items(&mut rows, ListSort::Smallest, "");
        assert_eq!(
            rows.iter().map(|row| row.number).collect::<Vec<_>>(),
            [1, 2, 3]
        );
    }

    #[test]
    fn facets_rank_authors_by_merges_then_count() {
        let mut merged = item(1, "a", "bob");
        merged.state = ChangeRequestState::Merged;
        let mut labeled = item(2, "b", "carol");
        labeled.labels = vec![ChangeRequestLabel {
            name: "bug".into(),
            color: Some("d73a4a".into()),
        }];
        let other = item(3, "c", "carol");
        let (authors, labels) = facets(
            &[merged.clone(), labeled, other],
            ChangeRequestListState::All,
        );
        assert_eq!(
            authors
                .iter()
                .map(|author| (author.login.as_str(), author.merged, author.count))
                .collect::<Vec<_>>(),
            [("bob", 1, 1), ("carol", 0, 2)]
        );
        assert_eq!(labels.len(), 1);
        let (open_authors, _) = facets(&[merged], ChangeRequestListState::Open);
        assert!(open_authors.is_empty());
    }

    #[test]
    fn filter_count_matches_the_menu_pill() {
        let mut prefs = PullRequestListPrefs::default();
        assert_eq!(filter_count(&prefs), 0);
        prefs.state = ChangeRequestListState::All;
        prefs.filters.labels = vec![vec!["a".into()], vec!["b".into()]];
        prefs.filters.draft = Some(ChangeRequestDraftFilter::Only);
        assert_eq!(filter_count(&prefs), 4);
    }
}
