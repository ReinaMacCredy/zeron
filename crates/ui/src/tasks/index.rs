use std::collections::{HashMap, HashSet};
use std::time::Instant;

use chrono::{DateTime, Local, Utc};
use gpui::{
    AnyElement, ClickEvent, Context, Entity, EventEmitter, Hsla, IntoElement, Render, ScrollHandle,
    SharedString, Subscription, Window, div, prelude::*, px,
};
use zeron_proto::{Chat, PlanProgress};

use super::strip_tree::{StripTree, StripTreeEvent, TreeHost};
use super::thread_tree::{NodeKind, TreeNode, tree_rows};
use super::tree::{self, StatsCache, Status};
use super::{plan_led_by, rollup};
use crate::icons::{self, icon};
use crate::motion::{self, Tween};
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;
use crate::typography::ui_rems;

const OPEN_LINK: f32 = 30.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Mission,
    Tree,
}

#[derive(Debug, Clone)]
pub(crate) struct IndexRow {
    pub root: String,
    pub title: String,
    pub kind: Kind,
    pub status: Status,
    pub line: String,
    pub asks: bool,
    pub steps: Option<PlanProgress>,
    pub agents: usize,
    pub nested: usize,
    pub segments: Vec<Status>,
    pub chat_ids: Vec<String>,
    pub started: DateTime<Utc>,
    pub ended: Option<DateTime<Utc>>,
    pub watched: bool,
}

#[derive(Debug, Default)]
pub(crate) struct IndexRows {
    pub running: Vec<IndexRow>,
    pub done_today: Vec<IndexRow>,
}

fn is_today(at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    at.with_timezone(&Local).date_naive() == now.with_timezone(&Local).date_naive()
}

fn roots_a_tree(
    state: &AppState,
    chat: &Chat,
    children: &HashMap<&str, Vec<&Chat>>,
    nested: &HashSet<&str>,
) -> bool {
    if chat.archived || chat.is_system() || state.is_hidden_chat(&chat.id) {
        return false;
    }
    !nested.contains(chat.id.as_str())
        && (plan_led_by(&state.tasks, &chat.id).is_some()
            || children.contains_key(chat.id.as_str()))
}

pub(crate) fn index_rows(state: &AppState, now: DateTime<Utc>) -> IndexRows {
    let children = super::thread_tree::children_by_parent(state);
    let nested: HashSet<&str> = children
        .iter()
        .flat_map(|(parent, kids)| {
            kids.iter()
                .filter(move |kid| kid.id != *parent)
                .map(|kid| kid.id.as_str())
        })
        .collect();
    let mut rows = IndexRows::default();
    for chat in &state.chats {
        if !roots_a_tree(state, chat, &children, &nested) {
            continue;
        }
        let nodes = tree_rows(state, &chat.id, false, now);
        if nodes.is_empty() {
            continue;
        }
        let task = plan_led_by(&state.tasks, &chat.id);
        let live = nodes.iter().any(|node| node.status.live());
        let last = nodes
            .iter()
            .filter_map(|node| node.chat_id.as_deref())
            .filter_map(|id| state.chats.iter().find(|chat| chat.id == id))
            .filter_map(|chat| chat.last_message_at)
            .max();
        let settled_today = chat
            .settle
            .filter(|settle| {
                settle.settled && chat.last_message_at.is_none_or(|at| at <= settle.at)
            })
            .map(|settle| settle.at);
        let done = match task {
            Some(task) => match task.completed_at {
                Some(at) if !live && is_today(at, now) => Some(at),
                Some(_) if !live => continue,
                _ => None,
            },
            None => match settled_today {
                Some(at) if is_today(at, now) => Some(at),
                Some(_) => continue,
                None if live => None,
                None => match last {
                    Some(at) if is_today(at, now) => Some(at),
                    _ => continue,
                },
            },
        };
        let row = index_row(state, chat, task, &nodes, done, now);
        if done.is_some() {
            rows.done_today.push(row);
        } else {
            rows.running.push(row);
        }
    }
    let latest = |row: &IndexRow| std::cmp::Reverse(row.ended.unwrap_or(row.started));
    rows.running.sort_by_key(latest);
    rows.done_today.sort_by_key(latest);
    rows
}

