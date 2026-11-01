use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, ClickEvent, Context, Div, Entity, Hsla, MouseButton, MouseDownEvent,
    SharedString, Stateful, Window, div, point, prelude::*, px,
};
use zeron_doc::background::{agent_runs, agent_tree, latest_prompt, rows_missing, walked_docs};
use zeron_doc::{MessagePart, SessionMessageEntry};
use zeron_proto::background::{
    AgentRun, RunState, agents_done, agents_label, aggregate, child_state, child_threads,
    delegate_runs, elapsed_label, latest_line, roster_heading, run_depths, shell_tasks,
    shells_label, stoppable,
};
use zeron_proto::{BackgroundTask, Chat, HarnessId, ReasoningLevel};
use zeron_rpc::methods;

use super::{Composer, ComposerEvent};
use crate::icons::{self, icon};
use crate::motion;
use crate::popover::{self, Popup};
use crate::shell::children::child_title;
use crate::theme::Theme;
use crate::transcript::{
    ChipTrail, Row, RowKind, TOOL_TREE_ROW_HEIGHT, ToolItem, ToolItemKind, activity_rail,
    block_plain_text, chip_a11y_label, chip_card, chip_header_row, detail_body, rows_for_entry,
    tool_group_collapses, tool_group_header, tool_title_shimmer_phase,
};

const FRAMES: [u16; 8] = [189, 220, 90, 78, 45, 291, 306, 433];
const FRAME_MS: u64 = 175;
const DOT_GRID_CYCLE: motion::MotionSpec =
    motion::MotionSpec::new(FRAME_MS * FRAMES.len() as u64, motion::EASE);
const DOT_SIZE: f32 = 2.25;
const DOT_GAP: f32 = 1.75;
const DOT_BOX: f32 = DOT_SIZE * 3.0 + DOT_GAP * 2.0;
const GLYPH_SIZE: f32 = 12.0;
const PILL_HEIGHT: f32 = 28.0;
const SETTLE_FADE: Duration = Duration::from_millis(160);
const CARD_RISE: f32 = 10.0;
const CARD_DROP: f32 = 4.0;
const PILL_RISE: f32 = 4.0;
const AGENTS_PILL_FLOOR: f32 = 140.0;
const AGENTS_PILL_SHRINK: f32 = 100.0;
const ELAPSED_TICK: Duration = Duration::from_secs(1);
const STOP_TIMEOUT: Duration = Duration::from_secs(15);
const SHELLS_MENU_WIDTH: f32 = 420.0;
const DETAIL_MENU_WIDTH: f32 = 600.0;
const DETAIL_LOG_HEIGHT: f32 = 300.0;
const DETAIL_ROW_LIMIT: usize = 300;
const DETAIL_TEXT_CHARS: usize = 4_000;
pub(crate) use zeron_proto::transcript::compact_count;

fn clip(text: &str) -> String {
    match text.char_indices().nth(DETAIL_TEXT_CHARS) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_owned(),
    }
}

pub(crate) fn detail_rows(entries: &[SessionMessageEntry], hide_thinking: bool) -> Vec<Row> {
    let mut parse = |_: &str, text: &str| Arc::new(crate::markdown::parse_full(text));
    let mut rows: Vec<Row> = entries
        .iter()
        .flat_map(|entry| {
            let mut entry = entry.clone();
            entry
                .parts
                .retain(|part| !(hide_thinking && matches!(part, MessagePart::Reasoning { .. })));
            for part in &mut entry.parts {
                if let MessagePart::Tool {
                    output: Some(output),
                    ..
                } = part
                {
                    *output = clip(output);
                }
            }
            rows_for_entry(&entry, false, false, &mut parse)
        })
        .collect();
    let skip = rows.len().saturating_sub(DETAIL_ROW_LIMIT);
    rows.split_off(skip)
}

fn detail_copy_text(rows: &[Row]) -> String {
    rows.iter()
        .flat_map(|row| match &row.kind {
            RowKind::ToolGroup { tools, .. } => tools
                .iter()
                .map(|tool| chip_a11y_label(tool).to_string())
                .collect(),
            RowKind::Markdown { tree, block_ix } | RowKind::LiveMarkdown { tree, block_ix } => tree
                .blocks
                .get(*block_ix)
                .map(|top| block_plain_text(&top.block))
                .into_iter()
                .collect(),
            RowKind::User { text, .. } => vec![text.to_string()],
            RowKind::ErrorChip { message } => vec![message.to_string()],
            _ => Vec::new(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn live_part(rows: &[Row]) -> Option<&str> {
    let RowKind::ToolGroup { tools, .. } = &rows.last()?.kind else {
        return None;
    };
    tools
        .last()
        .filter(|tool| tool.kind == ToolItemKind::Call && !tool.resolved)
        .map(|tool| tool.part_id.as_str())
}

fn reasoning_text<'a>(entries: &'a [SessionMessageEntry], part_id: &str) -> Option<&'a str> {
    entries
        .iter()
        .flat_map(|entry| &entry.parts)
        .find_map(|part| match part {
            MessagePart::Reasoning { id, text } if id == part_id => Some(text.as_str()),
            _ => None,
        })
}

pub(crate) fn tool_count(entries: &[SessionMessageEntry]) -> usize {
    entries
        .iter()
        .flat_map(|entry| &entry.parts)
        .filter(|part| matches!(part, MessagePart::Tool { .. }))
        .count()
}

pub(crate) fn dot_lit(frame: u16, dot: usize) -> bool {
    frame & (1 << (8 - dot)) != 0
}

pub(crate) fn sidebar_ring_mark(count: usize, theme: &Theme) -> Div {
    div()
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(3.0))
        .child(
            div()
                .size(px(7.0))
                .flex_none()
                .rounded_full()
                .border(px(1.5))
                .border_color(theme.accent),
        )
        .when(count > 0, |el| {
            el.child(
                div()
                    .text_size(crate::typography::ui_rems(10.0))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(count.to_string())),
            )
        })
}

#[derive(Default)]
pub(crate) struct BackgroundUi {
    chat: Option<String>,
    roster: Popup<Instant>,
    agents_pill_since: Option<Instant>,
    shells_pill_since: Option<Instant>,
    shells: Popup<()>,
    agents_glyph: Option<(RunState, Option<Instant>)>,
    tick: Option<gpui::Task<()>>,
    outputs: HashMap<String, ShellOutput>,
    tokens: HashMap<String, AgentTokens>,
    walks: HashMap<String, AgentWalk>,
    widen: Option<bool>,
    detail: Option<TrayDetail>,
    stopping: HashSet<String>,
}

struct BackgroundRuns {
    chat_id: String,
    harness: Option<HarnessId>,
    chat_reasoning: Option<ReasoningLevel>,
    agents: Vec<AgentRun>,
    children: Vec<(Chat, RunState)>,
    shells: Vec<BackgroundTask>,
    shells_stoppable: bool,
    walk: Vec<String>,
    counted: u32,
}

struct ShellOutput {
    lines: Vec<String>,
    _watch: Option<gpui::Task<()>>,
}

struct AgentWalk {
    entries: Vec<SessionMessageEntry>,
    loaded: bool,
    _watch: Option<gpui::Task<()>>,
}

struct AgentTokens {
    count: Option<u64>,
    _watch: Option<gpui::Task<()>>,
}

#[derive(Clone, Debug, PartialEq)]
enum DetailTarget {
    Shell(String),
    Agent(String),
}

struct TrayDetail {
    target: DetailTarget,
    scroll: gpui::ScrollHandle,
    hide_thinking: bool,
    feed: Option<AgentFeed>,
    folded: HashSet<SharedString>,
    expanded: HashSet<SharedString>,
    opened: Instant,
}

impl TrayDetail {
    fn new(target: DetailTarget, feed: Option<AgentFeed>) -> Self {
        Self {
            target,
            scroll: gpui::ScrollHandle::new(),
            hide_thinking: false,
            feed,
            folded: HashSet::new(),
            expanded: HashSet::new(),
            opened: motion::now(),
        }
    }

    fn rebuild_rows(&mut self) {
        if let Some(feed) = self.feed.as_mut() {
            feed.rows = detail_rows(&feed.entries, self.hide_thinking);
        }
    }
}

struct AgentFeed {
    doc_id: String,
    entries: Vec<SessionMessageEntry>,
    rows: Vec<Row>,
    _watch: Option<gpui::Task<()>>,
}

fn tween_progress(since: Option<Instant>, span: Duration, reduced: bool, now: Instant) -> f32 {
    match since {
        Some(since) if !reduced => {
            let total = span.mul_f32(motion::speed_scale()).as_secs_f32();
            (now.saturating_duration_since(since).as_secs_f32() / total).clamp(0.0, 1.0)
        }
        _ => 1.0,
    }
}

pub(super) fn eased(
    spec: &motion::MotionSpec,
    since: Option<Instant>,
    reduced: bool,
    now: Instant,
) -> f32 {
    spec.progress(tween_progress(since, spec.total(), reduced, now))
}

#[derive(Clone, Copy, Debug)]
pub(super) struct CardMotion {
    pub opacity: f32,
    pub offset: f32,
    pub animating: bool,
    pub live: bool,
    pub gone: bool,
}

pub(super) fn card_motion(
    opened: Instant,
    closing: Option<Instant>,
    reduced: bool,
    now: Instant,
) -> CardMotion {
    let slide = eased(&motion::SIDECAR_POPOVER_SLIDE, Some(opened), reduced, now);
    let fade = eased(&motion::SIDECAR_POPOVER_FADE, Some(opened), reduced, now);
    let exit = closing.map(|since| eased(&motion::MENU_OUT, Some(since), reduced, now));
    let out = exit.unwrap_or(0.0);
    CardMotion {
        opacity: fade * (1.0 - out),
        offset: CARD_RISE * (1.0 - slide) + CARD_DROP * out,
        animating: slide < 1.0 || fade < 1.0 || exit.is_some_and(|t| t < 1.0),
        live: closing.is_none(),
        gone: exit.is_some_and(|t| t >= 1.0),
    }
}

pub(super) fn presence(since: &mut Option<Instant>, shown: bool, now: Instant) -> Option<Instant> {
    if !shown {
        *since = None;
        return None;
    }
    Some(*since.get_or_insert(now))
}

pub(super) fn pill_enter(since: Option<Instant>, reduced: bool, now: Instant) -> f32 {
    eased(&motion::FADE_QUICK, since, reduced, now)
}

