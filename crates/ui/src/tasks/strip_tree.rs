use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, Hsla, IntoElement, MouseButton, MouseDownEvent,
    Pixels, Point, Render, SharedString, Subscription, Window, div, prelude::*, px,
};

use super::Selection;
use super::thread_tree::{self, NodeKind};
use super::tree::{self, Dot, Link, NodeIn, StatsCache, Status, StatusTone, TreeMotion};
use crate::icons::{self, icon};
use crate::motion::{self, Tween};
use crate::popover::{self, Popup};
use crate::state::AppState;
use crate::theme::Theme;
use crate::typography::ui_rems;

const ROW: f32 = 30.0;
const LABEL_ROW: f32 = 22.0;
const GAP: f32 = 4.0;
const INDENT: f32 = 24.0;
const ANCHOR: (f32, f32) = (14.0, 15.0);
const PAD_X: f32 = 10.0;
const PAD_BOTTOM: f32 = 10.0;
const LINK_WIDTH: f32 = 1.75;
const DOT_RADIUS: f32 = 3.5;
const LEAD: &str = "lead";

#[derive(Clone)]
pub(crate) enum StripTreeEvent {
    Open {
        selection: Selection,
        chat_id: Option<String>,
    },
    OpenNative {
        parent_chat_id: String,
        doc_id: String,
        title: String,
        frozen: bool,
    },
    Brief(Selection),
}

struct RowMenu {
    selection: Selection,
    chat_id: Option<String>,
    position: Point<Pixels>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TreeHost {
    Strip,
    Supervisor,
    Intro,
    Center,
}

pub(crate) type RowRects = std::rc::Rc<std::cell::RefCell<HashMap<String, gpui::Bounds<Pixels>>>>;

pub(crate) struct SampleRow {
    pub title: &'static str,
    pub status: Status,
    pub word: &'static str,
    pub brand: Option<(&'static str, Option<Hsla>)>,
    pub secs: Option<i64>,
}

enum Source {
    Tree { root_chat_id: String, native: bool },
    Sample(Vec<SampleRow>, DateTime<Utc>),
}

struct Row {
    id: String,
    parent: Option<usize>,
    depth: usize,
    head: Option<&'static str>,
    title: String,
    status: Status,
    word: &'static str,
    brand: Option<(&'static str, Option<Hsla>)>,
    chat_id: Option<String>,
    since: Option<DateTime<Utc>>,
    selection: Selection,
    label: Option<String>,
    native_of: Option<String>,
}

struct Target {
    selection: Selection,
    chat_id: Option<String>,
    native_of: Option<String>,
    id: String,
    title: String,
    frozen: bool,
}

impl Row {
    fn height(&self) -> f32 {
        if self.label.is_some() { LABEL_ROW } else { ROW }
    }
}

pub(crate) struct StripTree {
    state: Entity<AppState>,
    source: Source,
    folded: HashSet<String>,
    seen: HashSet<String>,
    chevrons: HashMap<String, Tween>,
    open: Tween,
    motion: TreeMotion,
    stats: StatsCache,
    since: Instant,
    tick: Option<gpui::Task<()>>,
    host: TreeHost,
    fold_hover: Option<String>,
    menu: Popup<RowMenu>,
    briefs: bool,
    compact: bool,
    lit: bool,
    picked: Option<String>,
    rects: RowRects,
    _state: Subscription,
}

impl EventEmitter<StripTreeEvent> for StripTree {}

impl StripTree {
    pub fn new(
        state: Entity<AppState>,
        root_chat_id: String,
        native: bool,
        open: bool,
        host: TreeHost,
        cx: &mut Context<Self>,
    ) -> Self {
        let now = motion::now();
        Self {
            _state: cx.observe(&state, |_, _, cx| cx.notify()),
            state,
            source: Source::Tree {
                root_chat_id,
                native,
            },
            folded: HashSet::new(),
            seen: HashSet::new(),
            chevrons: HashMap::new(),
            open: Tween::at(f32::from(u8::from(open)), now),
            motion: TreeMotion::solid(),
            stats: StatsCache::default(),
            since: now,
            tick: None,
            host,
            fold_hover: None,
            menu: Popup::default(),
            briefs: false,
            compact: false,
            lit: false,
            picked: None,
            rects: RowRects::default(),
        }
    }