fn index_row(
    state: &AppState,
    chat: &Chat,
    task: Option<&zeron_proto::Task>,
    nodes: &[TreeNode],
    done: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> IndexRow {
    let plan = task.map(|task| rollup(state, task, now));
    let agents = nodes.iter().filter(|n| n.kind == NodeKind::Agent).count();
    let nested = nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Agent && n.depth >= 2)
        .count();
    let asker = nodes
        .iter()
        .find(|node| node.status == Status::Waiting && node.kind != NodeKind::Root);
    let root_waits = nodes[0].status == Status::Waiting;
    let working = |kind: fn(NodeKind) -> bool| {
        nodes
            .iter()
            .filter(|node| kind(node.kind) && node.status == Status::Running)
            .count()
    };
    let steps_running = working(|kind| matches!(kind, NodeKind::Step(_)));
    let agents_running = working(|kind| matches!(kind, NodeKind::Agent | NodeKind::Native));
    let line = if let Some(asker) = asker {
        format!("Needs you \u{b7} {} asks", asker.title)
    } else if root_waits {
        format!("Needs you \u{b7} {}", nodes[0].title)
    } else if task.is_some_and(|task| crate::supervisor::approval_held(state, &task.id, now)) {
        crate::supervisor::HELD_APPROVAL.into()
    } else if done.is_some() {
        "Done".into()
    } else if steps_running + agents_running > 0 {
        let mut parts = Vec::new();
        if steps_running > 0 {
            parts.push(count(steps_running, "step", "steps"));
        }
        if agents_running > 0 {
            parts.push(count(agents_running, "agent", "agents"));
        }
        format!("Watching {}", parts.join(", "))
    } else if nodes[0].status == Status::Running {
        "Working".into()
    } else {
        "Idle".into()
    };
    let status = if done.is_some() {
        Status::Done
    } else if asker.is_some() || root_waits {
        Status::Waiting
    } else if nodes[0].status == Status::Done && nodes.iter().any(|node| node.status.live()) {
        Status::Running
    } else {
        nodes[0].status
    };
    let started = task.map_or_else(
        || {
            nodes[1..]
                .iter()
                .filter_map(|node| node.chat_id.as_deref())
                .filter_map(|id| state.chats.iter().find(|chat| chat.id == id))
                .map(|chat| chat.created_at)
                .min()
                .unwrap_or(chat.created_at)
        },
        |task| task.created_at,
    );
    let watched = state.supervisor.is_some()
        && crate::supervisor::tree_watch(state, &chat.id) == crate::supervisor::Watch::Watched;
    IndexRow {
        root: chat.id.clone(),
        title: match task {
            Some(task) => task.title.clone(),
            None => crate::transcript::single_line(&crate::home::chat_title(chat)),
        },
        kind: if task.is_some() {
            Kind::Mission
        } else {
            Kind::Tree
        },
        status,
        line,
        asks: asker.is_some() || root_waits,
        steps: plan.map(|plan| plan.progress),
        agents,
        nested,
        segments: nodes[1..].iter().map(|node| node.status).collect(),
        chat_ids: nodes
            .iter()
            .filter_map(|node| node.chat_id.clone())
            .collect(),
        started,
        ended: done,
        watched,
    }
}

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Missions,
    Trees,
    NeedsYou,
}

impl Filter {
    fn keeps(self, row: &IndexRow) -> bool {
        match self {
            Self::All => true,
            Self::Missions => row.kind == Kind::Mission,
            Self::Trees => row.kind == Kind::Tree,
            Self::NeedsYou => row.asks,
        }
    }
}

pub(crate) enum MissionIndexEvent {
    NewMission,
    OpenTree(String),
    OpenTreeAt(String, super::Selection),
    Tree(StripTreeEvent),
}

pub(crate) struct MissionIndex {
    state: Entity<AppState>,
    filter: Filter,
    trees: HashMap<String, (Entity<StripTree>, Subscription)>,
    chevrons: HashMap<String, Tween>,
    stats: StatsCache,
    scroll: ScrollHandle,
    _state: Subscription,
}

impl EventEmitter<MissionIndexEvent> for MissionIndex {}