pub(super) fn pill_exit(since: Instant, reduced: bool, now: Instant) -> f32 {
    eased(&motion::MENU_OUT, Some(since), reduced, now)
}

pub(super) fn entering(pill: Stateful<Div>, t: f32) -> Stateful<Div> {
    pill.relative().top(px(PILL_RISE * (1.0 - t))).opacity(t)
}

pub(super) fn card_frame<E>(card: E, motion: CardMotion) -> crate::frost::Frosted
where
    E: Styled + ParentElement + IntoElement,
{
    let mut card = card
        .relative()
        .top(px(motion.offset))
        .opacity(motion.opacity);
    if !motion.live {
        card = card.child(div().absolute().inset_0().occlude());
    }
    crate::frost::frosted(14.0, crate::frost::MENU_BLUR * motion.opacity, card)
}

impl Composer {
    pub(super) fn render_corner(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Option<AnyElement>, Option<AnyElement>) {
        let (mut pills, roster) = self
            .render_background_corner(window, cx)
            .unwrap_or_default();
        let (waiting, waiting_card) = self.render_waiting(window, cx);
        pills.extend(waiting);
        let row = (!pills.is_empty()).then(|| {
            div()
                .relative()
                .flex()
                .flex_row()
                .items_end()
                .gap(px(6.0))
                .children(pills)
                .child(div().flex_1().min_w_0())
                .child(self.waiting_row_probe())
                .into_any_element()
        });
        (waiting_card.or(roster), row)
    }

