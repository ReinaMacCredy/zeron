use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, Hsla, IntoElement, Render, SharedString,
    Subscription, Window, div, prelude::*, px,
};

use super::center::TaskCenter;
use super::tree::{self, Dot, Link, NodeIn, StatsCache, Status, TreeMotion};
use super::{Selection, chat_brand, linked_chat, rollup, step_chat};
use crate::icons::{self, icon};
use crate::motion::{self, Tween};
use crate::state::AppState;
use crate::theme::Theme;
use crate::typography::ui_rems;

const ROW: f32 = 26.0;
const GAP: f32 = 4.0;
const INDENT: f32 = 22.0;
const ANCHOR: (f32, f32) = (13.0, 13.0);
const PAD_X: f32 = 10.0;
const PAD_BOTTOM: f32 = 10.0;
const LEAD: &str = "lead";

pub(crate) enum StripTreeEvent {
    Open(Selection),
    Brief(Selection),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TreeHost {
    Strip,
    Supervisor,
    Intro,
}

pub(crate) struct SampleRow {
    pub title: &'static str,
    pub status: Status,
    pub word: &'static str,
    pub brand: Option<(&'static str, Option<Hsla>)>,
    pub secs: Option<i64>,
}

enum Source {
    Task(String),
    Sample(Vec<SampleRow>, DateTime<Utc>),
}

struct Row {
    id: String,
    parent: Option<usize>,
    depth: usize,
    title: String,
    status: Status,
    word: &'static str,
    brand: Option<(&'static str, Option<Hsla>)>,
    chat_id: Option<String>,
    since: Option<DateTime<Utc>>,
    selection: Selection,
}

pub(crate) struct StripTree {
    state: Entity<AppState>,
    source: Source,
    folded: HashSet<String>,
    chevrons: HashMap<String, Tween>,
    open: Tween,
    motion: TreeMotion,
    stats: StatsCache,
    since: Instant,
    tick: Option<gpui::Task<()>>,
    host: TreeHost,
    _state: Subscription,
}

impl EventEmitter<StripTreeEvent> for StripTree {}

impl StripTree {
    pub fn new(
        state: Entity<AppState>,
        task_id: String,
        open: bool,
        host: TreeHost,
        cx: &mut Context<Self>,
    ) -> Self {
        let now = Instant::now();
        Self {
            _state: cx.observe(&state, |_, _, cx| cx.notify()),
            state,
            source: Source::Task(task_id),
            folded: HashSet::new(),
            chevrons: HashMap::new(),
            open: Tween::at(f32::from(u8::from(open)), now),
            motion: TreeMotion::default(),
            stats: StatsCache::default(),
            since: now,
            tick: None,
            host,
        }
    }

    pub fn sample(state: Entity<AppState>, rows: Vec<SampleRow>, cx: &mut Context<Self>) -> Self {
        let mut tree = Self::new(state, String::new(), true, TreeHost::Intro, cx);
        tree.source = Source::Sample(rows, Utc::now());
        tree
    }

    pub fn task_id(&self) -> &str {
        match &self.source {
            Source::Task(task_id) => task_id,
            Source::Sample(..) => "",
        }
    }

    pub fn is_open(&self) -> bool {
        self.open.target() > 0.5
    }

    pub fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        let target = f32::from(u8::from(open));
        self.open.retarget(
            target,
            tree::FOLD,
            motion::reduced_motion(cx),
            Instant::now(),
        );
        cx.notify();
    }