    pub fn with_briefs(mut self, briefs: bool) -> Self {
        self.briefs = briefs;
        self
    }

    pub fn sample(state: Entity<AppState>, rows: Vec<SampleRow>, cx: &mut Context<Self>) -> Self {
        let mut tree = Self::new(state, String::new(), false, true, TreeHost::Intro, cx);
        tree.source = Source::Sample(rows, Utc::now());
        tree
    }

    pub fn root_id(&self) -> &str {
        match &self.source {
            Source::Tree { root_chat_id, .. } => root_chat_id,
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
            motion::now(),
        );
        cx.notify();
    }

    pub fn set_compact(&mut self, compact: bool, cx: &mut Context<Self>) {
        if self.compact != compact {
            self.compact = compact;
            cx.notify();
        }
    }

    pub fn rects(&self) -> RowRects {
        self.rects.clone()
    }

    pub fn set_picked(&mut self, picked: Option<String>, cx: &mut Context<Self>) {
        if self.picked != picked {
            self.picked = picked;
            cx.notify();
        }
    }

    pub fn set_lit(&mut self, lit: bool, cx: &mut Context<Self>) {
        if self.lit != lit {
            self.lit = lit;
            cx.notify();
        }
    }

    fn rows(&self, cx: &App) -> Option<Vec<Row>> {
        let (root_chat_id, native) = match &self.source {
            Source::Tree {
                root_chat_id,
                native,
            } => (root_chat_id, *native),
            Source::Sample(rows, at) => return Some(sample_rows(rows, *at)),
        };
        let nodes = thread_tree::tree_rows(self.state.read(cx), root_chat_id, native, Utc::now());
        if nodes.is_empty() {
            return None;
        }
        let mut rows: Vec<Row> = Vec::with_capacity(nodes.len() + 2);
        let mut at: Vec<usize> = Vec::with_capacity(nodes.len());
        for node in &nodes {
            if let Some(group) = node.group {
                let label = group.label();
                rows.push(Row {
                    id: format!("label-{}", node.id),
                    parent: None,
                    depth: node.depth,
                    head: None,
                    title: String::new(),
                    status: Status::Done,
                    word: "",
                    brand: None,
                    chat_id: None,
                    since: None,
                    selection: Selection::Lead,
                    label: Some(label),
                    native_of: None,
                });
            }
            at.push(rows.len());
            let selection = match (node.kind, &node.step_id) {
                (NodeKind::Root, _) => Selection::Lead,
                (NodeKind::Step(_), Some(step_id)) => Selection::Step(step_id.clone()),
                _ => Selection::Node(node.id.clone()),
            };
            rows.push(Row {
                id: node.id.clone(),
                parent: node.parent.map(|parent| at[parent]),
                depth: node.depth,
                head: (node.kind == NodeKind::Root).then_some("Lead"),
                title: node.title.clone(),
                status: node.status,
                word: node.word,
                brand: node.brand,
                chat_id: node.chat_id.clone(),
                since: node.since,
                selection,
                label: None,
                native_of: (node.kind == NodeKind::Native)
                    .then(|| node.parent.and_then(|parent| nodes[parent].chat_id.clone()))
                    .flatten(),
            });
        }
        Some(rows)
    }

    fn fold_deep(&mut self, rows: &[Row]) {
        for (ix, row) in rows.iter().enumerate() {
            if row.depth >= 2
                && self.seen.insert(row.id.clone())
                && rows.iter().any(|other| other.parent == Some(ix))
            {
                self.folded.insert(row.id.clone());
            }
        }
    }