    fn render_background_corner(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<(Vec<AnyElement>, Option<AnyElement>)> {
        let BackgroundRuns {
            chat_id,
            harness,
            chat_reasoning,
            agents,
            children,
            shells,
            shells_stoppable,
            walk,
            counted,
        } = self.background_runs(cx)?;
        if self.background.chat.as_deref() != Some(chat_id.as_str()) {
            self.background = BackgroundUi {
                chat: Some(chat_id.clone()),
                ..Default::default()
            };
        }
        self.sync_agent_walks(walk, cx);
        if self.background.walks.values().all(|walk| walk.loaded) {
            let short = rows_missing(&agents, counted);
            match self.background.widen {
                None => {
                    self.background.widen = Some(short);
                    if short {
                        cx.notify();
                    }
                }
                Some(true) if !short => self.background.widen = Some(false),
                _ => {}
            }
        }
        let listed: Vec<AgentRun> = agents
            .iter()
            .cloned()
            .chain(
                children
                    .iter()
                    .map(|(child, state)| child_run(child, *state)),
            )
            .collect();
        if listed.is_empty() {
            self.background.roster = Popup::default();
            self.background.agents_glyph = None;
        }
        if shells.is_empty() {
            self.background.shells = Popup::default();
        }
        self.background
            .stopping
            .retain(|id| shells.iter().any(|task| &task.id == id));
        let stale = match self.background.detail.as_ref().map(|d| &d.target) {
            Some(DetailTarget::Shell(id)) => {
                self.background.shells.get().is_none() || !shells.iter().any(|t| &t.id == id)
            }
            Some(DetailTarget::Agent(part)) => {
                self.background.roster.get().is_none()
                    || !agents.iter().any(|run| &run.part_id == part)
            }
            None => false,
        };
        if stale {
            self.background.detail = None;
        }
        if listed.is_empty() && shells.is_empty() {
            return None;
        }

        let theme = Theme::of(cx).clone();
        let stoppable = stoppable(harness);
        let now = motion::now();
        let now_ms = chrono::Utc::now().timestamp_millis();
        let reduced = motion::reduced_motion(cx);

        let overall = aggregate(&listed);
        let settled_since = match self.background.agents_glyph {
            Some((state, since)) if state == overall => since,
            Some(_) => {
                self.background.agents_glyph = Some((overall, Some(now)));
                Some(now)
            }
            None => {
                self.background.agents_glyph = Some((overall, None));
                None
            }
        };
        let fade = tween_progress(settled_since, SETTLE_FADE, reduced, now);
        let roster_motion = self.background.roster.get().map(|opened| {
            card_motion(
                *opened,
                self.background.roster.closing_since(),
                reduced,
                now,
            )
        });
        let roster_open = self.background.roster.is_open();
        let agents_since = presence(
            &mut self.background.agents_pill_since,
            !listed.is_empty(),
            now,
        );
        let shells_since = presence(
            &mut self.background.shells_pill_since,
            !shells.is_empty(),
            now,
        );
        let agents_in = pill_enter(agents_since, reduced, now);
        let shells_in = pill_enter(shells_since, reduced, now);
        if fade < 1.0
            || roster_motion.is_some_and(|motion| motion.animating)
            || agents_in < 1.0
            || shells_in < 1.0
        {
            window.request_animation_frame();
        }
        let shells_mounted = self.background.shells.get().is_some();
        self.sync_output_watches(if shells_mounted { &shells } else { &[] }, cx);
        let token_docs: Vec<&str> = match roster_motion {
            Some(_) => agents.iter().map(|run| run.doc_id.as_str()).collect(),
            None => Vec::new(),
        };
        self.sync_token_watches(&token_docs, cx);
        let ticking = (roster_motion.is_some() && overall == RunState::Working) || shells_mounted;
        if ticking && self.background.tick.is_none() {
            self.background.tick = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(ELAPSED_TICK).await;
                this.update(cx, |composer, cx| {
                    composer.background.tick = None;
                    cx.notify();
                })
                .ok();
            }));
        }

        let roster = roster_motion.map(|motion| {
            self.render_roster(
                &agents,
                &children,
                &listed,
                harness,
                chat_reasoning,
                stoppable,
                motion,
                now_ms,
                &theme,
                window,
                cx,
            )
        });
        let (agents_title, agents_tail) = match listed.as_slice() {
            [one] => match self.review_pill_suffix(&one.doc_id, cx) {
                Some(suffix) => (Some(one.title.clone()), format!("· {suffix}")),
                None => agents_label(&listed),
            },
            _ => agents_label(&listed),
        };
        let agents_done = agents_done(&listed).filter(|_| agents_title.is_none());
        let agents_pill = (!listed.is_empty()).then(|| {
            let pill = pill_frame("background-agents-pill", &theme)
                .debug_selector(|| "background-agents-pill".into())
                .when(roster_open, |el| {
                    el.border_color(theme.border_strong).text_color(theme.text)
                })
                .child(pill_leading(glyph(
                    "background-agents-glyph",
                    overall,
                    fade,
                    &theme,
                )))
                .when_some(agents_title, |el, title| {
                    el.flex_shrink(AGENTS_PILL_SHRINK)
                        .min_w(px(AGENTS_PILL_FLOOR))
                        .child(div().min_w_0().truncate().child(title))
                })
                .child(div().flex_none().child(SharedString::from(agents_tail)))
                .when_some(agents_done, |el, done| {
                    el.child(div().flex_none().child("·")).child(
                        div()
                            .flex_none()
                            .debug_selector(|| "background-agents-done".into())
                            .text_color(theme.success)
                            .child(SharedString::from(done)),
                    )
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, _| this.background.roster.note_trigger_press()),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if !this.background.roster.take_press_was_open() {
                        this.open_roster(cx);
                    }
                }));
            frosted_pill(entering(pill, agents_in), agents_in).into_any_element()
        });
        let shell_pill = (!shells.is_empty()).then(|| {
            self.render_shell_pill(&shells, shells_stoppable, now_ms, shells_in, &theme, cx)
        });
        Some((agents_pill.into_iter().chain(shell_pill).collect(), roster))
    }

    fn background_runs(&self, cx: &App) -> Option<BackgroundRuns> {
        let (chat_id, harness, chat_reasoning) = self.bound_chat_row(cx).map(|chat| {
            let config = chat.config.as_ref();
            (
                chat.id.clone(),
                config.map(|c| c.harness),
                config.and_then(|c| c.reasoning),
            )
        })?;
        let chat_reasoning =
            chat_reasoning.or_else(|| self.pickers.read(cx).resolved(cx).reasoning);
        let state = self.state.read(cx);
        let transcript = self.bound_transcript(state);
        let turn_start = latest_prompt(transcript);
        let walks = &self.background.walks;
        let mut agents = agent_tree(transcript, turn_start.unwrap_or(0), &|id| {
            walks.get(id).map(|walk| walk.entries.as_slice())
        });
        let pending_review = state
            .reviews
            .get(&chat_id)
            .and_then(|review| review.run.as_ref())
            .filter(|run| !run.findings.is_empty())
            .and_then(|run| run.task_id.as_deref());
        let turn_start_ms = turn_start.map_or(0, |ix| transcript[ix].created_at);
        agents.extend(delegate_runs(
            &state.chats,
            &chat_id,
            turn_start_ms,
            pending_review,
        ));
        let walk = walked_docs(transcript, &agents, self.background.widen == Some(true));
        let now = chrono::Utc::now();
        let counted = state.running_subagents_for(&chat_id, now);
        let children = child_threads(&state.chats, &chat_id, turn_start_ms, |child| {
            child_state(state.display_status_for(child, now))
        })
        .into_iter()
        .filter(|(child, _)| !agents.iter().any(|run| run.doc_id == child.id))
        .map(|(child, state)| (child.clone(), state))
        .collect();
        let shells = shell_tasks(state.session_for(&chat_id));
        Some(BackgroundRuns {
            chat_id,
            harness,
            chat_reasoning,
            agents,
            children,
            shells,
            shells_stoppable: self.shells_stoppable(cx),
            walk,
            counted,
        })
    }

    fn shells_stoppable(&self, cx: &App) -> bool {
        let capability = zeron_proto::capabilities::BACKGROUND_TASKS_V1;
        let state = self.state.read(cx);
        self.bound_chat_row(cx).is_some_and(|chat| {
            state.device_supports(&chat.device_id, capability)
                && state
                    .engine()
                    .is_none_or(|engine| engine.engine_info().supports(capability))
        })
    }

    fn stoppable_runs(&self, cx: &App) -> (Vec<(String, bool)>, Vec<String>) {
        let Some(runs) = self.background_runs(cx) else {
            return Default::default();
        };
        let agents = stoppable_agents(&runs.agents, stoppable(runs.harness));
        let shells = if runs.shells_stoppable {
            runs.shells.into_iter().map(|task| task.id).collect()
        } else {
            Vec::new()
        };
        (agents, shells)
    }

    pub(crate) fn can_stop_background(&self, cx: &App) -> bool {
        let (agents, shells) = self.stoppable_runs(cx);
        !agents.is_empty() || !shells.is_empty()
    }

    pub(crate) fn can_stop_shells(&self, cx: &App) -> bool {
        self.shells_stoppable(cx)
    }

    pub(crate) fn stop_all_background(&mut self, cx: &mut Context<Self>) {
        let (agents, shells) = self.stoppable_runs(cx);
        for (task_id, delegated) in agents {
            self.stop_agent_run(task_id, delegated, cx);
        }
        for task_id in shells {
            self.stop_background_task(task_id, cx);
        }
    }

    pub(super) fn roster_open(&self) -> bool {
        self.background.roster.is_open()
    }

    pub(super) fn close_roster(&mut self, cx: &mut Context<Self>) {
        if self.background.roster.begin_close() {
            popover::reap_popup(cx, |this: &mut Composer| &mut this.background.roster);
            cx.notify();
        }
    }

    pub(super) fn open_roster(&mut self, cx: &mut Context<Self>) {
        self.background.roster.open(motion::now());
        self.close_shells(cx);
        self.close_waiting(cx);
        cx.notify();
    }

    pub(super) fn close_shells(&mut self, cx: &mut Context<Self>) {
        if self.background.shells.begin_close() {
            popover::reap_popup(cx, |this: &mut Composer| &mut this.background.shells);
            cx.notify();
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_roster(
        &self,
        agents: &[AgentRun],
        children: &[(Chat, RunState)],
        listed: &[AgentRun],
        harness: Option<HarnessId>,
        chat_reasoning: Option<ReasoningLevel>,
        stoppable: bool,
        motion: CardMotion,
        now_ms: i64,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let running = stoppable_agents(agents, stoppable);
        let header = div()
            .h(px(36.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .pl(px(12.0))
            .pr(px(8.0))
            .text_size(crate::typography::ui_rems(13.0))
            .text_color(theme.text.opacity(0.8))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(SharedString::from(roster_heading(listed))),
            )
            .when(!running.is_empty(), |el| {
                el.child(
                    div()
                        .id("background-roster-stop-all")
                        .debug_selector(|| "background-roster-stop-all".into())
                        .cursor_pointer()
                        .text_color(theme.text_muted)
                        .hover(|s| s.text_color(theme.text))
                        .child("Stop all")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            for (task_id, delegated) in &running {
                                this.stop_agent_run(task_id.clone(), *delegated, cx);
                            }
                        })),
                )
            })
            .child(
                div()
                    .id("background-roster-close")
                    .debug_selector(|| "background-roster-close".into())
                    .size(px(20.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .text_color(theme.text_muted)
                    .hover(|s| s.bg(theme.glass_hover()).text_color(theme.text))
                    .child(
                        icon(icons::CLOSE)
                            .size(px(12.0))
                            .text_color(theme.text_muted),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.close_roster(cx))),
            );
        let dismiss = motion.live.then(|| {
            cx.listener(|this: &mut Self, _: &MouseDownEvent, _, cx| this.close_roster(cx))
        });
        let depths = run_depths(agents);
        let rows = agents.iter().enumerate().map(|(ix, run)| {
            let depth = depths[ix];
            let open = {
                let part_id = run.part_id.clone();
                let doc_id = run.doc_id.clone();
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.open_agent_detail(part_id.clone(), doc_id.clone(), cx);
                })
            };
            let state_text = run_state_text(run, now_ms);
            let run_harness = run.harness.or(harness);
            let tag = run.tag(harness);
            let level = run.reasoning.or(chat_reasoning);
            let tokens = self.agent_tokens(&run.doc_id);
            let stop = run.can_stop(stoppable).then(|| {
                let task_id = run.part_id.clone();
                let delegated = run.delegated;
                stop_button(format!("background-roster-stop-{ix}"), theme).on_click(cx.listener(
                    move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.stop_agent_run(task_id.clone(), delegated, cx);
                    },
                ))
            });
            div()
                .id(("background-roster-row", ix))
                .debug_selector(move || format!("background-roster-row-{ix}"))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .px(px(12.0))
                .pl(px(12.0 + 16.0 * depth as f32))
                .py(px(7.0))
                .cursor_pointer()
                .text_size(crate::typography::ui_rems(13.0))
                .text_color(theme.text.opacity(0.8))
                .hover(|s| s.bg(theme.glass_hover()))
                .child(glyph(
                    SharedString::from(format!("background-roster-glyph-{ix}")),
                    run.state,
                    1.0,
                    theme,
                ))
                .child(
                    div()
                        .debug_selector(move || format!("background-roster-title-{ix}"))
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(run.title.clone()),
                )
                .when_some(tag, |el, tag| el.child(model_tag(tag, run_harness, theme)))
                .when_some(level, |el, level| {
                    el.child(
                        detail_chip(
                            "thinking",
                            crate::pickers::reasoning_label(level).to_string(),
                            theme,
                        )
                        .debug_selector(move || format!("background-roster-thinking-{ix}")),
                    )
                })
                .when_some(tokens, |el, tokens| {
                    el.child(
                        detail_chip("tokens", compact_count(tokens), theme)
                            .debug_selector(move || format!("background-roster-tokens-{ix}")),
                    )
                })
                .child(
                    div()
                        .flex_none()
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.text_faint)
                        .child(SharedString::from(state_text)),
                )
                .child(div().size(px(20.0)).flex_none().children(stop))
                .on_click(open)
        });
        let child_rows = children.iter().enumerate().map(|(ix, (child, state))| {
            let kind = crate::shell::children::ChildKind::of(child);
            let id = child.id.clone();
            let state_text = match state {
                RunState::Working => RunState::Working.word().to_string(),
                _ => run_state_text(&child_run(child, *state), now_ms),
            };
            div()
                .id(("background-roster-child", ix))
                .debug_selector(move || format!("background-roster-child-{ix}"))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .px(px(12.0))
                .py(px(7.0))
                .cursor_pointer()
                .text_size(crate::typography::ui_rems(13.0))
                .text_color(theme.text.opacity(0.8))
                .hover(|s| s.bg(theme.glass_hover()))
                .child(glyph(
                    SharedString::from(format!("background-roster-child-glyph-{ix}")),
                    *state,
                    1.0,
                    theme,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(SharedString::from(child_title(child))),
                )
                .child(model_tag(kind.label().to_owned(), None, theme))
                .child(
                    div()
                        .flex_none()
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.text_faint)
                        .child(SharedString::from(state_text)),
                )
                .child(div().size(px(20.0)).flex_none())
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                    cx.emit(ComposerEvent::OpenChildChat(id.clone()));
                }))
        });
        let opened = match self.background.detail.as_ref().map(|d| &d.target) {
            Some(DetailTarget::Agent(part)) => agents.iter().find(|run| &run.part_id == part),
            _ => None,
        };
        let card = div()
            .when_some(dismiss, |el, dismiss| {
                el.debug_selector(|| "background-roster".into())
                    .on_mouse_down_out(dismiss)
            })
            .occlude()
            .w_full()
            .rounded(px(14.0))
            .border_1()
            .border_color(theme.border)
            .bg(popover::surface_bg(theme))
            .when(!theme.is_frost(), |el| el.shadow_lg())
            .overflow_hidden()
            .pb(px(4.0));
        let card = match opened {
            Some(run) => card.p(px(6.0)).child(self.render_agent_detail(
                run,
                harness,
                chat_reasoning,
                stoppable,
                now_ms,
                theme,
                window,
                cx,
            )),
            None => card.child(header).children(rows).children(child_rows),
        };
        card_frame(card, motion).into_any_element()
    }

    fn render_shell_pill(
        &self,
        shells: &[BackgroundTask],
        stoppable: bool,
        now_ms: i64,
        enter: f32,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let pill = pill_frame("background-shells-pill", theme)
            .debug_selector(|| "background-shells-pill".into())
            .child(pill_leading(glyph(
                "background-shells-glyph",
                RunState::Working,
                1.0,
                theme,
            )))
            .child(
                div()
                    .font_family(theme.font_mono.clone())
                    .text_size(crate::typography::ui_rems(10.5))
                    .text_color(theme.text_muted)
                    .child(">_"),
            )
            .child(div().child(SharedString::from(shells_label(shells.len()))))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.background.shells.note_trigger_press()),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                if this.background.shells.take_press_was_open() {
                    return;
                }
                this.background.shells.open(());
                this.close_roster(cx);
                this.close_waiting(cx);
                cx.notify();
            }));
        let opened = match self.background.detail.as_ref().map(|d| &d.target) {
            Some(DetailTarget::Shell(id)) => shells.iter().find(|task| &task.id == id),
            _ => None,
        };
        let menu = self.background.shells.get().map(|_| {
            let rows = shells.iter().enumerate().map(|(ix, task)| {
                let elapsed = elapsed_label((now_ms - task.started_at.timestamp_millis()) / 1000);
                let stop = stoppable.then(|| {
                    self.shell_stop(&task.id, format!("background-shell-stop-{ix}"), theme, cx)
                });
                let description = task.description.clone().map(|description| {
                    div()
                        .debug_selector(move || format!("background-shell-description-{ix}"))
                        .pl(px(GLYPH_SIZE + 8.0))
                        .truncate()
                        .text_size(crate::typography::ui_rems(11.5))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(description))
                });
                let lines = self
                    .background
                    .outputs
                    .get(&task.id)
                    .map(|output| output.lines.as_slice())
                    .unwrap_or_default();
                let latest = latest_line(lines).map(|line| {
                    div()
                        .debug_selector(move || format!("background-shell-line-{ix}"))
                        .pl(px(GLYPH_SIZE + 8.0))
                        .truncate()
                        .font_family(theme.font_mono.clone())
                        .text_size(crate::typography::ui_rems(10.5))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(line.to_owned()))
                });
                let open = {
                    let task_id = task.id.clone();
                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.background.detail =
                            Some(TrayDetail::new(DetailTarget::Shell(task_id.clone()), None));
                        cx.notify();
                    })
                };
                let head = div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(glyph(
                        SharedString::from(format!("background-shell-glyph-{ix}")),
                        RunState::Working,
                        1.0,
                        theme,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(theme.font_mono.clone())
                            .text_size(crate::typography::ui_rems(11.5))
                            .text_color(theme.text)
                            .child(SharedString::from(task.command.clone())),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(crate::typography::ui_rems(11.5))
                            .text_color(theme.text_muted)
                            .child(SharedString::from(elapsed)),
                    )
                    .child(stop_slot(stop));
                div()
                    .id(("background-shell-row", ix))
                    .debug_selector(move || format!("background-shell-row-{ix}"))
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .px(px(8.0))
                    .py(px(7.0))
                    .rounded(px(popover::MENU_ITEM_RADIUS))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.glass_hover()))
                    .child(head)
                    .children(description)
                    .children(latest)
                    .on_click(open)
            });
            let rows: Vec<_> = rows.collect();
            let card = popover::popover_card(theme)
                .debug_selector(|| "background-shells-menu".into())
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    if this.background.shells.begin_close() {
                        popover::reap_popup(cx, |this: &mut Composer| &mut this.background.shells);
                        cx.notify();
                    }
                }));
            let content = match opened {
                Some(task) => card
                    .w(px(DETAIL_MENU_WIDTH))
                    .child(self.render_shell_detail(task, stoppable, now_ms, theme, cx))
                    .into_any_element(),
                None => card
                    .w(px(SHELLS_MENU_WIDTH))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .px(px(8.0))
                            .pt(px(6.0))
                            .pb(px(4.0))
                            .text_size(crate::typography::ui_rems(11.0))
                            .text_color(theme.text_muted)
                            .child(div().flex_1().child("Background shells"))
                            .child(SharedString::from(format!("{} running", shells.len()))),
                    )
                    .children(rows)
                    .into_any_element(),
            };
            popover::anchored_menu_above_at(
                "background-shells-menu",
                point(px(0.0), px(0.0)),
                content,
                self.background.shells.closing_since(),
            )
        });
        div()
            .relative()
            .child(frosted_pill(entering(pill, enter), enter))
            .children(menu)
            .into_any_element()
    }

    fn sync_output_watches(&mut self, shells: &[BackgroundTask], cx: &mut Context<Self>) {
        let watched: Vec<&str> = shells.iter().map(|task| task.id.as_str()).collect();
        self.background
            .outputs
            .retain(|id, _| watched.contains(&id.as_str()));
        for id in watched {
            if !self.background.outputs.contains_key(id) {
                let watch = self.watch_shell_output(id.to_owned(), cx);
                self.background.outputs.insert(
                    id.to_owned(),
                    ShellOutput {
                        lines: Vec::new(),
                        _watch: watch,
                    },
                );
            }
        }
    }

    fn watch_shell_output(
        &self,
        task_id: String,
        cx: &mut Context<Self>,
    ) -> Option<gpui::Task<()>> {
        #[derive(serde::Deserialize)]
        struct OutputFrame {
            #[serde(default)]
            lines: Vec<String>,
        }
        let engine = self.state.read(cx).engine().cloned()?;
        let (chat_id, device_id) = self
            .bound_chat_row(cx)
            .map(|chat| (chat.id.clone(), chat.device_id.clone()))?;
        let params = serde_json::json!({
            "chatId": chat_id,
            "taskId": task_id,
            "targetDeviceId": device_id,
        });
        Some(cx.spawn(async move |this, cx| {
            let mut rx = match engine
                .client()
                .subscribe_checked(methods::WATCH_BACKGROUND_OUTPUT, params)
                .await
            {
                Ok(rx) => rx,
                Err(error) => {
                    tracing::debug!(%error, task_id, "background output watch failed");
                    return;
                }
            };
            while let Some(value) = rx.recv().await {
                let Ok(frame) = serde_json::from_value::<OutputFrame>(value) else {
                    continue;
                };
                let alive = this.update(cx, |this, cx| {
                    if let Some(output) = this.background.outputs.get_mut(&task_id) {
                        output.lines = frame.lines;
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    return;
                }
            }
        }))
    }

    fn sync_agent_walks(&mut self, docs: Vec<String>, cx: &mut Context<Self>) {
        self.background.walks.retain(|id, _| docs.contains(id));
        for id in docs {
            if !self.background.walks.contains_key(&id) {
                let watch = self.watch_doc_entries(id.clone(), true, cx);
                self.background.walks.insert(
                    id,
                    AgentWalk {
                        entries: Vec::new(),
                        loaded: false,
                        _watch: watch,
                    },
                );
            }
        }
    }

    fn agent_tokens(&self, doc_id: &str) -> Option<u64> {
        self.background.tokens.get(doc_id)?.count
    }

    fn sync_token_watches(&mut self, docs: &[&str], cx: &mut Context<Self>) {
        self.background
            .tokens
            .retain(|id, _| docs.contains(&id.as_str()));
        for &id in docs {
            if !self.background.tokens.contains_key(id) {
                let watch = self.watch_agent_tokens(id.to_owned(), cx);
                self.background.tokens.insert(
                    id.to_owned(),
                    AgentTokens {
                        count: None,
                        _watch: watch,
                    },
                );
            }
        }
    }

    fn watch_agent_tokens(&self, doc_id: String, cx: &mut Context<Self>) -> Option<gpui::Task<()>> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct UsageFrame {
            #[serde(default)]
            context_usage: Option<zeron_proto::ContextUsage>,
        }
        let engine = self.state.read(cx).engine().cloned()?;
        Some(cx.spawn(async move |this, cx| {
            let mut rx = match engine
                .client()
                .subscribe_checked(
                    methods::WATCH_DOC_MESSAGES,
                    serde_json::json!({ "chatId": doc_id }),
                )
                .await
            {
                Ok(rx) => rx,
                Err(error) => {
                    tracing::debug!(%error, doc_id, "subagent token watch failed");
                    return;
                }
            };
            while let Some(value) = rx.recv().await {
                let Ok(frame) = serde_json::from_value::<UsageFrame>(value) else {
                    continue;
                };
                let Some(count) = frame.context_usage.and_then(|usage| usage.tokens) else {
                    continue;
                };
                let alive = this.update(cx, |this, cx| {
                    if let Some(tokens) = this.background.tokens.get_mut(&doc_id)
                        && tokens.count != Some(count)
                    {
                        tokens.count = Some(count);
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    return;
                }
            }
        }))
    }

    fn open_agent_detail(&mut self, part_id: String, doc_id: String, cx: &mut Context<Self>) {
        let watch = self.watch_doc_entries(doc_id.clone(), false, cx);
        self.background.detail = Some(TrayDetail::new(
            DetailTarget::Agent(part_id),
            Some(AgentFeed {
                doc_id,
                entries: Vec::new(),
                rows: Vec::new(),
                _watch: watch,
            }),
        ));
        cx.notify();
    }

    fn watch_doc_entries(
        &self,
        doc_id: String,
        walk: bool,
        cx: &mut Context<Self>,
    ) -> Option<gpui::Task<()>> {
        let engine = self.state.read(cx).engine().cloned()?;
        Some(cx.spawn(async move |this, cx| {
            'resubscribe: loop {
                let mut rx = match engine
                    .client()
                    .subscribe_checked(
                        methods::WATCH_DOC_MESSAGES,
                        serde_json::json!({ "chatId": doc_id }),
                    )
                    .await
                {
                    Ok(rx) => rx,
                    Err(error) => {
                        tracing::debug!(%error, doc_id, "subagent feed watch failed");
                        return;
                    }
                };
                while let Some(value) = rx.recv().await {
                    let decoded = cx
                        .background_executor()
                        .spawn(async move {
                            serde_json::from_value::<zeron_doc::TranscriptUpdate>(value)
                        })
                        .await;
                    let Ok(update) = decoded else {
                        continue;
                    };
                    let applied = this.update(cx, |this, cx| {
                        let (entries, quiet) = if walk {
                            let agent = this.background.walks.get_mut(&doc_id)?;
                            let loaded = std::mem::replace(&mut agent.loaded, true);
                            (&mut agent.entries, loaded)
                        } else {
                            let feed = this
                                .background
                                .detail
                                .as_mut()
                                .and_then(|detail| detail.feed.as_mut())
                                .filter(|feed| feed.doc_id == doc_id)?;
                            (&mut feed.entries, false)
                        };
                        let spawns = quiet.then(|| agent_runs(&*entries, 0));
                        let synced =
                            zeron_doc::apply_transcript_frame(entries, update.frame).is_ok();
                        if spawns.is_none_or(|spawns| spawns != agent_runs(&*entries, 0)) {
                            cx.notify();
                        }
                        if !walk && let Some(detail) = this.background.detail.as_mut() {
                            detail.rebuild_rows();
                        }
                        Some(synced)
                    });
                    match applied {
                        Ok(Some(true)) => {}
                        Ok(Some(false)) => continue 'resubscribe,
                        _ => return,
                    }
                }
                return;
            }
        }))
    }

    #[allow(clippy::too_many_arguments)]
    fn render_agent_detail(
        &self,
        run: &AgentRun,
        harness: Option<HarnessId>,
        chat_reasoning: Option<ReasoningLevel>,
        stoppable: bool,
        now_ms: i64,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(detail) = self.background.detail.as_ref() else {
            return div().into_any_element();
        };
        let entries = detail
            .feed
            .as_ref()
            .map(|feed| feed.entries.as_slice())
            .unwrap_or_default();
        let tokens = self.agent_tokens(&run.doc_id);
        let hide_thinking = detail.hide_thinking;
        let stop = run.can_stop(stoppable).then(|| {
            let task_id = run.part_id.clone();
            let delegated = run.delegated;
            stop_button("background-detail-stop".into(), theme)
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.stop_agent_run(task_id.clone(), delegated, cx);
                }))
                .into_any_element()
        });
        let head = detail_head(
            glyph("background-detail-glyph", run.state, 1.0, theme),
            run.title.clone().into(),
            run_state_text(run, now_ms),
            stop,
            theme,
            cx,
        );
        let level = run.reasoning.or(chat_reasoning);
        let run_harness = run.harness.or(harness);
        let chips = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(px(6.0))
            .pl(px(36.0))
            .pb(px(8.0))
            .children(
                run.tag(harness)
                    .map(|tag| model_tag(tag, run_harness, theme)),
            )
            .children(level.map(|level| {
                detail_chip(
                    "thinking",
                    crate::pickers::reasoning_label(level).to_string(),
                    theme,
                )
                .debug_selector(|| "background-detail-thinking".into())
            }))
            .children(tokens.map(|tokens| {
                detail_chip("tokens", compact_count(tokens), theme)
                    .debug_selector(|| "background-detail-tokens".into())
            }))
            .child(detail_chip("tools", tool_count(entries).to_string(), theme));
        let rows = self.detail_feed_rows(run.state == RunState::Working, theme, window, cx);
        let chat_id = self.background.chat.clone().unwrap_or_default();
        let open_pane = {
            let doc_id = run.doc_id.clone();
            let title = run.title.to_string();
            let frozen = run.state != RunState::Working;
            cx.listener(move |_, _: &ClickEvent, _, cx| {
                cx.emit(ComposerEvent::OpenSubagent {
                    chat_id: chat_id.clone(),
                    doc_id: doc_id.clone(),
                    title: title.clone(),
                    frozen,
                });
            })
        };
        let footer = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(14.0))
            .px(px(8.0))
            .pt(px(8.0))
            .pb(px(2.0))
            .text_size(crate::typography::ui_rems(12.0))
            .child(
                detail_link(
                    "background-detail-thinking-toggle",
                    if hide_thinking {
                        "Show thinking"
                    } else {
                        "Hide thinking"
                    },
                    theme.text_muted,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(detail) = this.background.detail.as_mut() {
                        detail.hide_thinking = !detail.hide_thinking;
                        detail.rebuild_rows();
                        cx.notify();
                    }
                })),
            )
            .child(div().flex_1())
            .child(
                detail_link("background-detail-copy", "Copy", theme.accent).on_click(cx.listener(
                    |this, _, _, cx| {
                        let Some(feed) = this
                            .background
                            .detail
                            .as_ref()
                            .and_then(|detail| detail.feed.as_ref())
                        else {
                            return;
                        };
                        let text = detail_copy_text(&feed.rows);
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                    },
                )),
            )
            .child(
                detail_link("background-detail-open-pane", "Open in pane", theme.accent)
                    .on_click(open_pane),
            );
        div()
            .debug_selector(|| "background-detail".into())
            .flex()
            .flex_col()
            .child(head)
            .child(chips)
            .child(self.detail_log(rows, false, theme, cx))
            .child(footer)
            .into_any_element()
    }

    fn detail_feed_rows(
        &self,
        working: bool,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(detail) = self.background.detail.as_ref() else {
            return Vec::new();
        };
        let Some(feed) = detail.feed.as_ref() else {
            return Vec::new();
        };
        let live = live_part(&feed.rows).filter(|_| working);
        let shimmer = live.filter(|_| !cx.reduce_motion()).map(|_| {
            motion::pulse_lease(cx.entity_id(), cx);
            tool_title_shimmer_phase(detail.opened, motion::now())
        });
        let mut group_ix = 0;
        feed.rows
            .iter()
            .filter_map(|row| match &row.kind {
                RowKind::ToolGroup { summary, tools, .. } => {
                    let group = group_ix;
                    group_ix += 1;
                    Some(
                        self.detail_group(
                            detail, row, group, summary, tools, live, shimmer, theme, cx,
                        ),
                    )
                }
                RowKind::Markdown { tree, block_ix } | RowKind::LiveMarkdown { tree, block_ix } => {
                    let options = crate::markdown::render::RenderOptions {
                        math: crate::settings::math_style(cx),
                        ..crate::markdown::render::RenderOptions::settled(row.id.clone())
                    };
                    let block = &tree.blocks.get(*block_ix)?.block;
                    Some(
                        div()
                            .py(px(4.0))
                            .child(crate::markdown::render::render_block(
                                block, *block_ix, *block_ix, &options, theme, window, None,
                            ))
                            .into_any_element(),
                    )
                }
                RowKind::User { text, .. } => Some(
                    div()
                        .debug_selector(|| "background-detail-user".into())
                        .my(px(4.0))
                        .px(px(10.0))
                        .py(px(6.0))
                        .rounded(px(8.0))
                        .bg(theme.wash(0.06))
                        .text_size(crate::typography::ui_rems(12.5))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(clip(text)))
                        .into_any_element(),
                ),
                RowKind::ErrorChip { message } => Some(
                    div()
                        .py(px(4.0))
                        .text_size(crate::typography::ui_rems(12.5))
                        .text_color(theme.danger)
                        .child(message.clone())
                        .into_any_element(),
                ),
                RowKind::InputChip { header, .. } => Some(
                    div()
                        .py(px(4.0))
                        .text_size(crate::typography::ui_rems(12.5))
                        .text_color(theme.text_muted)
                        .child(header.clone())
                        .into_any_element(),
                ),
                _ => None,
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn detail_group(
        &self,
        detail: &TrayDetail,
        row: &Row,
        group: usize,
        summary: &SharedString,
        tools: &[ToolItem],
        live: Option<&str>,
        shimmer: Option<f32>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rail = tool_group_collapses(tools);
        let open = !rail || !detail.folded.contains(&row.id);
        let header = rail.then(|| {
            let key = row.id.clone();
            tool_group_header(
                SharedString::from(format!("{}-hdr", row.id)),
                if open { 1.0 } else { 0.0 },
                summary.clone().into_any_element(),
                theme,
            )
            .debug_selector(move || format!("background-detail-group-{group}"))
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(detail) = this.background.detail.as_mut()
                    && !detail.folded.remove(&key)
                {
                    detail.folded.insert(key.clone());
                }
                cx.notify();
            }))
        });
        let entries = detail
            .feed
            .as_ref()
            .map(|feed| feed.entries.as_slice())
            .unwrap_or_default();
        let items: Vec<AnyElement> = if open {
            tools
                .iter()
                .enumerate()
                .map(|(ix, tool)| {
                    let key = SharedString::from(format!("{}/{}", row.id, tool.part_id));
                    let expanded = detail.expanded.contains(&key);
                    let thought = tool.kind == ToolItemKind::Thought;
                    let expandable = if thought {
                        reasoning_text(entries, &tool.part_id).is_some()
                    } else {
                        tool.detail.is_some()
                    };
                    let body = (expanded && expandable).then(|| {
                        if thought {
                            div()
                                .pb(px(6.0))
                                .text_size(crate::typography::ui_rems(12.5))
                                .text_color(theme.text_muted)
                                .child(SharedString::from(clip(
                                    reasoning_text(entries, &tool.part_id).unwrap_or_default(),
                                )))
                                .into_any_element()
                        } else {
                            div()
                                .mb(px(6.0))
                                .px(px(9.0))
                                .rounded(px(7.0))
                                .bg(theme.wash(0.06))
                                .text_color(theme.text_muted)
                                .children(
                                    tool.detail
                                        .as_deref()
                                        .map(|output| detail_body(output, None, None, &key, theme)),
                                )
                                .into_any_element()
                        }
                    });
                    let live = live == Some(tool.part_id.as_str());
                    let header = chip_header_row(
                        tool,
                        None,
                        expandable.then_some(ChipTrail::Chevron { open: expanded }),
                        None,
                        shimmer.filter(|_| live),
                        theme,
                        cx.entity_id(),
                        cx,
                    );
                    let toggle = key.clone();
                    let card = chip_card(rail)
                        .id(key.clone())
                        .debug_selector(move || format!("background-detail-row-{group}-{ix}"))
                        .when(expandable, |card| {
                            card.cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(detail) = this.background.detail.as_mut()
                                        && !detail.expanded.remove(&toggle)
                                    {
                                        detail.expanded.insert(toggle.clone());
                                    }
                                    cx.notify();
                                }))
                        })
                        .child(header);
                    div()
                        .w_full()
                        .flex()
                        .flex_row()
                        .when(rail, |el| {
                            el.child(activity_rail(
                                tool,
                                ix > 0,
                                ix + 1 < tools.len(),
                                1.0,
                                1.0,
                                TOOL_TREE_ROW_HEIGHT,
                                live.then(|| {
                                    glyph(
                                        SharedString::from(format!("{key}-live")),
                                        RunState::Working,
                                        1.0,
                                        theme,
                                    )
                                }),
                                theme,
                            ))
                        })
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(card)
                                .children(body),
                        )
                        .into_any_element()
                })
                .collect()
        } else {
            Vec::new()
        };
        div()
            .flex()
            .flex_col()
            .py(px(2.0))
            .children(header)
            .children(items)
            .into_any_element()
    }

    fn render_shell_detail(
        &self,
        task: &BackgroundTask,
        stoppable: bool,
        now_ms: i64,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let lines = self
            .background
            .outputs
            .get(&task.id)
            .map(|output| output.lines.as_slice())
            .unwrap_or_default();
        let text = lines.join("\n");
        let stop = stoppable
            .then(|| self.shell_stop(&task.id, "background-detail-stop".into(), theme, cx));
        let elapsed = elapsed_label((now_ms - task.started_at.timestamp_millis()) / 1000);
        let head = detail_head(
            glyph("background-detail-glyph", RunState::Working, 1.0, theme),
            SharedString::from(task.command.clone()),
            elapsed,
            stop,
            theme,
            cx,
        );
        let body = (!text.trim().is_empty())
            .then(|| {
                div()
                    .text_color(theme.text.opacity(0.8))
                    .child(SharedString::from(text.clone()))
                    .into_any_element()
            })
            .into_iter()
            .collect();
        let footer = div()
            .flex()
            .flex_row()
            .items_center()
            .px(px(8.0))
            .pt(px(8.0))
            .pb(px(2.0))
            .text_size(crate::typography::ui_rems(12.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(theme.text_muted)
                    .children(task.description.clone().map(SharedString::from)),
            )
            .child(
                detail_link("background-detail-copy", "Copy", theme.accent).on_click(
                    move |_, _, cx| {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.clone()));
                    },
                ),
            );
        div()
            .debug_selector(|| "background-detail".into())
            .flex()
            .flex_col()
            .p(px(2.0))
            .child(head.pb(px(8.0)))
            .child(self.detail_log(body, true, theme, cx))
            .child(footer)
            .into_any_element()
    }

    fn detail_log(
        &self,
        rows: Vec<AnyElement>,
        mono: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let scroll = self
            .background
            .detail
            .as_ref()
            .map(|detail| detail.scroll.clone())
            .unwrap_or_else(gpui::ScrollHandle::new);
        let pinned = -scroll.offset().y >= scroll.max_offset().y - px(4.0);
        if pinned {
            scroll.scroll_to_bottom();
        }
        let jump = (!pinned).then(|| {
            let scroll = scroll.clone();
            div()
                .id("background-detail-jump")
                .debug_selector(|| "background-detail-jump".into())
                .absolute()
                .right(px(10.0))
                .bottom(px(8.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(4.0))
                .px(px(8.0))
                .py(px(3.0))
                .rounded_full()
                .border_1()
                .border_color(theme.border)
                .bg(popover::surface_bg(theme))
                .cursor_pointer()
                .text_size(crate::typography::ui_rems(11.0))
                .text_color(theme.text_muted)
                .hover(|s| s.text_color(theme.text))
                .child("Jump to latest")
                .child(
                    icon(icons::ALT_ARROW_DOWN)
                        .size(px(10.0))
                        .text_color(theme.text_muted),
                )
                .on_click(cx.listener(move |_, _, _, cx| {
                    scroll.scroll_to_bottom();
                    cx.notify();
                }))
        });
        let empty = rows.is_empty().then(|| {
            div()
                .debug_selector(|| "background-detail-empty".into())
                .child("No output yet")
        });
        div()
            .relative()
            .child(
                div()
                    .id("background-detail-log")
                    .debug_selector(|| "background-detail-log".into())
                    .max_h(px(DETAIL_LOG_HEIGHT))
                    .overflow_y_scroll()
                    .track_scroll(&scroll)
                    .flex()
                    .flex_col()
                    .px(px(10.0))
                    .py(px(8.0))
                    .text_color(theme.text_muted)
                    .map(|log| {
                        if mono {
                            log.gap(px(6.0))
                                .rounded(px(8.0))
                                .bg(theme.wash(0.06))
                                .font_family(theme.font_mono.clone())
                                .text_size(crate::typography::ui_rems(10.5))
                        } else {
                            log.border_t_1()
                                .border_color(theme.border)
                                .font_family(theme.font_sans_fixed.clone())
                        }
                    })
                    .children(rows)
                    .children(empty),
            )
            .children(jump)
    }

    pub(super) fn stop_agent_run(
        &mut self,
        task_id: String,
        delegated: bool,
        cx: &mut Context<Self>,
    ) {
        if !delegated {
            self.stop_background_task(task_id, cx);
            return;
        }
        self.send_stop(
            methods::TASK_CANCEL,
            "cancel delegated task",
            task_id,
            |chat, task_id| serde_json::json!({ "chatId": chat.id, "taskId": task_id }),
            cx,
        );
    }

    fn shell_stop(
        &self,
        task_id: &str,
        id: String,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.background.stopping.contains(task_id) {
            return div()
                .flex_none()
                .debug_selector(move || format!("{id}-stopping"))
                .text_size(crate::typography::ui_rems(11.5))
                .text_color(theme.warning)
                .child("Stopping\u{2026}")
                .into_any_element();
        }
        let task_id = task_id.to_owned();
        stop_button(id, theme)
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.stop_background_task(task_id.clone(), cx);
            }))
            .into_any_element()
    }

    pub(crate) fn stop_background_task(&mut self, task_id: String, cx: &mut Context<Self>) {
        self.background.stopping.insert(task_id.clone());
        cx.notify();
        self.send_stop(
            methods::STOP_BACKGROUND_TASK,
            "stop background task",
            task_id,
            |chat, task_id| {
                serde_json::json!({
                    "chatId": chat.id,
                    "taskId": task_id,
                    "targetDeviceId": chat.device_id,
                })
            },
            cx,
        );
    }

    fn send_stop(
        &mut self,
        method: &'static str,
        label: &'static str,
        task_id: String,
        params: impl FnOnce(&zeron_proto::Chat, String) -> serde_json::Value,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            tracing::warn!(task_id, "{label}: engine not connected");
            return;
        };
        let Some(params) = self.bound_chat_row(cx).map(|chat| params(chat, task_id)) else {
            return;
        };
        cx.spawn(async move |_, cx| {
            if let Err(error) = crate::attachments::call_with_timeout(
                &engine,
                cx.background_executor(),
                method,
                params,
                STOP_TIMEOUT,
            )
            .await
            {
                tracing::warn!(%error, "{label} failed");
            }
        })
        .detach();
    }
}