    fn rows(&self, cx: &App) -> Option<Vec<Row>> {
        let task_id = match &self.source {
            Source::Task(task_id) => task_id,
            Source::Sample(rows, at) => return Some(sample_rows(rows, *at)),
        };
        let state = self.state.read(cx);
        let task = state
            .tasks
            .iter()
            .find(|task| task.id == *task_id && !task.deleted)?;
        let plan = rollup(state, task, Utc::now());
        let since = |chat: &zeron_proto::Chat| {
            state
                .session_for(&chat.id)
                .and_then(|session| session.started_at)
                .or(chat.delegation.as_ref().map(|d| d.started_at))
        };
        let lead = linked_chat(&state.chats, task);
        let (status, word) = tree::lead_status(task, &plan);
        let mut rows = vec![Row {
            id: LEAD.into(),
            parent: None,
            depth: 0,
            title: format!("Lead · {}", TaskCenter::lead_line(task, &plan)),
            status,
            word,
            brand: lead.map(chat_brand),
            chat_id: lead.map(|chat| chat.id.clone()),
            since: lead.and_then(since),
            selection: Selection::Lead,
        }];
        rows.extend(plan.steps.iter().map(|row| {
            let chat = step_chat(&state.chats, &row.step);
            let delegation = chat.and_then(|chat| chat.delegation.as_ref().map(|d| d.status));
            let (status, word) = Status::of_step(row, delegation);
            Row {
                id: row.step.id.clone(),
                parent: Some(0),
                depth: 1,
                title: crate::transcript::single_line(&row.step.title),
                status,
                word,
                brand: row.brand,
                chat_id: chat.map(|chat| chat.id.clone()),
                since: chat.and_then(since),
                selection: Selection::Step(row.step.id.clone()),
            }
        }));
        Some(rows)
    }

    fn visible(&self, rows: &[Row], ix: usize) -> bool {
        let mut parent = rows[ix].parent;
        while let Some(up) = parent {
            if self.folded.contains(&rows[up].id) {
                return false;
            }
            parent = rows[up].parent;
        }
        true
    }

    fn sync(&mut self, rows: &[Row], reduced: bool, now: Instant) {
        let nodes: Vec<NodeIn> = (0..rows.len())
            .map(|ix| NodeIn {
                id: &rows[ix].id,
                root: rows[ix].parent.is_none(),
                visible: self.visible(rows, ix),
                status: rows[ix].status,
            })
            .collect();
        self.motion.sync(&nodes, reduced, now);
    }

    fn content_height(&self, rows: &[Row]) -> f32 {
        let shown = rows.iter().filter(|row| self.motion.shown(&row.id)).count();
        ROW + shown.saturating_sub(1) as f32 * (ROW + GAP) + PAD_BOTTOM
    }

    pub fn band(&mut self, cx: &mut Context<Self>) -> (f32, bool) {
        let now = Instant::now();
        let Some(rows) = self.rows(cx) else {
            return (0.0, false);
        };
        self.sync(&rows, motion::reduced_motion(cx), now);
        (
            self.content_height(&rows) * self.open.value_at(now),
            !self.open.settled_at(now),
        )
    }

    fn toggle_fold(&mut self, id: &str, cx: &mut Context<Self>) {
        let folded = !self.folded.remove(id);
        if folded {
            self.folded.insert(id.to_string());
        }
        let now = Instant::now();
        let reduced = motion::reduced_motion(cx);
        self.chevrons
            .entry(id.to_string())
            .or_insert_with(|| Tween::at(f32::from(u8::from(!folded)), now))
            .retarget(f32::from(u8::from(folded)), tree::CHEVRON, reduced, now);
        cx.notify();
    }