    fn visible(&self, rows: &[Row], ix: usize) -> bool {
        if rows[ix].label.is_some() {
            return rows
                .get(ix + 1)
                .is_none_or(|next| self.visible(rows, ix + 1) || next.parent.is_none());
        }
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

    fn content_height(&self, rows: &[Row], now: Instant) -> f32 {
        let shown: f32 = rows
            .iter()
            .map(|row| (row.height() + GAP) * self.motion.presence(&row.id, now))
            .sum();
        (shown - GAP).max(ROW) + PAD_BOTTOM
    }

    pub fn band(&mut self, cx: &mut Context<Self>) -> (f32, bool) {
        let now = motion::now();
        let Some(rows) = self.rows(cx) else {
            return (0.0, false);
        };
        self.fold_deep(&rows);
        self.sync(&rows, motion::reduced_motion(cx), now);
        (
            self.content_height(&rows, now) * self.open.value_at(now),
            !self.open.settled_at(now) || self.motion.animating(now),
        )
    }

    fn toggle_fold(&mut self, id: &str, cx: &mut Context<Self>) {
        let folded = !self.folded.remove(id);
        if folded {
            self.folded.insert(id.to_string());
        }
        let now = motion::now();
        let reduced = motion::reduced_motion(cx);
        self.chevrons
            .entry(id.to_string())
            .or_insert_with(|| Tween::at(f32::from(u8::from(!folded)), now))
            .retarget(f32::from(u8::from(folded)), tree::CHEVRON, reduced, now);
        cx.notify();
    }

    fn open(&mut self, selection: Selection, chat_id: Option<String>, cx: &mut Context<Self>) {
        self.close_menu(cx);
        cx.emit(StripTreeEvent::Open { selection, chat_id });
    }

    fn open_target(&mut self, target: &Target, cx: &mut Context<Self>) {
        match &target.native_of {
            Some(parent) => {
                self.close_menu(cx);
                cx.emit(StripTreeEvent::OpenNative {
                    parent_chat_id: parent.clone(),
                    doc_id: target.id.clone(),
                    title: target.title.clone(),
                    frozen: target.frozen,
                });
            }
            None => self.open(target.selection.clone(), target.chat_id.clone(), cx),
        }
    }

    fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu.begin_close() {
            popover::reap_popup(cx, |tree: &mut Self| &mut tree.menu);
            cx.notify();
        }
    }