fn stoppable_agents(agents: &[AgentRun], stoppable: bool) -> Vec<(String, bool)> {
    agents
        .iter()
        .filter(|run| run.can_stop(stoppable))
        .map(|run| (run.part_id.clone(), run.delegated))
        .collect()
}

fn child_run(child: &Chat, state: RunState) -> AgentRun {
    AgentRun {
        part_id: child.id.clone(),
        doc_id: child.id.clone(),
        title: child_title(child),
        model: None,
        reasoning: None,
        state,
        started_ms: child.created_at.timestamp_millis(),
        delegated: false,
        harness: None,
        parent: None,
    }
}

fn run_state_text(run: &AgentRun, now_ms: i64) -> String {
    match run.state {
        RunState::Working => elapsed_label((now_ms - run.started_ms) / 1000),
        RunState::Completed => "Done".to_string(),
        RunState::Stopped => "Stopped".to_string(),
        RunState::Failed => "Failed".to_string(),
    }
}

fn model_tag(tag: String, harness: Option<HarnessId>, theme: &Theme) -> Div {
    div()
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .px(px(5.0))
        .py(px(1.0))
        .rounded(px(5.0))
        .bg(theme.wash(0.08))
        .text_size(crate::typography::ui_rems(11.0))
        .text_color(theme.text_muted)
        .when_some(harness, |el, harness| {
            let (path, tint) = crate::pickers::harness_brand_icon(harness);
            el.child(
                icon(path)
                    .size(px(11.0))
                    .text_color(tint.unwrap_or(theme.text_muted)),
            )
        })
        .child(SharedString::from(tag))
}

