use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, ClickEvent, Context, Div, Entity, Hsla, MouseButton, SharedString, Stateful,
    Window, div, point, prelude::*, px,
};
use zeron_doc::{MessagePart, MessageRole, SessionMessageEntry, SubagentStatus};
use zeron_proto::{
    BackgroundTask, BackgroundTaskKind, ChatIndicator, HarnessId, Session, SessionStatus,
};
use zeron_rpc::methods;

use super::{Composer, ComposerEvent, harness_display_name};
use crate::icons::{self, icon};
use crate::motion;
use crate::popover::{self, Popup};
use crate::theme::Theme;

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
const ROSTER_RISE: Duration = Duration::from_millis(150);
const ELAPSED_TICK: Duration = Duration::from_secs(1);
const STOP_TIMEOUT: Duration = Duration::from_secs(15);
const SHELLS_MENU_WIDTH: f32 = 420.0;
const EXPANDED_TAIL_LINES: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunState {
    Working,
    Completed,
    Failed,
}

impl RunState {
    fn word(self) -> &'static str {
        match self {
            RunState::Working => "Working",
            RunState::Completed => "Completed",
            RunState::Failed => "Failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AgentRun {
    pub part_id: String,
    pub doc_id: String,
    pub title: SharedString,
    pub model: Option<String>,
    pub state: RunState,
    pub started_ms: i64,
}

/// Spawn chips from the latest prompt onward, plus any chip still running
/// from an earlier turn. A chip without a status predates the field and has
/// no truthful state to report, so it is left out.
pub(crate) fn agent_runs(transcript: &[SessionMessageEntry]) -> Vec<AgentRun> {
    let turn_start = transcript
        .iter()
        .rposition(|entry| entry.role == MessageRole::User && entry.answers_request_id.is_none())
        .unwrap_or(0);
    transcript
        .iter()
        .enumerate()
        .flat_map(|(ix, entry)| {
            entry.parts.iter().filter_map(move |part| {
                let MessagePart::Tool {
                    id,
                    call,
                    subagent_ref: Some(doc_id),
                    subagent_status: Some(status),
                    ..
                } = part
                else {
                    return None;
                };
                if !call.is_subagent_spawn() {
                    return None;
                }
                let state = match status {
                    SubagentStatus::Running => RunState::Working,
                    SubagentStatus::Done => RunState::Completed,
                    SubagentStatus::Failed => RunState::Failed,
                };
                (ix >= turn_start || state == RunState::Working).then(|| AgentRun {
                    part_id: id.clone(),
                    doc_id: doc_id.clone(),
                    title: crate::transcript::subagent_tab_title(call),
                    model: call.subagent_model().map(str::to_string),
                    state,
                    started_ms: entry.created_at,
                })
            })
        })
        .collect()
}

pub(crate) fn shell_tasks(session: Option<&Session>) -> Vec<BackgroundTask> {
    session
        .map(|session| {
            session
                .background_tasks
                .iter()
                .filter(|task| task.kind == BackgroundTaskKind::Shell)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn aggregate(runs: &[AgentRun]) -> RunState {
    if runs.iter().any(|run| run.state == RunState::Working) {
        RunState::Working
    } else if runs.iter().any(|run| run.state == RunState::Failed) {
        RunState::Failed
    } else {
        RunState::Completed
    }
}

fn working_count(runs: &[AgentRun]) -> usize {
    runs.iter()
        .filter(|run| run.state == RunState::Working)
        .count()
}

pub(crate) fn agents_label(runs: &[AgentRun]) -> String {
    let working = working_count(runs);
    match runs {
        [one] => format!("{} · {}", one.title, one.state.word()),
        _ if working > 0 => format!("{working} Working"),
        _ => format!("{} Completed", runs.len()),
    }
}

pub(crate) fn roster_heading(runs: &[AgentRun]) -> String {
    match working_count(runs) {
        0 => "Subagents".to_string(),
        working => format!("{working} of {} working", runs.len()),
    }
}

pub(crate) fn shells_label(count: usize) -> String {
    format!("{count} Running")
}

pub(crate) fn elapsed_label(secs: i64) -> String {
    let secs = secs.max(0);
    if secs < 60 {
        return format!("{secs}s");
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins}m {:02}s", secs % 60);
    }
    format!("{}h {:02}m", mins / 60, mins % 60)
}

pub(crate) fn latest_line(lines: &[String]) -> Option<&str> {
    lines
        .iter()
        .rev()
        .map(|line| line.trim())
        .find(|line| !line.is_empty())
}

pub(crate) fn expanded_tail(lines: &[String]) -> &[String] {
    &lines[lines.len().saturating_sub(EXPANDED_TAIL_LINES)..]
}

pub(crate) fn dot_lit(frame: u16, dot: usize) -> bool {
    frame & (1 << (8 - dot)) != 0
}

pub(crate) fn sidebar_ring(indicator: ChatIndicator, session: Option<&Session>) -> Option<usize> {
    let session = session?;
    let held = session.held_by_subagents && indicator == ChatIndicator::Working;
    let idle = session.status == SessionStatus::Idle
        && !session.background_tasks.is_empty()
        && matches!(indicator, ChatIndicator::Idle | ChatIndicator::Completed);
    (held || idle).then_some(session.background_tasks.len())
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
    roster_opened: Option<Instant>,
    shells: Popup<()>,
    agents_glyph: Option<(RunState, Option<Instant>)>,
    tick: Option<gpui::Task<()>>,
    outputs: HashMap<String, ShellOutput>,
    expanded: Option<String>,
}

struct ShellOutput {
    lines: Vec<String>,
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

impl Composer {
    pub(super) fn render_background_corner(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.chat_override.is_some() {
            return None;
        }
        let (chat_id, harness) = self
            .bound_chat_row(cx)
            .map(|chat| (chat.id.clone(), chat.config.as_ref().map(|c| c.harness)))?;
        let (agents, shells) = {
            let state = self.state.read(cx);
            (
                agent_runs(&state.transcript),
                shell_tasks(state.session_for(&chat_id)),
            )
        };
        if self.background.chat.as_deref() != Some(chat_id.as_str()) {
            self.background = BackgroundUi {
                chat: Some(chat_id),
                ..Default::default()
            };
        }
        if agents.is_empty() {
            self.background.roster_opened = None;
            self.background.agents_glyph = None;
        }
        if shells.is_empty() {
            self.background.shells = Popup::default();
        }
        if agents.is_empty() && shells.is_empty() {
            return None;
        }

        let theme = Theme::of(cx).clone();
        let stoppable = harness == Some(HarnessId::ClaudeCode);
        let now = Instant::now();
        let now_ms = chrono::Utc::now().timestamp_millis();
        let reduced = motion::reduced_motion(cx);

        let overall = aggregate(&agents);
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
        let rise = self
            .background
            .roster_opened
            .map(|opened| tween_progress(Some(opened), ROSTER_RISE, reduced, now));
        if fade < 1.0 || rise.is_some_and(|rise| rise < 1.0) {
            window.request_animation_frame();
        }
        let shells_mounted = self.background.shells.get().is_some();
        self.sync_output_watches(if shells_mounted { &shells } else { &[] }, cx);
        let ticking = (rise.is_some() && overall == RunState::Working) || shells_mounted;
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

        let roster = rise
            .map(|rise| self.render_roster(&agents, harness, stoppable, rise, now_ms, &theme, cx));
        let agents_pill = (rise.is_none() && !agents.is_empty()).then(|| {
            pill_frame("background-agents-pill", &theme)
                .debug_selector(|| "background-agents-pill".into())
                .child(pill_leading(glyph(
                    "background-agents-glyph",
                    overall,
                    fade,
                    &theme,
                )))
                .child(div().child(SharedString::from(agents_label(&agents))))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.background.roster_opened = Some(Instant::now());
                    if this.background.shells.begin_close() {
                        popover::reap_popup(cx, |this: &mut Composer| &mut this.background.shells);
                    }
                    cx.notify();
                }))
        });
        let shell_pill = (!shells.is_empty())
            .then(|| self.render_shell_pill(&shells, stoppable, now_ms, &theme, cx));

        let row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .children(agents_pill)
            .children(shell_pill);
        Some(
            div()
                .debug_selector(|| "background-corner".into())
                .flex()
                .flex_col()
                .gap(px(6.0))
                .when(rise.is_none() || !shells.is_empty(), |el| el.child(row))
                .children(roster)
                .into_any_element(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_roster(
        &self,
        agents: &[AgentRun],
        harness: Option<HarnessId>,
        stoppable: bool,
        rise: f32,
        now_ms: i64,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let running: Vec<String> = agents
            .iter()
            .filter(|run| run.state == RunState::Working)
            .map(|run| run.part_id.clone())
            .collect();
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
                    .child(SharedString::from(roster_heading(agents))),
            )
            .when(stoppable && !running.is_empty(), |el| {
                el.child(
                    div()
                        .id("background-roster-stop-all")
                        .debug_selector(|| "background-roster-stop-all".into())
                        .cursor_pointer()
                        .text_color(theme.text_muted)
                        .hover(|s| s.text_color(theme.text))
                        .child("Stop all")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            for task_id in &running {
                                this.stop_background_task(task_id.clone(), cx);
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
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.background.roster_opened = None;
                        cx.notify();
                    })),
            );
        let chat_id = self.background.chat.clone().unwrap_or_default();
        let rows = agents.iter().enumerate().map(|(ix, run)| {
            let open = {
                let chat_id = chat_id.clone();
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
            let state_text = match run.state {
                RunState::Working => elapsed_label((now_ms - run.started_ms) / 1000),
                RunState::Completed => "Done".to_string(),
                RunState::Failed => "Failed".to_string(),
            };
            let tag = run
                .model
                .clone()
                .or_else(|| harness.map(|h| harness_display_name(h).to_string()));
            let stop = (stoppable && run.state == RunState::Working).then(|| {
                let task_id = run.part_id.clone();
                stop_button(format!("background-roster-stop-{ix}"), theme).on_click(cx.listener(
                    move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.stop_background_task(task_id.clone(), cx);
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
                .child(div().flex_1().min_w_0().truncate().child(run.title.clone()))
                .when_some(tag, |el, tag| {
                    el.child(
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
                            .child(SharedString::from(tag)),
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
        div()
            .debug_selector(|| "background-roster".into())
            .w_full()
            .relative()
            .top(px(4.0 * (1.0 - rise)))
            .opacity(rise)
            .rounded(px(14.0))
            .border_1()
            .border_color(theme.border)
            .bg(popover::surface_bg(theme))
            .when(!theme.is_frost(), |el| el.shadow_lg())
            .overflow_hidden()
            .pb(px(4.0))
            .child(header)
            .children(rows)
            .into_any_element()
    }

    fn render_shell_pill(
        &self,
        shells: &[BackgroundTask],
        stoppable: bool,
        now_ms: i64,
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
                cx.notify();
            }));
        let menu = self.background.shells.get().map(|_| {
            let rows = shells.iter().enumerate().map(|(ix, task)| {
                let elapsed = task
                    .started_at
                    .map(|at| elapsed_label((now_ms - at.timestamp_millis()) / 1000));
                let stop = stoppable.then(|| {
                    let task_id = task.id.clone();
                    stop_button(format!("background-shell-stop-{ix}"), theme).on_click(cx.listener(
                        move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.stop_background_task(task_id.clone(), cx);
                        },
                    ))
                });
                let lines = self
                    .background
                    .outputs
                    .get(&task.id)
                    .map(|output| output.lines.as_slice())
                    .unwrap_or_default();
                let expanded = self.background.expanded.as_deref() == Some(task.id.as_str());
                let latest = (!expanded)
                    .then(|| latest_line(lines))
                    .flatten()
                    .map(|line| {
                        div()
                            .debug_selector(move || format!("background-shell-line-{ix}"))
                            .pl(px(GLYPH_SIZE + 8.0))
                            .truncate()
                            .font_family(theme.font_mono.clone())
                            .text_size(crate::typography::ui_rems(10.5))
                            .text_color(theme.text_muted)
                            .child(SharedString::from(line.to_owned()))
                    });
                let tail = (expanded && !lines.is_empty()).then(|| {
                    div()
                        .debug_selector(move || format!("background-shell-tail-{ix}"))
                        .ml(px(GLYPH_SIZE + 8.0))
                        .px(px(8.0))
                        .py(px(6.0))
                        .rounded(px(6.0))
                        .bg(theme.wash(0.06))
                        .font_family(theme.font_mono.clone())
                        .text_size(crate::typography::ui_rems(10.5))
                        .text_color(theme.text_muted)
                        .children(expanded_tail(lines).iter().map(|line| {
                            div()
                                .truncate()
                                .child(SharedString::from(if line.is_empty() {
                                    " ".to_owned()
                                } else {
                                    line.clone()
                                }))
                        }))
                });
                let toggle = {
                    let task_id = task.id.clone();
                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.background.expanded = (this.background.expanded.as_deref()
                            != Some(task_id.as_str()))
                        .then(|| task_id.clone());
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
                            .child(SharedString::from(task.label.clone())),
                    )
                    .children(elapsed.map(|elapsed| {
                        div()
                            .flex_none()
                            .text_size(crate::typography::ui_rems(11.5))
                            .text_color(theme.text_muted)
                            .child(SharedString::from(elapsed))
                    }))
                    .child(div().size(px(20.0)).flex_none().children(stop));
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
                    .children(latest)
                    .children(tail)
                    .on_click(toggle)
            });
            let content = popover::popover_card(theme)
                .debug_selector(|| "background-shells-menu".into())
                .w(px(SHELLS_MENU_WIDTH))
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    if this.background.shells.begin_close() {
                        popover::reap_popup(cx, |this: &mut Composer| &mut this.background.shells);
                        cx.notify();
                    }
                }))
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
                .into_any_element();
            popover::anchored_menu_above_at(
                "background-shells-menu",
                point(px(0.0), px(0.0)),
                content,
                self.background.shells.closing_since(),
            )
        });
        div()
            .relative()
            .child(pill)
            .children(menu)
            .into_any_element()
    }

    fn sync_output_watches(&mut self, shells: &[BackgroundTask], cx: &mut Context<Self>) {
        let watched: Vec<&str> = shells
            .iter()
            .filter(|task| task.output_path.is_some())
            .map(|task| task.id.as_str())
            .collect();
        self.background
            .outputs
            .retain(|id, _| watched.contains(&id.as_str()));
        if self
            .background
            .expanded
            .as_deref()
            .is_some_and(|id| !shells.iter().any(|task| task.id == id))
        {
            self.background.expanded = None;
        }
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

    fn stop_background_task(&mut self, task_id: String, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            tracing::warn!(task_id, "stop background task: engine not connected");
            return;
        };
        let Some((chat_id, device_id)) = self
            .bound_chat_row(cx)
            .map(|chat| (chat.id.clone(), chat.device_id.clone()))
        else {
            return;
        };
        let params = serde_json::json!({
            "chatId": chat_id,
            "taskId": task_id,
            "targetDeviceId": device_id,
        });
        cx.spawn(async move |_, cx| {
            if let Err(error) = crate::attachments::call_with_timeout(
                &engine,
                cx.background_executor(),
                methods::STOP_BACKGROUND_TASK,
                params,
                STOP_TIMEOUT,
            )
            .await
            {
                tracing::warn!(%error, "stop background task failed");
            }
        })
        .detach();
    }
}

fn pill_frame(id: &'static str, theme: &Theme) -> Stateful<Div> {
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

fn pill_leading(glyph: AnyElement) -> Div {
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
        .child(icon(icons::STOP).size(px(10.0)))
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
        let (path, color) = if state == RunState::Failed {
            (icons::CLOSE, theme.danger)
        } else {
            (icons::CHECK, theme.success)
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
    use zeron_proto::ToolCall;

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
        }
    }

    fn task(id: &str, kind: BackgroundTaskKind) -> BackgroundTask {
        BackgroundTask {
            id: id.into(),
            kind,
            label: format!("cargo test {id}"),
            started_at: None,
            output_path: None,
        }
    }

    fn run(title: &str, state: RunState) -> AgentRun {
        AgentRun {
            part_id: title.into(),
            doc_id: title.into(),
            title: SharedString::from(title.to_string()),
            model: None,
            state,
            started_ms: 0,
        }
    }

    #[test]
    fn spawn_chips_of_the_latest_turn_and_earlier_running_ones_become_runs() {
        let transcript = vec![
            entry("u1", MessageRole::User, 1, vec![]),
            entry(
                "a1",
                MessageRole::Assistant,
                2,
                vec![
                    spawn(
                        "old-done",
                        "Agent: old finished",
                        Some(SubagentStatus::Done),
                    ),
                    spawn(
                        "old-live",
                        "Agent: old still going",
                        Some(SubagentStatus::Running),
                    ),
                ],
            ),
            entry("u2", MessageRole::User, 3, vec![]),
            entry(
                "a2",
                MessageRole::Assistant,
                4,
                vec![
                    spawn(
                        "new-live",
                        "Agent: audit drag",
                        Some(SubagentStatus::Running),
                    ),
                    spawn(
                        "new-fail",
                        "Agent: rewrite fixture",
                        Some(SubagentStatus::Failed),
                    ),
                    spawn("legacy", "Agent: no status", None),
                    tool(
                        "stray",
                        ToolCall::Exec {
                            command: "ls".into(),
                        },
                        Some(SubagentStatus::Running),
                    ),
                ],
            ),
        ];
        let runs = agent_runs(&transcript);
        let ids: Vec<&str> = runs.iter().map(|run| run.part_id.as_str()).collect();
        assert_eq!(ids, ["old-live", "new-live", "new-fail"]);
        assert_eq!(runs[0].state, RunState::Working);
        assert_eq!(runs[0].started_ms, 2);
        assert_eq!(runs[1].title.as_ref(), "audit drag");
        assert_eq!(runs[1].model.as_deref(), Some("opus"));
        assert_eq!(runs[1].doc_id, "c1--sub--new-live");
        assert_eq!(runs[2].state, RunState::Failed);
    }

    #[test]
    fn an_answer_entry_does_not_start_a_new_turn_window() {
        let mut answer = entry("u2", MessageRole::User, 3, vec![]);
        answer.answers_request_id = Some("req".into());
        let transcript = vec![
            entry("u1", MessageRole::User, 1, vec![]),
            entry(
                "a1",
                MessageRole::Assistant,
                2,
                vec![spawn("done", "Agent: finished", Some(SubagentStatus::Done))],
            ),
            answer,
        ];
        assert_eq!(agent_runs(&transcript).len(), 1);
    }

    #[test]
    fn labels_for_one_many_and_settled_runs() {
        assert_eq!(
            agents_label(&[run("Audit drag", RunState::Working)]),
            "Audit drag · Working"
        );
        assert_eq!(
            agents_label(&[run("Audit drag", RunState::Completed)]),
            "Audit drag · Completed"
        );
        assert_eq!(
            agents_label(&[run("Audit drag", RunState::Failed)]),
            "Audit drag · Failed"
        );
        let mixed = [
            run("a", RunState::Working),
            run("b", RunState::Completed),
            run("c", RunState::Working),
        ];
        assert_eq!(agents_label(&mixed), "2 Working");
        assert_eq!(roster_heading(&mixed), "2 of 3 working");
        assert_eq!(aggregate(&mixed), RunState::Working);
        let settled = [run("a", RunState::Completed), run("b", RunState::Failed)];
        assert_eq!(agents_label(&settled), "2 Completed");
        assert_eq!(roster_heading(&settled), "Subagents");
        assert_eq!(aggregate(&settled), RunState::Failed);
        assert_eq!(
            aggregate(&[run("a", RunState::Completed)]),
            RunState::Completed
        );
        assert_eq!(shells_label(2), "2 Running");
    }

    #[test]
    fn shell_list_keeps_only_shell_tasks() {
        let session = session(
            SessionStatus::Idle,
            false,
            vec![
                task("s1", BackgroundTaskKind::Shell),
                task("a1", BackgroundTaskKind::Agent),
                task("o1", BackgroundTaskKind::Other),
                task("s2", BackgroundTaskKind::Shell),
            ],
        );
        let ids: Vec<String> = shell_tasks(Some(&session))
            .into_iter()
            .map(|task| task.id)
            .collect();
        assert_eq!(ids, ["s1", "s2"]);
        assert!(shell_tasks(None).is_empty());
    }

    #[test]
    fn ring_shows_for_idle_background_work_and_for_held_turns() {
        let shell = || vec![task("s1", BackgroundTaskKind::Shell)];
        let idle = session(SessionStatus::Idle, false, shell());
        assert_eq!(sidebar_ring(ChatIndicator::Idle, Some(&idle)), Some(1));
        assert_eq!(sidebar_ring(ChatIndicator::Completed, Some(&idle)), Some(1));
        let quiet = session(SessionStatus::Idle, false, vec![]);
        assert_eq!(sidebar_ring(ChatIndicator::Idle, Some(&quiet)), None);
        let working = session(SessionStatus::Working, false, shell());
        assert_eq!(sidebar_ring(ChatIndicator::Working, Some(&working)), None);
        let held = session(SessionStatus::Working, true, vec![]);
        assert_eq!(sidebar_ring(ChatIndicator::Working, Some(&held)), Some(0));
        assert_eq!(sidebar_ring(ChatIndicator::Errored, Some(&held)), None);
        assert_eq!(sidebar_ring(ChatIndicator::Idle, None), None);
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
                permission_mode: Default::default(),
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
            previous_harnesses: Vec::new(),
        }
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
            state.chats = vec![claude_chat("c1")];
            state.selected_chat = Some("c1".into());
            state.sessions = vec![session(
                SessionStatus::Working,
                true,
                vec![BackgroundTask {
                    output_path: Some("/tmp/shell-1.output".into()),
                    ..task("shell-1", BackgroundTaskKind::Shell)
                }],
            )];
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
        let sink = opened.clone();
        cx.update(|cx| {
            cx.subscribe(&composer, move |_, event: &ComposerEvent, _| {
                if let ComposerEvent::OpenSubagent {
                    chat_id,
                    doc_id,
                    frozen,
                    ..
                } = event
                {
                    sink.borrow_mut()
                        .push((chat_id.clone(), doc_id.clone(), *frozen));
                }
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

        cx.simulate_click(agents_pill.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("background-roster").is_some());
        assert!(cx.debug_bounds("background-agents-pill").is_none());
        assert!(cx.debug_bounds("background-roster-stop-all").is_some());
        assert!(cx.debug_bounds("background-roster-stop-0").is_some());

        let row = cx
            .debug_bounds("background-roster-row-0")
            .expect("roster row painted");
        cx.simulate_click(row.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert_eq!(
            opened.borrow().as_slice(),
            [("c1".to_string(), "c1--sub--p1".to_string(), false)]
        );

        let close = cx
            .debug_bounds("background-roster-close")
            .expect("close painted");
        cx.simulate_click(close.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("background-roster").is_none());
        assert!(cx.debug_bounds("background-agents-pill").is_some());

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
        assert!(cx.debug_bounds("background-shell-tail-0").is_none());

        let row = cx
            .debug_bounds("background-shell-row-0")
            .expect("shell row painted");
        cx.simulate_click(row.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("background-shells-menu").is_some());
        assert!(cx.debug_bounds("background-shell-tail-0").is_some());
        assert!(cx.debug_bounds("background-shell-line-0").is_none());
    }

    #[test]
    fn the_live_line_is_the_last_non_blank_one_and_the_tail_is_capped() {
        let lines: Vec<String> = (0..20).map(|i| format!("line {i}")).collect();
        assert_eq!(expanded_tail(&lines).len(), EXPANDED_TAIL_LINES);
        assert_eq!(expanded_tail(&lines)[0], "line 8");
        let with_blank = vec!["  Finished dev ".to_string(), "  ".to_string()];
        assert_eq!(latest_line(&with_blank), Some("Finished dev"));
        assert_eq!(latest_line(&[]), None);
    }

    #[test]
    fn dot_frames_and_elapsed_text() {
        let lit: Vec<bool> = (0..9).map(|dot| dot_lit(FRAMES[0], dot)).collect();
        assert_eq!(
            lit,
            [false, true, false, true, true, true, true, false, true]
        );
        assert_eq!(elapsed_label(7), "7s");
        assert_eq!(elapsed_label(74), "1m 14s");
        assert_eq!(elapsed_label(3_725), "1h 02m");
        assert_eq!(elapsed_label(-3), "0s");
    }
}