impl MissionIndex {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        Self {
            _state: cx.observe(&state, |_, _, cx| cx.notify()),
            state,
            filter: Filter::All,
            trees: HashMap::new(),
            chevrons: HashMap::new(),
            stats: StatsCache::default(),
            scroll: ScrollHandle::new(),
        }
    }

    fn toggle(&mut self, root: &str, cx: &mut Context<Self>) {
        let now = Instant::now();
        let reduced = motion::reduced_motion(cx);
        let tree = match self.trees.get(root) {
            Some((tree, _)) => tree.clone(),
            None => {
                let state = self.state.clone();
                let id = root.to_string();
                let tree =
                    cx.new(|cx| StripTree::new(state, id, false, false, TreeHost::Index, cx));
                let owner = root.to_string();
                let events = cx.subscribe(&tree, move |_, _, event: &StripTreeEvent, cx| {
                    cx.emit(match event {
                        StripTreeEvent::Open {
                            selection,
                            chat_id: None,
                        } => MissionIndexEvent::OpenTreeAt(owner.clone(), selection.clone()),
                        event => MissionIndexEvent::Tree(event.clone()),
                    })
                });
                self.trees.insert(root.to_string(), (tree.clone(), events));
                tree
            }
        };
        let open = !tree.read(cx).is_open();
        tree.update(cx, |tree, cx| tree.set_open(open, cx));
        self.chevrons
            .entry(root.to_string())
            .or_insert_with(|| Tween::at(f32::from(u8::from(!open)), now))
            .retarget(f32::from(u8::from(open)), tree::CHEVRON, reduced, now);
        cx.notify();
    }

    fn segment_color(status: Status, theme: &Theme) -> Hsla {
        match status {
            Status::Queued | Status::Stopped => theme.wash(0.12),
            _ => status.tone(theme),
        }
    }

    fn render_row(
        &mut self,
        row: &IndexRow,
        now: Instant,
        reduced: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> (AnyElement, bool) {
        let root = row.root.clone();
        let turn = self
            .chevrons
            .get(&row.root)
            .map_or(0.0, |tween| tween.value_at(now));
        let tree = self.trees.get(&row.root).map(|(tree, _)| tree.clone());
        let (band, moving) = match &tree {
            Some(tree) => tree.update(cx, |tree, cx| tree.band(cx)),
            None => (0.0, false),
        };
        let open = tree.as_ref().is_some_and(|tree| tree.read(cx).is_open());
        let tokens = {
            let state = self.state.read(cx);
            tree::amount::branch(
                None,
                row.chat_ids
                    .iter()
                    .map(|id| tree::amount::of(self.stats.get(state, id).as_ref())),
            )
        };
        let elapsed = tree::fmt_time(
            (row.ended.unwrap_or_else(Utc::now) - row.started)
                .num_milliseconds()
                .max(0) as u64,
        );
        let mut meta = vec!["Lead".to_string()];
        let lead = {
            let state = self.state.read(cx);
            state
                .chats
                .iter()
                .find(|chat| chat.id == row.root)
                .and_then(|chat| super::center::agent_of(chat, cx))
        };
        meta.extend(lead);
        if let Some(PlanProgress { done, total }) = row.steps {
            meta.push(format!("{done}/{total} steps"));
        }
        if row.agents > 0 {
            let mut agents = count(row.agents, "agent", "agents");
            if row.nested > 0 {
                agents.push_str(&format!(", {} nested", row.nested));
            }
            meta.push(agents);
        }
        let tone = row.status.tone(theme);
        let selector = format!("mission-index-row-{}", row.root);
        let head =
            div()
                .id(SharedString::from(selector.clone()))
                .debug_selector(move || selector.clone())
                .role(gpui::Role::Button)
                .aria_expanded(open)
                .aria_label(format!("{}: {}", row.title, row.line))
                .flex()
                .items_center()
                .gap(px(10.0))
                .pl(px(10.0))
                .pr(px(14.0))
                .py(px(10.0))
                .cursor_pointer()
                .hover(|s| s.bg(theme.wash(0.03)))
                .on_click(cx.listener(move |index, _: &ClickEvent, _, cx| index.toggle(&root, cx)))
                .child(
                    div()
                        .flex_none()
                        .size(px(18.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            icon(icons::LUCIDE_CHEVRON_RIGHT)
                                .size(px(13.0))
                                .text_color(theme.text_faint)
                                .with_transformation(gpui::Transformation::rotate(gpui::radians(
                                    std::f32::consts::FRAC_PI_2 * turn,
                                ))),
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .size(px(16.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(tree::glyph(row.status, 15.0, tone, reduced)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(3.0))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .min_w_0()
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(ui_rems(14.0))
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .text_color(theme.text)
                                        .child(row.title.clone()),
                                )
                                .child(tree::kind_badge(row.kind == Kind::Mission, theme)),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(ui_rems(12.0))
                                .text_color(theme.text_muted)
                                .child(meta.join(" \u{b7} ")),
                        ),
                )
                .child(div().flex_none().w(px(96.0)).flex().gap(px(2.0)).children(
                    row.segments.iter().map(|status| {
                        div()
                            .flex_1()
                            .h(px(3.0))
                            .rounded(px(2.0))
                            .bg(Self::segment_color(*status, theme))
                    }),
                ))
                .child(
                    div()
                        .flex_none()
                        .w(px(236.0))
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .overflow_hidden()
                        .text_size(ui_rems(12.0))
                        .text_color(if row.asks {
                            theme.warning
                        } else {
                            theme.text_muted
                        })
                        .children(
                            row.asks
                                .then(|| icon(icons::CLOCK_CIRCLE).size(px(13.0)).flex_none()),
                        )
                        .child(div().min_w_0().truncate().child(row.line.clone())),
                )
                .child(
                    div()
                        .flex_none()
                        .w(px(110.0))
                        .flex()
                        .justify_end()
                        .gap(px(4.0))
                        .text_size(ui_rems(12.0))
                        .text_color(theme.text_muted)
                        .children(tokens.map(|tokens| {
                            div()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(theme.text)
                                .child(crate::session_stats::tokens(tokens))
                        }))
                        .children(tokens.map(|_| div().child("\u{b7}")))
                        .child(elapsed),
                )
                .child(div().flex_none().w(px(76.0)).flex().justify_end().children(
                    row.watched.then(|| {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.0))
                            .text_size(ui_rems(11.5))
                            .text_color(theme.text_muted)
                            .child(
                                icon(icons::LUCIDE_SHIELD)
                                    .size(px(12.0))
                                    .text_color(Status::Done.tone(theme)),
                            )
                            .child("Watched")
                    }),
                ));
        let open_label = match row.kind {
            Kind::Mission => "Open mission",
            Kind::Tree => "Open tree",
        };
        let root = row.root.clone();
        let open_selector = format!("mission-index-open-{}", row.root);
        let openness = tree.as_ref().map_or(0.0, |tree| tree.read(cx).openness());
        let body = tree.filter(|_| band > 0.5).map(|tree| {
            div()
                .h(px(band + OPEN_LINK * openness))
                .overflow_hidden()
                .border_t_1()
                .border_color(theme.border)
                .child(
                    div()
                        .pt(px(6.0))
                        .pl(px(28.0))
                        .pr(px(4.0))
                        .flex()
                        .flex_col()
                        .child(tree)
                        .child(
                            div()
                                .id(SharedString::from(open_selector.clone()))
                                .debug_selector(move || open_selector.clone())
                                .role(gpui::Role::Button)
                                .aria_label(open_label)
                                .flex()
                                .items_center()
                                .gap(px(4.0))
                                .pl(px(16.0))
                                .h(px(OPEN_LINK - 6.0))
                                .cursor_pointer()
                                .text_size(ui_rems(11.5))
                                .text_color(theme.text_muted)
                                .hover(|s| s.text_color(theme.text))
                                .child(open_label)
                                .child(
                                    icon(icons::LUCIDE_CHEVRON_RIGHT)
                                        .size(px(11.0))
                                        .text_color(theme.text_muted),
                                )
                                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                                    cx.emit(MissionIndexEvent::OpenTree(root.clone()))
                                })),
                        ),
                )
        });
        let card = div()
            .flex_none()
            .rounded(px(11.0))
            .border_1()
            .border_color(theme.border)
            .bg(widgets::block_fill(theme))
            .overflow_hidden()
            .child(head)
            .children(body)
            .into_any_element();
        (card, moving)
    }

    fn group(label: &'static str, theme: &Theme) -> AnyElement {
        div()
            .mt(px(22.0))
            .mb(px(8.0))
            .ml(px(2.0))
            .text_size(ui_rems(10.5))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(theme.text_faint)
            .child(label)
            .into_any_element()
    }

    fn filter_chip(
        &self,
        filter: Filter,
        label: &'static str,
        n: usize,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let on = self.filter == filter;
        let selector = format!("mission-index-filter-{label}");
        div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector.clone())
            .role(gpui::Role::Button)
            .aria_label(format!("{label} {n}"))
            .aria_selected(on)
            .flex()
            .items_center()
            .gap(px(6.0))
            .h(px(26.0))
            .px(px(10.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(if on {
                theme.border_strong
            } else {
                theme.border
            })
            .when(on, |el| el.bg(theme.wash(0.06)))
            .cursor_pointer()
            .text_size(ui_rems(12.0))
            .text_color(if on { theme.text } else { theme.text_muted })
            .child(label)
            .child(
                div()
                    .text_color(if filter == Filter::NeedsYou && n > 0 {
                        theme.warning
                    } else {
                        theme.text_faint
                    })
                    .child(n.to_string()),
            )
            .on_click(cx.listener(move |index, _: &ClickEvent, _, cx| {
                index.filter = if index.filter == filter && filter != Filter::All {
                    Filter::All
                } else {
                    filter
                };
                cx.notify();
            }))
            .into_any_element()
    }
}