fn detail_chip(key: &'static str, value: String, theme: &Theme) -> Div {
    div()
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .px(px(6.0))
        .py(px(1.0))
        .rounded(px(5.0))
        .bg(theme.wash(0.08))
        .text_size(crate::typography::ui_rems(11.0))
        .child(div().text_color(theme.text_faint).child(key))
        .child(
            div()
                .text_color(theme.text_muted)
                .child(SharedString::from(value)),
        )
}

fn detail_link(id: &'static str, label: &'static str, color: Hsla) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .cursor_pointer()
        .text_color(color)
        .hover(|s| s.opacity(0.75))
        .child(label)
}

fn detail_head(
    glyph: AnyElement,
    title: SharedString,
    state: String,
    stop: Option<AnyElement>,
    theme: &Theme,
    cx: &mut Context<Composer>,
) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .pr(px(4.0))
        .pb(px(6.0))
        .child(
            div()
                .id("background-detail-back")
                .debug_selector(|| "background-detail-back".into())
                .size(px(24.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(|s| s.bg(theme.glass_hover()))
                .child(
                    icon(icons::ALT_ARROW_LEFT)
                        .size(px(12.0))
                        .text_color(theme.text_muted),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.background.detail = None;
                    cx.notify();
                })),
        )
        .child(glyph)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(crate::typography::ui_rems(13.0))
                .text_color(theme.text)
                .child(title),
        )
        .child(
            div()
                .flex_none()
                .text_size(crate::typography::ui_rems(12.0))
                .text_color(theme.text_muted)
                .child(SharedString::from(state)),
        )
        .child(stop_slot(stop))
}