    fn render_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let menu = self.menu.get()?;
        let theme = theme.for_popup();
        let (selection, chat_id) = (menu.selection.clone(), menu.chat_id.clone());
        let brief = menu.selection.clone();
        let item = |key: &'static str, label: &'static str, mark: &'static str| {
            popover::menu_row(&theme, false, key)
                .id(key)
                .debug_selector(move || key.into())
                .role(gpui::Role::MenuItem)
                .aria_label(label)
                .child(icon(mark).size(px(15.0)).text_color(theme.text_muted))
                .child(label)
        };
        let card = popover::popover_card(&theme)
            .w(px(200.0))
            .flex()
            .flex_col()
            .on_mouse_down_out(cx.listener(|tree, _, _, cx| tree.close_menu(cx)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                if chat_id.is_some() {
                    item(
                        "strip-tree-menu-open",
                        "Open thread",
                        icons::LUCIDE_ARROW_UP_RIGHT,
                    )
                } else {
                    item(
                        "strip-tree-menu-open",
                        crate::tasks::OPEN_TASK_PAGE,
                        icons::LUCIDE_CHEVRON_RIGHT,
                    )
                }
                .on_click(cx.listener(move |tree, _: &ClickEvent, _, cx| {
                    tree.open(selection.clone(), chat_id.clone(), cx)
                })),
            )
            .when(self.briefs, |card| {
                card.child(
                    item("strip-tree-menu-brief", "Brief", icons::DOCUMENT).on_click(cx.listener(
                        move |tree, _: &ClickEvent, _, cx| {
                            tree.close_menu(cx);
                            cx.emit(StripTreeEvent::Brief(brief.clone()));
                        },
                    )),
                )
            });
        Some(popover::menu_at(
            "strip-tree-menu",
            menu.position,
            card.into_any_element(),
            self.menu.closing_since(),
        ))
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
        let now = motion::now();
        let reduced = motion::reduced_motion(cx);
        let theme = match self.host {
            TreeHost::Strip | TreeHost::Intro => Theme::of(cx).for_popup(),
            TreeHost::Supervisor | TreeHost::Center => Theme::of(cx).clone(),
        };
        let Some(rows) = self.rows(cx) else {
            return div().into_any_element();
        };
        self.fold_deep(&rows);
        self.sync(&rows, reduced, now);
        if !self.is_open() && self.open.settled_at(now) {
            return div().into_any_element();
        }
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
        let mut k = 0.0;
        for row in &rows {
            if !self.motion.shown(&row.id) {
                placed.push(None);
                continue;
            }
            let layout = k;
            k += row.height() + GAP;
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
        let selected = self.state.read(cx).selected_chat.clone();
        let mut sweeping = false;
        let mut elements = Vec::new();
        self.rects
            .borrow_mut()
            .retain(|id, _| rows.iter().any(|row| row.label.is_none() && row.id == *id));
        for (ix, row) in rows.iter().enumerate() {
            let Some((left, top, frame)) = &placed[ix] else {
                continue;
            };
            if let Some(label) = &row.label {
                elements.push(
                    div()
                        .absolute()
                        .top(px(*top))
                        .left(px(*left + 6.0))
                        .right(px(PAD_X))
                        .h(px(LABEL_ROW))
                        .opacity(frame.opacity)
                        .flex()
                        .items_center()
                        .text_size(ui_rems(11.5))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(theme.text_faint)
                        .child(label.clone())
                        .into_any_element(),
                );
                continue;
            }
            let running = row.status == Status::Running;
            let muted = row.status.muted();
            let title_color = if running || muted {
                theme.text_muted
            } else {
                theme.text
            };
            let line_color = if row.head.is_some() {
                theme.text_muted
            } else {
                title_color
            };
            let line = if running && !reduced {
                sweeping = true;
                crate::transcript::shimmer_label(
                    SharedString::from(row.title.clone()),
                    tree::title_phase(self.since, now),
                    theme.font_sans.clone(),
                    ui_rems(14.0).into(),
                    None,
                    theme.text_muted,
                    theme.text,
                )
                .whitespace_nowrap()
                .text_color(theme.text_muted)
                .into_any_element()
            } else {
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(line_color)
                    .child(row.title.clone())
                    .into_any_element()
            };
            let title = div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(5.0))
                .overflow_hidden()
                .text_size(ui_rems(14.0))
                .font_weight(gpui::FontWeight::MEDIUM)
                .children(row.head.map(|head| {
                    div()
                        .flex_none()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(theme.text)
                        .child(head)
                }))
                .children(row.head.map(|_| {
                    div()
                        .flex_none()
                        .text_color(theme.text_muted)
                        .child("\u{b7}")
                }))
                .child(line);
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
                let hover_id = row.id.clone();
                let selector = format!("strip-tree-fold-{}", row.id);
                div()
                    .id(SharedString::from(selector.clone()))
                    .debug_selector(move || selector.clone())
                    .role(gpui::Role::Button)
                    .aria_expanded(!folded)
                    .aria_label(if folded {
                        format!("Show {hidden} under {}", row.head.unwrap_or(&row.title))
                    } else {
                        format!("Fold {}", row.head.unwrap_or(&row.title))
                    })
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(2.0))
                    .h(px(22.0))
                    .pl(px(6.0))
                    .pr(px(4.0))
                    .rounded(px(5.0))
                    .text_size(ui_rems(12.0))
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
                    .on_hover(cx.listener(move |tree, hovered: &bool, _, cx| {
                        let next = hovered.then(|| hover_id.clone());
                        if tree.fold_hover != next {
                            tree.fold_hover = next;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |tree, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        tree.toggle_fold(&id, cx);
                    }))
            });
            let current = self.host == TreeHost::Strip
                && row.chat_id.is_some()
                && row.chat_id.as_deref() == selected.as_deref();
            let opens = self.host != TreeHost::Intro && !current;
            let go = opens.then(|| {
                let over_fold = self.fold_hover.as_deref() == Some(row.id.as_str());
                div()
                    .flex_none()
                    .size(px(18.0))
                    .mr(px(-4.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .opacity(0.0)
                    .when(!over_fold, |el| {
                        el.group_hover(group.clone(), |s| s.opacity(1.0))
                    })
                    .child(
                        icon(if row.chat_id.is_some() {
                            icons::LUCIDE_ARROW_UP_RIGHT
                        } else {
                            icons::LUCIDE_CHEVRON_RIGHT
                        })
                        .size(px(13.0))
                        .text_color(theme.text),
                    )
            });
            let target = Target {
                selection: row.selection.clone(),
                chat_id: row.chat_id.clone(),
                native_of: row.native_of.clone(),
                id: row.id.clone(),
                title: row.title.clone(),
                frozen: row.status != Status::Running,
            };
            let menu = (opens && self.host == TreeHost::Supervisor)
                .then(|| (row.selection.clone(), row.chat_id.clone()));
            let selector = format!("strip-tree-row-{}", row.id);
            let label = match row.head {
                Some(head) => format!("{head} \u{b7} {}: {}", row.title, row.word),
                None => format!("{}: {}", row.title, row.word),
            };
            let measure = {
                let (rects, id) = (self.rects.clone(), row.id.clone());
                gpui::canvas(
                    move |bounds, _, _| {
                        rects.borrow_mut().insert(id, bounds);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0()
            };
            elements.push(
                div()
                    .id(SharedString::from(selector.clone()))
                    .debug_selector(move || selector.clone())
                    .group(group)
                    .child(measure)
                    .role(if opens {
                        gpui::Role::Button
                    } else {
                        gpui::Role::ListItem
                    })
                    .aria_label(label)
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
                    .when(self.lit && row.head.is_some(), |el| el.bg(theme.wash(0.07)))
                    .when(self.picked.as_deref() == Some(row.id.as_str()), |el| {
                        el.bg(theme.wash(0.09))
                    })
                    .when(opens, |el| {
                        el.cursor_pointer()
                            .hover(|s| s.bg(theme.wash(0.07)))
                            .active(|s| s.bg(theme.wash(0.11)))
                    })
                    .text_size(ui_rems(12.5))
                    .text_color(theme.text_muted)
                    .child(
                        div()
                            .flex_none()
                            .size(px(16.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(tree::glyph(
                                row.status,
                                14.0,
                                row.status.tone(&theme),
                                reduced,
                            )),
                    )
                    .child(title)
                    .children(row.brand.filter(|_| !self.compact).map(|(mark, tint)| {
                        icon(mark)
                            .size(px(14.0))
                            .flex_none()
                            .text_color(tint.unwrap_or(theme.text_muted))
                    }))
                    .child(
                        div()
                            .flex_none()
                            .text_color(if muted {
                                theme.text_faint
                            } else {
                                row.status.tone(&theme)
                            })
                            .child(row.word),
                    )
                    .children(
                        times[ix]
                            .clone()
                            .filter(|_| !self.compact)
                            .map(|time| div().flex_none().child(time)),
                    )
                    .children(fold)
                    .children(go)
                    .when(opens, |el| {
                        el.on_click(cx.listener(move |tree, _: &ClickEvent, _, cx| {
                            tree.open_target(&target, cx)
                        }))
                    })
                    .when_some(menu, |el, (selection, chat_id)| {
                        el.on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |tree, event: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                window.prevent_default();
                                tree.menu.open(RowMenu {
                                    selection: selection.clone(),
                                    chat_id: chat_id.clone(),
                                    position: event.position,
                                });
                                cx.notify();
                            }),
                        )
                    })
                    .into_any_element(),
            );
        }
        if sweeping {
            motion::pulse_lease(cx.entity_id(), cx);
        }
        if rows.iter().any(|row| row.status.live()) {
            self.keep_ticking(cx);
        }
        let menu = self.render_menu(&theme, cx);
        if self.motion.animating(now) || self.chevrons.values().any(|t| !t.settled_at(now)) {
            window.request_animation_frame();
        }
        div()
            .id("strip-tree")
            .debug_selector(|| "strip-tree".into())
            .relative()
            .w_full()
            .h(px(self.content_height(&rows, now)))
            .children(elements)
            .child(tree::links(
                links,
                dots.into_iter().map(|(_, dot)| dot).collect(),
                LINK_WIDTH,
                DOT_RADIUS,
            ))
            .children(menu)
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
            head: (ix == 0).then_some("Lead"),
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
            label: None,
            native_of: None,
        })
        .collect()
}