impl Render for MissionIndex {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let now = Instant::now();
        let reduced = motion::reduced_motion(cx);
        let rows = index_rows(self.state.read(cx), Utc::now());
        let all: Vec<&IndexRow> = rows.running.iter().chain(&rows.done_today).collect();
        let roots: HashSet<&str> = all.iter().map(|row| row.root.as_str()).collect();
        self.trees.retain(|root, _| roots.contains(root.as_str()));
        let ids: Vec<String> = all
            .iter()
            .flat_map(|row| row.chat_ids.iter().cloned())
            .collect();
        let state = self.state.clone();
        self.stats
            .refresh(&state, ids, |index: &mut Self| &mut index.stats, cx);
        let counts = [
            (Filter::All, "All", all.len()),
            (
                Filter::Missions,
                "Missions",
                all.iter().filter(|row| row.kind == Kind::Mission).count(),
            ),
            (
                Filter::Trees,
                "Trees",
                all.iter().filter(|row| row.kind == Kind::Tree).count(),
            ),
            (
                Filter::NeedsYou,
                "Needs you",
                all.iter().filter(|row| row.asks).count(),
            ),
        ];
        let chips: Vec<AnyElement> = counts
            .into_iter()
            .map(|(filter, label, n)| self.filter_chip(filter, label, n, &theme, cx))
            .collect();
        let watching = self.state.read(cx).supervisor.is_some().then(|| {
            let watched = rows.running.iter().filter(|row| row.watched).count();
            div()
                .debug_selector(|| "mission-index-watching".into())
                .flex_none()
                .flex()
                .items_center()
                .gap(px(5.0))
                .text_size(ui_rems(12.0))
                .text_color(theme.text_muted)
                .child(
                    icon(icons::LUCIDE_SHIELD)
                        .size(px(13.0))
                        .text_color(theme.text_muted),
                )
                .child("Supervisor")
                .child(
                    div()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(theme.text)
                        .child(format!("watching {watched}")),
                )
                .child(format!("of {} running", rows.running.len()))
        });
        let filter = self.filter;
        let mut body: Vec<AnyElement> = Vec::new();
        let mut moving = false;
        for (label, group) in [("RUNNING", &rows.running), ("DONE TODAY", &rows.done_today)] {
            let kept: Vec<&IndexRow> = group.iter().filter(|row| filter.keeps(row)).collect();
            if kept.is_empty() {
                continue;
            }
            body.push(Self::group(label, &theme));
            for row in kept {
                let (element, slides) = self.render_row(row, now, reduced, &theme, cx);
                moving |= slides;
                body.push(div().mb(px(8.0)).child(element).into_any_element());
            }
        }
        if body.is_empty() {
            body.push(
                div()
                    .debug_selector(|| "mission-index-empty".into())
                    .mt(px(40.0))
                    .text_size(ui_rems(13.0))
                    .text_color(theme.text_muted)
                    .child(if all.is_empty() {
                        "Nothing running. A chat shows here once it starts another thread or gets a plan."
                    } else {
                        "Nothing matches this filter."
                    })
                    .into_any_element(),
            );
        }
        if moving || self.chevrons.values().any(|tween| !tween.settled_at(now)) {
            window.request_animation_frame();
        }
        if all.iter().any(|row| row.status.live()) && !reduced {
            motion::pulse_lease(cx.entity_id(), cx);
        }
        div()
            .id("mission-index")
            .debug_selector(|| "mission-index".into())
            .size_full()
            .pt(px(Theme::TITLEBAR_HEIGHT))
            .child(
                div()
                    .id("mission-index-scroll")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .px(px(28.0))
                    .pt(px(22.0))
                    .pb(px(30.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .child(
                                div()
                                    .flex_none()
                                    .size(px(30.0))
                                    .rounded(px(8.0))
                                    .border_1()
                                    .border_color(theme.border)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        icon(icons::MISSION_CONTROL)
                                            .size(px(15.0))
                                            .text_color(theme.text_muted),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .text_size(ui_rems(17.0))
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(theme.text)
                                            .child("Mission Control"),
                                    )
                                    .child(
                                        div()
                                            .mt(px(2.0))
                                            .text_size(ui_rems(12.5))
                                            .text_color(theme.text_muted)
                                            .child(
                                                "Every chat that started other agents, and every mission.",
                                            ),
                                    ),
                            )
                            .child(div().flex_1())
                            .child(
                                widgets::action_button(&theme, widgets::ActionTone::Solid)
                                    .id("mission-index-new")
                                    .debug_selector(|| "mission-index-new".into())
                                    .role(gpui::Role::Button)
                                    .aria_label("New mission")
                                    .flex_none()
                                    .min_h(px(28.0))
                                    .rounded_full()
                                    .gap(px(6.0))
                                    .child(
                                        icon(icons::PLUS)
                                            .size(px(13.0))
                                            .text_color(theme.on_solid),
                                    )
                                    .child("New mission")
                                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                                        cx.emit(MissionIndexEvent::NewMission)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .mt(px(16.0))
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .children(chips)
                            .child(div().flex_1())
                            .children(watching),
                    )
                    .children(body),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(id: &str, at: DateTime<Utc>) -> Chat {
        serde_json::from_value(serde_json::json!({
            "id": id, "title": id, "deviceId": "mac", "archived": false,
            "createdAt": at, "lastMessageAt": at,
        }))
        .unwrap()
    }

    #[test]
    fn only_zeron_children_or_a_plan_root_a_row_and_a_settled_tree_is_done_today() {
        let now = Utc::now();
        let mut state = AppState::new();
        state.local_device_id = Some("mac".into());
        let mut native_only = chat("native-only", now);
        native_only.last_message_at = Some(now);
        let mut sidechat = chat("side", now);
        sidechat.sidechat_source_chat_id = Some("side-source".into());
        let mut fork = chat("fork", now);
        fork.forked_from_chat_id = Some("fork-source".into());
        fork.created_by_chat_id = Some("fork-source".into());
        let mut child = chat("child", now);
        child.created_by_chat_id = Some("parent".into());
        let mut grandchild = chat("grandchild", now);
        grandchild.created_by_chat_id = Some("child".into());
        let mut settled = chat("settled", now - chrono::Duration::minutes(5));
        settled.settle = Some(zeron_proto::SidebarSettle {
            settled: true,
            at: now,
        });
        let mut settled_child = chat("settled-child", now - chrono::Duration::minutes(5));
        settled_child.created_by_chat_id = Some("settled".into());
        state.chats = vec![
            native_only,
            chat("side-source", now),
            sidechat,
            chat("fork-source", now),
            fork,
            chat("parent", now),
            child,
            grandchild,
            settled,
            settled_child,
        ];
        state.sessions = vec![
            serde_json::from_value(serde_json::json!({
                "chatId": "native-only", "deviceId": "mac", "status": "idle",
                "runningSubagents": 2, "updatedAt": now,
            }))
            .unwrap(),
        ];
        let rows = index_rows(&state, now);
        let ids = |rows: &[IndexRow]| -> Vec<String> {
            rows.iter().map(|row| row.root.clone()).collect()
        };
        let all: Vec<String> = ids(&rows.running)
            .into_iter()
            .chain(ids(&rows.done_today))
            .collect();
        assert!(
            !all.contains(&"native-only".to_string()),
            "native sub-agents alone never root a row"
        );
        assert!(
            !all.contains(&"side-source".to_string()),
            "a sidechat is no child"
        );
        assert!(
            !all.contains(&"fork-source".to_string()),
            "a fork is no child"
        );
        assert!(
            !all.contains(&"child".to_string()),
            "a child sits in its parent's tree, not its own row"
        );
        assert!(all.contains(&"parent".to_string()));
        let mut done = ids(&rows.done_today);
        done.sort();
        assert_eq!(done, ["parent", "settled"].map(String::from));
        let parent = rows
            .done_today
            .iter()
            .find(|row| row.root == "parent")
            .unwrap();
        assert_eq!((parent.agents, parent.nested), (2, 1));
        assert_eq!(parent.kind, Kind::Tree);
    }

    #[test]
    fn a_finished_plan_whose_lead_still_runs_an_agent_stays_running() {
        let now = Utc::now();
        let mut state = AppState::new();
        state.local_device_id = Some("mac".into());
        let mut agent = chat("agent", now);
        agent.created_by_chat_id = Some("lead".into());
        state.chats = vec![chat("lead", now), agent, chat("quiet-lead", now)];
        let task = |id: &str, lead: &str, done: DateTime<Utc>| -> zeron_proto::Task {
            serde_json::from_value(serde_json::json!({
                "id": id, "title": id, "orderKey": id, "chatId": lead,
                "mode": "orchestrated", "completedAt": done,
            }))
            .unwrap()
        };
        state.tasks = vec![
            task("old-plan", "lead", now - chrono::Duration::days(3)),
            task("today-plan", "quiet-lead", now),
        ];
        state.sessions = vec![
            serde_json::from_value(serde_json::json!({
                "chatId": "agent", "deviceId": "mac", "status": "working", "updatedAt": now,
            }))
            .unwrap(),
        ];
        let rows = index_rows(&state, now);
        let running: Vec<&IndexRow> = rows.running.iter().collect();
        assert_eq!(
            running.len(),
            1,
            "{:?}",
            running.iter().map(|r| &r.root).collect::<Vec<_>>()
        );
        assert_eq!(running[0].root, "lead");
        assert_eq!(running[0].status, Status::Running);
        let done: Vec<&str> = rows
            .done_today
            .iter()
            .map(|row| row.root.as_str())
            .collect();
        assert_eq!(done, ["quiet-lead"]);
    }

    #[test]
    fn only_a_local_live_main_chat_without_an_open_plan_can_add_one() {
        let now = Utc::now();
        let mut state = AppState::new();
        state.local_device_id = Some("mac".into());
        let mut remote = chat("remote", now);
        remote.device_id = "pc".into();
        let mut archived = chat("archived", now);
        archived.archived = true;
        let mut side = chat("side", now);
        side.sidechat_source_chat_id = Some("plain".into());
        state.chats = vec![
            chat("plain", now),
            remote,
            archived,
            side,
            chat("planned", now),
            chat("was-planned", now),
        ];
        let task = |id: &str, lead: &str, done: Option<DateTime<Utc>>| -> zeron_proto::Task {
            serde_json::from_value(serde_json::json!({
                "id": id, "title": id, "orderKey": id, "chatId": lead,
                "mode": "orchestrated", "completedAt": done,
            }))
            .unwrap()
        };
        state.tasks = vec![
            task("open", "planned", None),
            task("done", "was-planned", Some(now)),
        ];
        let can: Vec<&str> = state
            .chats
            .iter()
            .filter(|chat| crate::tasks::can_add_plan(&state, chat))
            .map(|chat| chat.id.as_str())
            .collect();
        assert_eq!(can, ["plain", "was-planned"]);
    }
}