fn stop_slot(stop: Option<AnyElement>) -> Div {
    div()
        .min_w(px(20.0))
        .h(px(20.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_end()
        .children(stop)
}

pub(super) fn pill_frame(id: &'static str, theme: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .h(px(PILL_HEIGHT))
        .px(px(10.0))
        .rounded_full()
        .border_1()
        .border_color(theme.composer_surface_border())
        .bg(theme.composer_surface_bg())
        .text_size(crate::typography::ui_rems(13.0))
        .text_color(theme.text_muted)
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(|s| s.border_color(theme.border_strong).text_color(theme.text))
        .active(|s| s.opacity(0.85))
}

pub(super) fn frosted_pill(pill: impl IntoElement, opacity: f32) -> crate::frost::Frosted {
    crate::frost::frosted(PILL_HEIGHT / 2.0, crate::frost::MENU_BLUR * opacity, pill)
}

pub(super) fn pill_leading(glyph: AnyElement) -> Div {
    div().flex().items_center().mr(px(4.0)).child(glyph)
}

fn stop_button(id: String, theme: &Theme) -> Stateful<Div> {
    div()
        .id(SharedString::from(id.clone()))
        .debug_selector(move || id.clone())
        .size(px(20.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.0))
        .cursor_pointer()
        .text_color(theme.text_faint)
        .hover(|s| s.bg(theme.glass_hover()).text_color(theme.danger))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            icon(icons::STOP)
                .size(px(10.0))
                .text_color(theme.text_faint),
        )
}

fn glyph(key: impl Into<SharedString>, state: RunState, fade: f32, theme: &Theme) -> AnyElement {
    let tint = theme.text_faint;
    let dots_opacity = if state == RunState::Working {
        1.0
    } else {
        1.0 - fade
    };
    let dots = (dots_opacity > 0.0).then(|| {
        let grid = if state == RunState::Working {
            DotGrid {
                key: key.into(),
                tint,
            }
            .into_any_element()
        } else {
            dot_cells(FRAMES[0], tint).into_any_element()
        };
        layer().opacity(dots_opacity).child(grid)
    });
    let outcome = (state != RunState::Working).then(|| {
        let (path, color) = match state {
            RunState::Failed => (icons::CLOSE, theme.danger),
            RunState::Stopped => (icons::STOP, theme.text_muted),
            _ => (icons::CHECK, theme.success),
        };
        layer()
            .opacity(fade)
            .child(icon(path).size(px(11.0)).text_color(color))
    });
    div()
        .relative()
        .flex_none()
        .size(px(GLYPH_SIZE))
        .children(dots)
        .children(outcome)
        .into_any_element()
}

fn layer() -> Div {
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
}

fn dot_cells(frame: u16, tint: Hsla) -> Div {
    div()
        .size(px(DOT_BOX))
        .flex()
        .flex_col()
        .gap(px(DOT_GAP))
        .children((0..3).map(move |row| {
            div()
                .flex()
                .flex_row()
                .gap(px(DOT_GAP))
                .children((0..3).map(move |col| {
                    div().size(px(DOT_SIZE)).rounded_full().bg(tint).opacity(
                        if dot_lit(frame, row * 3 + col) {
                            1.0
                        } else {
                            0.0
                        },
                    )
                }))
        }))
}

#[derive(IntoElement)]
struct DotGrid {
    key: SharedString,
    tint: Hsla,
}

impl RenderOnce for DotGrid {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let view = window.with_global_id(self.key.into(), |id, window| {
            window.with_element_state(id, |previous: Option<Entity<DotGridView>>, _| {
                let view = previous.unwrap_or_else(|| cx.new(|_| DotGridView { tint: self.tint }));
                view.update(cx, |view, cx| {
                    if view.tint != self.tint {
                        view.tint = self.tint;
                        cx.notify();
                    }
                });
                (view.clone(), view)
            })
        });
        view.cached(
            gpui::StyleRefinement::default()
                .w(px(DOT_BOX))
                .h(px(DOT_BOX)),
        )
    }
}