    fn keep_ticking(&mut self, cx: &mut Context<Self>) {
        if self.tick.is_some() {
            return;
        }
        self.tick = Some(cx.spawn(async move |tree, cx| {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            tree.update(cx, |tree, cx| {
                tree.tick = None;
                cx.notify();
            })
            .ok();
        }));
    }
}

impl Render for StripTree {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        let reduced = motion::reduced_motion(cx);
        let theme = match self.host {
            TreeHost::Strip | TreeHost::Intro => Theme::of(cx).for_popup(),
            TreeHost::Supervisor => Theme::of(cx).clone(),
        };
        let Some(rows) = self.rows(cx) else {
            return div().into_any_element();
        };
        self.sync(&rows, reduced, now);
        let chat_ids: Vec<String> = rows.iter().filter_map(|row| row.chat_id.clone()).collect();
        let state = self.state.clone();
        self.stats
            .refresh(&state, chat_ids, |tree: &mut Self| &mut tree.stats, cx);
        let times: Vec<Option<String>> = {
            let state = state.read(cx);
            rows.iter()
                .map(|row| {
                    let stats = row
                        .chat_id
                        .as_deref()
                        .and_then(|id| self.stats.get(state, id));
                    tree::thread_time(row.status, row.since, stats.as_ref())
                })
                .collect()
        };
        let mut placed: Vec<Option<(f32, f32, tree::Frame)>> = Vec::with_capacity(rows.len());
        let mut k = 0;
        for row in &rows {
            if !self.motion.shown(&row.id) {
                placed.push(None);
                continue;
            }
            let layout = k as f32 * (ROW + GAP);
            k += 1;
            let (_, glide) = self
                .motion
                .place(&row.id, (0.0, layout), (0.0, 0.0), reduced, now);
            let frame = self.motion.frame(&row.id, now);
            let left = PAD_X + row.depth as f32 * INDENT;
            placed.push(Some((left, layout + glide + frame.y, frame)));
        }
        let mut links = Vec::new();
        let mut dots: Vec<(usize, Dot)> = Vec::new();
        for (ix, row) in rows.iter().enumerate() {
            let (Some(parent), Some((left, top, frame))) = (row.parent, &placed[ix]) else {
                continue;
            };
            let Some((parent_left, parent_top, _)) = &placed[parent] else {
                continue;
            };
            let anchor = (parent_left + ANCHOR.0, parent_top + ROW);
            let stroke = self.motion.tint(&row.id, now, |status| {
                tree::link_color(status.muted(), &theme)
            });
            links.push(Link {
                from: anchor,
                to: (*left, top + ANCHOR.1),
                drawn: frame.drawn,
                color: stroke,
            });
            match dots.iter_mut().find(|(owner, _)| *owner == parent) {
                Some((_, dot)) => {
                    if !row.status.muted() {
                        dot.stroke = tree::link_color(false, &theme);
                    }
                    dot.opacity = if frame.leaving {
                        dot.opacity.max(frame.fade)
                    } else {
                        1.0
                    };
                }
                None => dots.push((
                    parent,
                    Dot {
                        at: anchor,
                        stroke: tree::link_color(row.status.muted(), &theme),
                        fill: Hsla {
                            a: 1.0,
                            ..crate::popover::surface_bg(&theme)
                        },
                        opacity: if frame.leaving { frame.fade } else { 1.0 },
                    },
                )),
            }
        }
        let mut sweeping = false;
        let mut elements = Vec::new();
        for (ix, row) in rows.iter().enumerate() {
            let Some((left, top, frame)) = &placed[ix] else {
                continue;
            };
            let running = row.status == Status::Running;
            let title = if running && !reduced {
                sweeping = true;
                crate::transcript::shimmer_label(
                    SharedString::from(row.title.clone()),
                    tree::title_phase(self.since, now),
                    theme.font_sans.clone(),
                    ui_rems(12.5).into(),
                    None,
                    theme.text_muted,
                    theme.text,
                )
                .flex_1()
                .whitespace_nowrap()
                .text_color(theme.text_muted)
                .into_any_element()
            } else {
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(if running || row.status.muted() {
                        theme.text_muted
                    } else {
                        theme.text
                    })
                    .child(row.title.clone())
                    .into_any_element()
            };
            let group = SharedString::from(format!("strip-tree-row-{}", row.id));
            let kids = rows.iter().filter(|other| other.parent == Some(ix)).count();
            let fold = (kids > 0).then(|| {
                let folded = self.folded.contains(&row.id);
                let hidden = (0..rows.len())
                    .filter(|other| {
                        let mut up = rows[*other].parent;
                        while let Some(parent) = up {
                            if parent == ix {
                                return true;
                            }
                            up = rows[parent].parent;
                        }
                        false
                    })
                    .count();
                let turn = self
                    .chevrons
                    .get(&row.id)
                    .map_or(f32::from(u8::from(folded)), |tween| tween.value_at(now));
                let id = row.id.clone();
                let selector = format!("strip-tree-fold-{}", row.id);
                div()
                    .id(SharedString::from(selector.clone()))
                    .debug_selector(move || selector.clone())
                    .role(gpui::Role::Button)
                    .aria_expanded(!folded)
                    .aria_label(if folded {
                        format!("Show {hidden} under {}", row.title)
                    } else {
                        format!("Fold {}", row.title)
                    })
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(2.0))
                    .h(px(20.0))
                    .pl(px(6.0))
                    .pr(px(4.0))
                    .rounded(px(5.0))
                    .text_size(ui_rems(11.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text_muted)
                    .hover(|s| s.bg(theme.wash(0.08)).text_color(theme.text))
                    .child(if folded {
                        format!("+{hidden}")
                    } else {
                        kids.to_string()
                    })
                    .child(
                        icon(icons::LUCIDE_CHEVRON_DOWN)
                            .size(px(12.0))
                            .text_color(theme.text_muted)
                            .with_transformation(gpui::Transformation::rotate(gpui::radians(
                                -std::f32::consts::FRAC_PI_2 * turn,
                            ))),
                    )
                    .on_click(cx.listener(move |tree, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        tree.toggle_fold(&id, cx);
                    }))
            });
            let brief = (self.host == TreeHost::Supervisor).then(|| {
                let selection = row.selection.clone();
                let selector = format!("strip-tree-brief-{}", row.id);
                div()
                    .id(SharedString::from(selector.clone()))
                    .debug_selector(move || selector.clone())
                    .role(gpui::Role::Button)
                    .aria_label(format!("Brief for {}", row.title))
                    .flex_none()
                    .cursor_pointer()
                    .text_color(theme.accent)
                    .opacity(0.0)
                    .group_hover(group.clone(), |s| s.opacity(1.0))
                    .child("Brief ›")
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        cx.emit(StripTreeEvent::Brief(selection.clone()))
                    }))
            });
            let selection = row.selection.clone();
            let selector = format!("strip-tree-row-{}", row.id);
            let opens = self.host != TreeHost::Intro;
            elements.push(
                div()
                    .id(SharedString::from(selector.clone()))
                    .debug_selector(move || selector.clone())
                    .group(group)
                    .role(if opens {
                        gpui::Role::Button
                    } else {
                        gpui::Role::ListItem
                    })
                    .aria_label(format!("{}: {}", row.title, row.word))
                    .absolute()
                    .top(px(*top))
                    .left(px(*left))
                    .right(px(PAD_X))
                    .h(px(ROW))
                    .opacity(frame.opacity)
                    .flex()
                    .items_center()
                    .gap(px(7.0))
                    .pl(px(6.0))
                    .pr(px(8.0))
                    .rounded(px(6.0))
                    .when(opens, |el| {
                        el.cursor_pointer().hover(|s| s.bg(theme.wash(0.05)))
                    })
                    .text_size(ui_rems(11.5))
                    .text_color(theme.text_muted)
                    .child(
                        div()
                            .flex_none()
                            .size(px(14.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(tree::glyph(
                                row.status,
                                12.0,
                                row.status.tone(&theme),
                                reduced,
                            )),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .text_size(ui_rems(12.5))
                            .child(title),
                    )
                    .children(row.brand.map(|(mark, tint)| {
                        icon(mark)
                            .size(px(12.0))
                            .flex_none()
                            .text_color(tint.unwrap_or(theme.text_muted))
                    }))
                    .child(
                        div()
                            .flex_none()
                            .text_color(if row.status.muted() {
                                theme.text_muted
                            } else {
                                row.status.tone(&theme)
                            })
                            .child(row.word),
                    )
                    .children(times[ix].clone().map(|time| div().flex_none().child(time)))
                    .children(fold)
                    .children(brief)
                    .when(opens, |el| {
                        el.on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.emit(StripTreeEvent::Open(selection.clone()))
                        }))
                    }),
            );
        }
        if sweeping {
            motion::pulse_lease(cx.entity_id(), cx);
        }
        if rows.iter().any(|row| row.status.live()) {
            self.keep_ticking(cx);
        }
        if self.motion.animating(now) || self.chevrons.values().any(|t| !t.settled_at(now)) {
            window.request_animation_frame();
        }
        div()
            .id("strip-tree")
            .debug_selector(|| "strip-tree".into())
            .relative()
            .w_full()
            .h(px(self.content_height(&rows)))
            .children(elements)
            .child(tree::links(
                links,
                dots.into_iter().map(|(_, dot)| dot).collect(),
            ))
            .into_any_element()
    }
}

fn sample_rows(rows: &[SampleRow], at: DateTime<Utc>) -> Vec<Row> {
    rows.iter()
        .enumerate()
        .map(|(ix, row)| Row {
            id: if ix == 0 { LEAD.into() } else { ix.to_string() },
            parent: (ix > 0).then_some(0),
            depth: usize::from(ix > 0),
            title: row.title.into(),
            status: row.status,
            word: row.word,
            brand: row.brand,
            chat_id: None,
            since: row.secs.map(|secs| at - chrono::Duration::seconds(secs)),
            selection: if ix == 0 {
                Selection::Lead
            } else {
                Selection::Step(ix.to_string())
            },
        })
        .collect()
}
