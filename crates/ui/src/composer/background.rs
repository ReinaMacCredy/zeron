use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::{AnyElement, App, Context, Div, SharedString, Stateful, div, prelude::*, px};
use zeron_doc::SessionMessageEntry;
use zeron_doc::background::{agent_runs, agent_tree, latest_prompt, rows_missing, walked_docs};
use zeron_proto::background::{delegate_runs, shell_tasks};
use zeron_proto::{BackgroundTask, HarnessId};
use zeron_rpc::methods;

use super::Composer;
use crate::motion;
use crate::theme::Theme;

const PILL_HEIGHT: f32 = 28.0;
const CARD_RISE: f32 = 10.0;
const CARD_DROP: f32 = 4.0;
const STOP_TIMEOUT: Duration = Duration::from_secs(15);
pub(crate) use zeron_proto::transcript::compact_count;

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
    walks: HashMap<String, AgentWalk>,
    widen: Option<bool>,
}

struct BackgroundRuns {
    chat_id: String,
    harness: Option<HarnessId>,
    agents: Vec<zeron_proto::background::AgentRun>,
    shells: Vec<BackgroundTask>,
    walk: Vec<String>,
    counted: u32,
}

struct AgentWalk {
    entries: Vec<SessionMessageEntry>,
    loaded: bool,
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

fn eased(spec: &motion::MotionSpec, since: Option<Instant>, reduced: bool, now: Instant) -> f32 {
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
    pub(super) fn render_corner(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.sync_background_walks(cx);
        let theme = Theme::of(cx).clone();
        let limit = (!self.side_chat)
            .then(|| self.render_limit_pill(&theme, cx))
            .flatten();
        limit.map(|limit| {
            div()
                .flex()
                .flex_row()
                .items_end()
                .child(div().flex_1().min_w_0())
                .child(limit)
                .into_any_element()
        })
    }

    fn sync_background_walks(&mut self, cx: &mut Context<Self>) {
        let Some(BackgroundRuns {
            chat_id,
            agents,
            walk,
            counted,
            ..
        }) = self.background_runs(cx)
        else {
            return;
        };
        if self.background.chat.as_deref() != Some(chat_id.as_str()) {
            self.background = BackgroundUi {
                chat: Some(chat_id),
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
    }

    fn background_runs(&self, cx: &App) -> Option<BackgroundRuns> {
        let (chat_id, harness) = self.bound_chat_row(cx).map(|chat| {
            (
                chat.id.clone(),
                chat.config.as_ref().map(|config| config.harness),
            )
        })?;
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
        agents.extend(delegate_runs(
            &state.chats,
            &chat_id,
            turn_start.map_or(0, |ix| transcript[ix].created_at),
            pending_review,
        ));
        let walk = walked_docs(transcript, &agents, self.background.widen == Some(true));
        let counted = state.running_subagents_for(&chat_id, chrono::Utc::now());
        let shells = shell_tasks(state.session_for(&chat_id));
        Some(BackgroundRuns {
            chat_id,
            harness,
            agents,
            shells,
            walk,
            counted,
        })
    }

    fn stoppable_runs(&self, cx: &App) -> (Vec<(String, bool)>, Vec<String>) {
        let Some(runs) = self.background_runs(cx) else {
            return Default::default();
        };
        let stoppable = runs.harness == Some(HarnessId::ClaudeCode);
        let agents = runs
            .agents
            .iter()
            .filter(|run| run.can_stop(stoppable))
            .map(|run| (run.part_id.clone(), run.delegated))
            .collect();
        let shells = if stoppable {
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
        self.bound_chat_row(cx)
            .and_then(|chat| chat.config.as_ref())
            .is_some_and(|config| config.harness == HarnessId::ClaudeCode)
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

    fn sync_agent_walks(&mut self, docs: Vec<String>, cx: &mut Context<Self>) {
        self.background.walks.retain(|id, _| docs.contains(id));
        for id in docs {
            if !self.background.walks.contains_key(&id) {
                let watch = self.watch_walk(id.clone(), cx);
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

    fn watch_walk(&self, doc_id: String, cx: &mut Context<Self>) -> Option<gpui::Task<()>> {
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
                        let agent = this.background.walks.get_mut(&doc_id)?;
                        let quiet = std::mem::replace(&mut agent.loaded, true);
                        let entries = &mut agent.entries;
                        let spawns = quiet.then(|| agent_runs(&*entries, 0));
                        let synced =
                            zeron_doc::apply_transcript_frame(entries, update.frame).is_ok();
                        if spawns.is_none_or(|spawns| spawns != agent_runs(&*entries, 0)) {
                            cx.notify();
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

    pub(crate) fn stop_background_task(&mut self, task_id: String, cx: &mut Context<Self>) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Entity, Window};
    use zeron_doc::{MessagePart, MessageRole, SubagentStatus};
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
            label: format!("cargo test {id}"),
            started_at: None,
            output_path: None,
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

    #[gpui::test]
    fn stop_all_reaches_agents_shells_and_walks_nested_runs(cx: &mut gpui::TestAppContext) {
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

        composer.read_with(cx, |composer, cx| {
            let (agents, shells) = composer.stoppable_runs(cx);
            assert_eq!(agents, [("p1".to_string(), false)]);
            assert_eq!(shells, ["shell-1"]);
            assert!(composer.can_stop_background(cx));
        });

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
        composer.read_with(cx, |composer, _| {
            assert!(composer.background.walks.contains_key("c1--sub--n1"));
        });
    }

    #[gpui::test]
    fn a_chat_bound_composer_stops_its_own_chats_runs(cx: &mut gpui::TestAppContext) {
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
        let counts = |cx: &mut gpui::VisualTestContext| {
            composer.read_with(cx, |composer, cx| {
                let (agents, shells) = composer.stoppable_runs(cx);
                (agents.len(), shells.len())
            })
        };
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
        assert_eq!(counts(cx), (1, 1));
    }

    #[test]
    fn token_counts_compact() {
        assert_eq!(compact_count(950), "950");
        assert_eq!(compact_count(29_216), "29.2k");
        assert_eq!(compact_count(1_250_000), "1.25M");
    }
}