struct DotGridView {
    tint: Hsla,
}

impl Render for DotGridView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let phase = motion::pulse_delta_slow(&DOT_GRID_CYCLE, cx.entity_id(), cx);
        let frame = ((phase * FRAMES.len() as f32) as usize).min(FRAMES.len() - 1);
        dot_cells(FRAMES[frame], self.tint)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeron_doc::{MessageRole, SubagentStatus};
    use zeron_proto::{BackgroundTaskKind, Session, SessionStatus, ToolCall};

    fn entry(
        id: &str,
        role: MessageRole,
        created_at: i64,
        parts: Vec<MessagePart>,
    ) -> SessionMessageEntry {
        SessionMessageEntry {
            id: id.into(),
            role,
            parts,
            created_at,
            device_id: "dev".into(),
            status: None,
            continuation_of: None,
            from_chat_id: None,
            fork_import: false,
            answers_request_id: None,
            duration_ms: None,
        }
    }

    fn tool(id: &str, call: ToolCall, status: Option<SubagentStatus>) -> MessagePart {
        MessagePart::Tool {
            id: id.into(),
            call,
            is_error: false,
            resolved: true,
            output: None,
            diff: None,
            output_ref: None,
            output_bytes: None,
            diff_ref: None,
            diff_stats: None,
            subagent_ref: Some(format!("c1--sub--{id}")),
            subagent_status: status,
            subagent_tail: None,
            html_render: None,
            delegate_route: None,
        }
    }

    fn spawn(id: &str, name: &str, status: Option<SubagentStatus>) -> MessagePart {
        tool(
            id,
            ToolCall::Unknown {
                name: name.into(),
                input: Some(serde_json::json!({ "model": "opus" })),
            },
            status,
        )
    }

    fn session(status: SessionStatus, held: bool, tasks: Vec<BackgroundTask>) -> Session {
        Session {
            last_completed_turn: None,
            chat_id: "c1".into(),
            device_id: "dev".into(),
            status,
            held_by_subagents: held,
            compacting: false,
            background_tasks: tasks,
            started_at: None,
            updated_at: chrono::Utc::now(),
            running_subagents: 0,
            compact_plan: None,
            classifying: None,
            last_turn_outcome: Default::default(),
            terminal_activity: Default::default(),
        }
    }

    fn task(id: &str, kind: BackgroundTaskKind) -> BackgroundTask {
        BackgroundTask {
            id: id.into(),
            kind,
            command: format!("cargo test {id}"),
            started_at: chrono::Utc::now(),
            ..Default::default()
        }
    }

    fn claude_chat(id: &str) -> zeron_proto::Chat {
        zeron_proto::Chat {
            id: id.into(),
            device_id: "dev".into(),
            title: Some(id.into()),
            archived: false,
            cwd: None,
            branch: None,
            checkout_id: None,
            source_context: None,
            config: Some(zeron_proto::ChatConfig {
                harness: HarnessId::ClaudeCode,
                model: None,
                reasoning: None,
                model_options: Default::default(),
                sandbox: zeron_proto::SandboxLevel::WorkspaceWrite,
                instructions: String::new(),
                profile_id: None,
                instance_id: None,
                disabled_tools: Vec::new(),
                access: Default::default(),
                plan: false,
            }),
            last_message_preview: None,
            last_message_at: None,
            created_at: chrono::Utc::now(),
            harness_session_id: None,
            harness_session_cwd: None,
            space_id: None,
            last_seen_at: None,
            room_gen: None,
            sidechat_source_chat_id: None,
            created_by_chat_id: None,
            forked_from_chat_id: None,
            previous_harnesses: Vec::new(),
            delegation: None,
            settle: None,
            unsettled_at: None,
            harness_session_harness: Default::default(),
        }
    }

    fn host_device() -> zeron_proto::Device {
        serde_json::from_value(serde_json::json!({
            "id": "dev", "name": "Dev", "platform": "macos",
            "capabilities": [zeron_proto::capabilities::BACKGROUND_TASKS_V1]
        }))
        .unwrap()
    }

    #[gpui::test]
    fn pills_render_above_the_composer_and_the_roster_opens(cx: &mut gpui::TestAppContext) {
        use std::{cell::RefCell, rc::Rc};

        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
            crate::app_menus::init(cx);
            crate::history::init(
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                cx,
            );
            crate::settings::init(crate::settings::UiSettings::default(), dir.path(), cx);
        });
        let state = cx.new(|_| crate::state::AppState::new());
        state.update(cx, |state, _| {
            state.chats = vec![
                claude_chat("c1"),
                zeron_proto::Chat {
                    forked_from_chat_id: Some("c1".into()),
                    ..claude_chat("f1")
                },
            ];
            state.selected_chat = Some("c1".into());
            state.devices = vec![host_device()];
            state.sessions = vec![
                session(
                    SessionStatus::Working,
                    true,
                    vec![BackgroundTask {
                        output_path: Some("/tmp/shell-1.output".into()),
                        ..task("shell-1", BackgroundTaskKind::Shell)
                    }],
                ),
                Session {
                    chat_id: "f1".into(),
                    ..session(SessionStatus::Working, false, Vec::new())
                },
            ];
            state.transcript = vec![
                entry("u1", MessageRole::User, 1, vec![]),
                entry(
                    "a1",
                    MessageRole::Assistant,
                    2,
                    vec![spawn(
                        "p1",
                        "Agent: audit drag",
                        Some(SubagentStatus::Running),
                    )],
                ),
            ];
        });
        let composer = cx.new(|cx| Composer::new(state.clone(), cx));
        let opened: Rc<RefCell<Vec<(String, String, bool)>>> = Rc::default();
        let threads: Rc<RefCell<Vec<String>>> = Rc::default();
        let sink = opened.clone();
        let thread_sink = threads.clone();
        cx.update(|cx| {
            cx.subscribe(&composer, move |_, event: &ComposerEvent, _| match event {
                ComposerEvent::OpenSubagent {
                    chat_id,
                    doc_id,
                    frozen,
                    ..
                } => sink
                    .borrow_mut()
                    .push((chat_id.clone(), doc_id.clone(), *frozen)),
                ComposerEvent::OpenChildChat(id) => thread_sink.borrow_mut().push(id.clone()),
                _ => {}
            })
            .detach()
        });
        struct Fixture(Entity<Composer>);
        impl Render for Fixture {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .justify_end()
                    .child(self.0.clone())
            }
        }
        let (_fixture, cx) = cx.add_window_view(|_, _| Fixture(composer.clone()));
        cx.run_until_parked();

        let agents_pill = cx
            .debug_bounds("background-agents-pill")
            .expect("agents pill painted");
        let shells_pill = cx
            .debug_bounds("background-shells-pill")
            .expect("shells pill painted");
        assert!(shells_pill.left() > agents_pill.right());
        assert!(cx.debug_bounds("background-roster").is_none());
        composer.read_with(cx, |composer, cx| {
            let (agents, shells) = composer.stoppable_runs(cx);
            assert_eq!(agents, [("p1".to_string(), false)]);
            assert_eq!(shells, ["shell-1"]);
            assert!(composer.can_stop_background(cx));
        });

        cx.simulate_click(agents_pill.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("background-roster").is_some());
        assert!(cx.debug_bounds("background-agents-pill").is_some());
        assert!(cx.debug_bounds("background-roster-stop-all").is_some());
        let child = cx
            .debug_bounds("background-roster-child-0")
            .expect("a running fork of this chat gets a roster row");
        cx.simulate_click(child.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert_eq!(threads.borrow().as_slice(), ["f1"]);
        assert!(cx.debug_bounds("background-roster-stop-0").is_some());
        assert!(cx.debug_bounds("background-roster-tokens-0").is_none());
        composer.update(cx, |composer, cx| {
            composer
                .background
                .tokens
                .get_mut("c1--sub--p1")
                .expect("the open roster watches each run's tokens")
                .count = Some(29_216);
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("background-roster-tokens-0").is_some());
        assert!(cx.debug_bounds("background-roster-row-1").is_none());
        composer.update(cx, |composer, cx| {
            composer
                .background
                .walks
                .get_mut("c1--sub--p1")
                .expect("each listed run's transcript is walked")
                .entries = vec![entry(
                "s0",
                MessageRole::Assistant,
                3,
                vec![spawn(
                    "n1",
                    "Agent: nested probe",
                    Some(SubagentStatus::Running),
                )],
            )];
            cx.notify();
        });
        cx.run_until_parked();
        let parent = cx
            .debug_bounds("background-roster-title-0")
            .expect("parent row painted");
        let nested = cx
            .debug_bounds("background-roster-title-1")
            .expect("a nested subagent gets its own row");
        assert!(nested.top() > parent.top());
        assert!(nested.left() > parent.left(), "nested rows indent");
        composer.read_with(cx, |composer, _| {
            assert!(composer.background.walks.contains_key("c1--sub--n1"));
        });

        let row = cx
            .debug_bounds("background-roster-row-0")
            .expect("roster row painted");
        cx.simulate_click(row.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(opened.borrow().is_empty());
        assert!(cx.debug_bounds("background-detail").is_some());
        assert!(cx.debug_bounds("background-roster-row-0").is_none());
        assert!(cx.debug_bounds("background-detail-tokens").is_some());
        assert!(cx.debug_bounds("background-detail-empty").is_some());
        composer.update(cx, |composer, cx| {
            let feed = composer
                .background
                .detail
                .as_mut()
                .and_then(|detail| detail.feed.as_mut())
                .expect("the detail owns a feed");
            assert_eq!(feed.doc_id, "c1--sub--p1");
            feed.entries = vec![entry(
                "s1",
                MessageRole::Assistant,
                3,
                vec![MessagePart::Reasoning {
                    id: "r0".into(),
                    text: "think".into(),
                }],
            )];
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("background-detail-log").is_some());
        assert!(cx.debug_bounds("background-detail-empty").is_none());
        let open_pane = cx
            .debug_bounds("background-detail-open-pane")
            .expect("open in pane painted");
        cx.simulate_click(open_pane.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert_eq!(
            opened.borrow().as_slice(),
            [("c1".to_string(), "c1--sub--p1".to_string(), false)]
        );
        let back = cx
            .debug_bounds("background-detail-back")
            .expect("back painted");
        cx.simulate_click(back.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("background-detail").is_none());
        assert!(cx.debug_bounds("background-roster-row-0").is_some());

        let close = cx
            .debug_bounds("background-roster-close")
            .expect("close painted");
        cx.simulate_click(close.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("background-roster").is_none());
        assert!(cx.debug_bounds("background-agents-pill").is_some());
        finish_exit(cx, &composer);
        composer.read_with(cx, |composer, _| {
            assert!(composer.background.tokens.is_empty())
        });

        cx.simulate_click(agents_pill.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("background-roster").is_some());
        cx.simulate_click(gpui::point(px(4.0), px(4.0)), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("background-roster").is_none(),
            "a click outside the roster closes it"
        );
        finish_exit(cx, &composer);
        cx.simulate_click(agents_pill.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(composer.update(cx, |composer, cx| composer.dismiss_corner_card(cx)));
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("background-roster").is_none(),
            "escape closes it"
        );
        assert!(!composer.update(cx, |composer, cx| composer.dismiss_corner_card(cx)));
        finish_exit(cx, &composer);

        let shells_pill = cx
            .debug_bounds("background-shells-pill")
            .expect("shells pill painted");
        cx.simulate_click(shells_pill.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("background-shells-menu").is_some());
        assert!(cx.debug_bounds("background-shell-row-0").is_some());
        assert!(cx.debug_bounds("background-shell-stop-0").is_some());
        assert!(cx.debug_bounds("background-shell-line-0").is_none());

        composer.update(cx, |composer, cx| {
            composer
                .background
                .outputs
                .get_mut("shell-1")
                .unwrap()
                .lines = vec![
                "Compiling zeron".into(),
                "   Finished dev".into(),
                "".into(),
            ];
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("background-shell-line-0").is_some());

        let row = cx
            .debug_bounds("background-shell-row-0")
            .expect("shell row painted");
        cx.simulate_click(row.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("background-shells-menu").is_some());
        assert!(cx.debug_bounds("background-detail-log").is_some());
        assert!(cx.debug_bounds("background-shell-row-0").is_none());
        assert!(cx.debug_bounds("background-detail-stop").is_some());
        assert!(cx.debug_bounds("background-detail-empty").is_none());
        composer.update(cx, |composer, cx| {
            composer
                .background
                .outputs
                .get_mut("shell-1")
                .unwrap()
                .lines
                .clear();
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("background-detail-empty").is_some());
        let back = cx
            .debug_bounds("background-detail-back")
            .expect("back painted");
        cx.simulate_click(back.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("background-detail").is_none());
        assert!(cx.debug_bounds("background-shell-row-0").is_some());
    }

    fn finish_exit(cx: &mut gpui::VisualTestContext, composer: &Entity<Composer>) {
        let span = motion::MENU_OUT.total() + Duration::from_millis(30);
        std::thread::sleep(span);
        cx.executor().advance_clock(span);
        composer.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
    }

    #[gpui::test]
    fn a_chat_bound_composer_shows_and_stops_its_own_chats_runs(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
            crate::app_menus::init(cx);
            crate::history::init(
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                cx,
            );
            crate::settings::init(crate::settings::UiSettings::default(), dir.path(), cx);
        });
        let state = cx.new(|_| crate::state::AppState::new());
        state.update(cx, |state, _| {
            state.chats = vec![claude_chat("c1"), claude_chat("c2")];
            state.selected_chat = Some("c1".into());
            state.sessions = vec![Session {
                chat_id: "c2".into(),
                ..session(
                    SessionStatus::Working,
                    false,
                    vec![task("shell-1", BackgroundTaskKind::Shell)],
                )
            }];
            state.transcript = vec![
                entry("u1", MessageRole::User, 1, vec![]),
                entry(
                    "a1",
                    MessageRole::Assistant,
                    2,
                    vec![spawn("p1", "Agent: main", Some(SubagentStatus::Running))],
                ),
            ];
        });
        let composer = cx.new(|cx| Composer::for_chat(state.clone(), "c2".into(), cx));
        struct Fixture(Entity<Composer>);
        impl Render for Fixture {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .justify_end()
                    .child(self.0.clone())
            }
        }
        let (_fixture, cx) = cx.add_window_view(|_, _| Fixture(composer.clone()));
        cx.run_until_parked();
        assert!(cx.debug_bounds("background-shells-pill").is_some());
        assert!(cx.debug_bounds("background-agents-pill").is_none());
        let counts = |cx: &mut gpui::VisualTestContext| {
            composer.read_with(cx, |composer, cx| {
                let (agents, shells) = composer.stoppable_runs(cx);
                (agents.len(), shells.len())
            })
        };
        assert_eq!(counts(cx), (0, 0));
        state.update(cx, |state, _| state.devices = vec![host_device()]);
        assert_eq!(counts(cx), (0, 1));

        state.update(cx, |state, _| {
            state.set_subagent_snapshot(
                "c2".into(),
                vec![
                    entry("u2", MessageRole::User, 1, vec![]),
                    entry(
                        "a2",
                        MessageRole::Assistant,
                        2,
                        vec![spawn("p2", "Agent: split", Some(SubagentStatus::Running))],
                    ),
                ],
            );
        });
        composer.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        assert!(cx.debug_bounds("background-agents-pill").is_some());
        assert_eq!(counts(cx), (1, 1));
    }

    #[test]
    fn token_counts_compact() {
        assert_eq!(compact_count(950), "950");
        assert_eq!(compact_count(29_216), "29.2k");
        assert_eq!(compact_count(1_250_000), "1.25M");
    }

    #[test]
    fn detail_rows_group_tools_between_messages_and_can_hide_thinking() {
        let mut grep = tool(
            "t1",
            ToolCall::Exec {
                command: "rg on_drag".into(),
            },
            None,
        );
        if let MessagePart::Tool { output, .. } = &mut grep {
            *output = Some("x".repeat(DETAIL_TEXT_CHARS + 10));
        }
        let entries = vec![
            entry(
                "u1",
                MessageRole::User,
                1,
                vec![MessagePart::Text {
                    id: "t0".into(),
                    text: "scan the fold".into(),
                }],
            ),
            entry(
                "a1",
                MessageRole::Assistant,
                2,
                vec![
                    MessagePart::Reasoning {
                        id: "r0".into(),
                        text: "start at the drop target".into(),
                    },
                    grep,
                    tool(
                        "t2",
                        ToolCall::ReadFile {
                            path: "sidebar.rs".into(),
                        },
                        None,
                    ),
                    MessagePart::Text {
                        id: "t3".into(),
                        text: "Three handlers.\n".into(),
                    },
                    tool(
                        "t4",
                        ToolCall::Exec {
                            command: "cargo test".into(),
                        },
                        None,
                    ),
                    MessagePart::Error {
                        id: "e0".into(),
                        message: "Stopped".into(),
                    },
                ],
            ),
        ];
        let groups = |rows: &[Row]| -> Vec<(String, Vec<String>)> {
            rows.iter()
                .filter_map(|row| match &row.kind {
                    RowKind::ToolGroup { summary, tools, .. } => Some((
                        summary.to_string(),
                        tools.iter().map(|tool| tool.part_id.clone()).collect(),
                    )),
                    _ => None,
                })
                .collect()
        };
        let rows = detail_rows(&entries, false);
        let kinds: Vec<&str> = rows
            .iter()
            .map(|row| match row.kind {
                RowKind::User { .. } => "user",
                RowKind::ToolGroup { .. } => "group",
                RowKind::Markdown { .. } => "text",
                RowKind::ErrorChip { .. } => "error",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, ["user", "group", "text", "group", "error"]);
        assert_eq!(
            groups(&rows),
            [
                (
                    "Thought process · Ran 1 command · read 1 file".to_string(),
                    vec!["r0".to_string(), "t1".into(), "t2".into()],
                ),
                ("Ran 1 command".to_string(), vec!["t4".to_string()]),
            ]
        );
        let RowKind::ToolGroup { tools, .. } = &rows[1].kind else {
            unreachable!()
        };
        let Some(crate::transcript::ToolDetail::Output { lines, .. }) = tools[1].detail.as_deref()
        else {
            panic!("the run keeps its output");
        };
        assert_eq!(lines[0].chars().count(), DETAIL_TEXT_CHARS + 1);
        assert_eq!(live_part(&rows), None);

        let hidden = detail_rows(&entries, true);
        assert_eq!(
            groups(&hidden),
            [
                (
                    "Ran 1 command · read 1 file".to_string(),
                    vec!["t1".to_string(), "t2".into()],
                ),
                ("Ran 1 command".to_string(), vec!["t4".to_string()]),
            ]
        );
        assert_eq!(
            detail_copy_text(&hidden),
            "scan the fold\nRun rg on_drag\nRead sidebar.rs\nThree handlers.\nRun cargo test\nStopped"
        );
        assert_eq!(tool_count(&entries), 3);
    }

    #[test]
    fn the_unfinished_last_tool_is_the_live_step() {
        let mut running = tool(
            "t1",
            ToolCall::Exec {
                command: "bun test".into(),
            },
            None,
        );
        if let MessagePart::Tool { resolved, .. } = &mut running {
            *resolved = false;
        }
        let entries = vec![entry("a1", MessageRole::Assistant, 1, vec![running])];
        assert_eq!(live_part(&detail_rows(&entries, false)), Some("t1"));
    }

    #[test]
    fn dot_frames() {
        let lit: Vec<bool> = (0..9).map(|dot| dot_lit(FRAMES[0], dot)).collect();
        assert_eq!(
            lit,
            [false, true, false, true, true, true, true, false, true]
        );
    }
}
