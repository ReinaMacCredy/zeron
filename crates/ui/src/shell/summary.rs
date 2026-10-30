//! Astra's sidecar (paseo-fork `Sidecar.tsx` + `sidecar.css`) beside the transcript.

use super::children::{ChildKind, ChildRef};
use super::*;
use crate::computer::{self, presence::PresenceTone, preview};
use crate::files::client::FilesRequestContext;
use crate::project_actions::{ProjectActionsStatus, action_icon, action_label};
use std::rc::Rc;
use zeron_proto::computer::ComputerPreviewPlacement;
use zeron_proto::device_hub::{DeviceOpener, DevicePlatform};
use zeron_proto::{GitHistoryComparison, GitHistoryPage};

pub(super) const SIDECAR_WIDTH: f32 = 260.0;
pub(super) const SIDECAR_RAIL_WIDTH: f32 = 40.0;
const SIDECAR_AUTO_COLLAPSE_WIDTH: f32 = 640.0;
const SIDECAR_TOP: f32 = Theme::TITLEBAR_HEIGHT + 4.0;
const POPOVER_WIDTH: f32 = 262.0;
const SECTION_CAP: usize = 8;
const RAIL_BUTTON: f32 = 32.0;
const RAIL_GAP: f32 = 4.0;

#[derive(Debug, Clone, Copy)]
pub(super) struct SidecarFlip {
    from: f32,
    started: std::time::Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SidecarPart {
    Workspace,
    Beside,
    Actions,
}

impl SidecarPart {
    fn heading(self) -> &'static str {
        match self {
            Self::Workspace => "Workspace",
            Self::Beside => "Beside this thread",
            Self::Actions => "Actions",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Workspace => icons::ASTRA_LAPTOP,
            Self::Beside => icons::BOT,
            Self::Actions => icons::PLAY,
        }
    }

    fn tag(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Beside => "beside",
            Self::Actions => "actions",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PopoverFade {
    open: bool,
    part: SidecarPart,
    started: std::time::Instant,
}

type SidecarAction = Rc<dyn Fn(&mut Shell, &mut Window, &mut Context<Shell>)>;

enum SidecarValue {
    Text(SharedString),
    Tone(SharedString, gpui::Hsla),
    Dot(SharedString, gpui::Hsla),
    Open,
}

struct SidecarTrail {
    icon: &'static str,
    label: &'static str,
    action: SidecarAction,
}

struct SidecarEntry {
    key: &'static str,
    ix: usize,
    icon: &'static str,
    ink: Option<gpui::Hsla>,
    label: SharedString,
    hint: Option<SharedString>,
    value: Option<SidecarValue>,
    action: Option<SidecarAction>,
    trail: Option<SidecarTrail>,
    thumbnail: Option<String>,
}

impl SidecarEntry {
    fn new(
        key: &'static str,
        ix: usize,
        icon: &'static str,
        label: impl Into<SharedString>,
    ) -> Self {
        Self {
            key,
            ix,
            icon,
            ink: None,
            label: label.into(),
            hint: None,
            value: None,
            action: None,
            trail: None,
            thumbnail: None,
        }
    }

    fn ink(mut self, ink: gpui::Hsla) -> Self {
        self.ink = Some(ink);
        self
    }

    fn hint(mut self, hint: impl Into<SharedString>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    fn value(mut self, value: Option<SidecarValue>) -> Self {
        self.value = value;
        self
    }

    fn action(
        mut self,
        action: impl Fn(&mut Shell, &mut Window, &mut Context<Shell>) + 'static,
    ) -> Self {
        self.action = Some(Rc::new(action));
        self
    }

    fn trail(
        mut self,
        icon: &'static str,
        label: &'static str,
        action: impl Fn(&mut Shell, &mut Window, &mut Context<Shell>) + 'static,
    ) -> Self {
        self.trail = Some(SidecarTrail {
            icon,
            label,
            action: Rc::new(action),
        });
        self
    }

    fn value_text(&self) -> Option<SharedString> {
        match self.value.as_ref()? {
            SidecarValue::Text(v) | SidecarValue::Tone(v, _) | SidecarValue::Dot(v, _) => {
                Some(v.clone())
            }
            SidecarValue::Open => Some("Open".into()),
        }
    }

    fn tooltip(&self) -> SharedString {
        if let Some(hint) = &self.hint {
            return hint.clone();
        }
        match self.value_text() {
            Some(value) => format!("{}: {value}", self.label).into(),
            None => self.label.clone(),
        }
    }
}

struct SidecarInk {
    label: gpui::Hsla,
    heading: gpui::Hsla,
}

impl SidecarInk {
    fn of(theme: &Theme) -> Self {
        Self {
            label: theme.text,
            heading: theme.text_muted,
        }
    }
}

struct SidecarSection {
    part: SidecarPart,
    running: usize,
    entries: Vec<SidecarEntry>,
    empty: Option<SharedString>,
    fold: Option<SidecarFold>,
    foot: Option<SidecarEntry>,
}

impl SidecarSection {
    fn new(part: SidecarPart, entries: Vec<SidecarEntry>) -> Self {
        Self {
            part,
            running: 0,
            entries,
            empty: None,
            fold: None,
            foot: None,
        }
    }

    fn label(&self) -> SharedString {
        match self.running {
            0 => self.part.heading().into(),
            n => format!("{} · {n} running", self.part.heading()).into(),
        }
    }
}

struct SidecarFold {
    label: SharedString,
    failed: usize,
    open: bool,
    entries: Vec<SidecarEntry>,
}

#[derive(Clone, Copy, PartialEq)]
enum SidecarSurface {
    Panel,
    Popover,
}

impl SidecarSurface {
    fn tag(self) -> &'static str {
        match self {
            SidecarSurface::Panel => "panel",
            SidecarSurface::Popover => "popover",
        }
    }
}

impl Shell {
    pub(super) fn sidecar_layout(
        &mut self,
        full_column: f32,
        split_open: bool,
        cx: &mut Context<Self>,
    ) -> f32 {
        // Each split pane is its own conversation; the sidecar would belong to neither.
        if split_open || self.state.read(cx).selected_chat.is_none() {
            return 0.0;
        }
        let collapsed = self.settings.sidecar_collapsed
            || self.right_pane_shown(cx)
            || full_column <= SIDECAR_AUTO_COLLAPSE_WIDTH;
        match self.sidecar_collapsed_now {
            Some(now) if now != collapsed => {
                let from = self.sidecar_width_now();
                self.sidecar_collapsed_now = Some(collapsed);
                self.sidecar_flip = Some(SidecarFlip {
                    from,
                    started: self.render_time.unwrap_or_else(motion::now),
                });
            }
            Some(_) => {}
            None => self.sidecar_collapsed_now = Some(collapsed),
        }
        if !collapsed && self.sidecar_popover_open() {
            self.close_sidecar_popover();
        }
        self.sidecar_width_now().min(full_column)
    }

    fn sidecar_width_now(&self) -> f32 {
        let target = if self.sidecar_collapsed_now == Some(true) {
            SIDECAR_RAIL_WIDTH
        } else {
            SIDECAR_WIDTH
        };
        match self.sidecar_flip {
            Some(flip) => motion::lerp(
                flip.from,
                target,
                self.motion_progress(flip.started, motion::SIDECAR_LAYOUT),
            ),
            None => target,
        }
    }

    fn motion_progress(&self, started: std::time::Instant, spec: motion::MotionSpec) -> f32 {
        if self.reduced_motion {
            return 1.0;
        }
        let total = spec.total().mul_f32(motion::speed_scale());
        let raw = self.tween_elapsed(started).as_secs_f32() / total.as_secs_f32();
        if raw >= 1.0 {
            return 1.0;
        }
        self.motion_active.set(true);
        spec.progress(raw)
    }

    pub(super) fn sidecar_popover_open(&self) -> bool {
        self.sidecar_popover_part().is_some()
    }

    fn sidecar_popover_part(&self) -> Option<SidecarPart> {
        self.sidecar_popover
            .filter(|fade| fade.open)
            .map(|fade| fade.part)
    }

    fn sidecar_popover_mounted(&self) -> Option<SidecarPart> {
        self.sidecar_popover
            .filter(|fade| {
                fade.open
                    || motion::since(fade.started)
                        < motion::SIDECAR_POPOVER_SLIDE
                            .total()
                            .mul_f32(motion::speed_scale())
            })
            .map(|fade| fade.part)
    }

    fn close_sidecar_popover(&mut self) {
        if let Some(fade) = self.sidecar_popover.filter(|fade| fade.open) {
            self.sidecar_popover = Some(PopoverFade {
                open: false,
                part: fade.part,
                started: motion::now(),
            });
        }
    }

    pub(super) fn set_sidecar_popover(
        &mut self,
        part: Option<SidecarPart>,
        cx: &mut Context<Self>,
    ) {
        if self.sidecar_popover_part() == part {
            return;
        }
        match part {
            Some(part) => {
                self.sidecar_popover = Some(PopoverFade {
                    open: true,
                    part,
                    started: motion::now(),
                })
            }
            None => self.close_sidecar_popover(),
        }
        cx.notify();
    }

    fn set_sidecar_user_collapsed(&mut self, collapsed: bool, cx: &mut Context<Self>) {
        if self.settings.sidecar_collapsed != collapsed {
            self.settings.sidecar_collapsed = collapsed;
            self.schedule_save(cx);
        }
        cx.notify();
    }

    fn expand_sidecar(&mut self, cx: &mut Context<Self>) {
        if self.right_pane_shown(cx) {
            self.toggle_right_pane(cx);
        }
        self.set_sidecar_popover(None, cx);
        self.set_sidecar_user_collapsed(false, cx);
    }

    fn toggle_computer_entry(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        let Some(store) = preview::store(cx) else {
            return;
        };
        let view = store.read(cx).view(chat_id);
        match view.placement {
            ComputerPreviewPlacement::Rail if view.floating.is_some() => {
                preview::update_store(&store, cx, |store| store.set_floating(chat_id, None))
            }
            ComputerPreviewPlacement::Rail if self.sidecar_collapsed_now == Some(true) => {
                self.pop_out_computer_preview(chat_id, cx)
            }
            ComputerPreviewPlacement::Rail => {
                preview::update_store(&store, cx, |store| store.toggle_thumbnail(chat_id))
            }
            ComputerPreviewPlacement::Card if view.open => {
                preview::update_store(&store, cx, |store| store.hide_for_task(chat_id))
            }
            ComputerPreviewPlacement::Card => {
                preview::update_store(&store, cx, |store| store.reopen(chat_id))
            }
        }
    }

    fn pop_out_computer_preview(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if let Some(store) = preview::store(cx) {
            preview::update_store(&store, cx, |store| store.reopen(chat_id));
        }
        if let Some(card) = self.transcript.read(cx).computer_preview_card() {
            card.update(cx, |card, cx| card.pop_out(cx));
        }
    }

    fn probe_summary_git(&mut self, chat_id: &str, key: String, cx: &mut Context<Self>) {
        if self.summary_git_probes.contains_key(chat_id)
            || self
                .summary_git
                .get(chat_id)
                .is_some_and(|(probed, _)| *probed == key)
        {
            return;
        }
        let state = self.state.read(cx);
        let Some(context) = FilesRequestContext::for_chat(state, chat_id) else {
            return;
        };
        let Some(engine) = state.engine().cloned() else {
            return;
        };
        let chat_id = chat_id.to_owned();
        let task = cx.spawn({
            let chat_id = chat_id.clone();
            async move |this, cx| {
                let mut params = serde_json::json!({
                    "cwd": context.cwd,
                    "cursor": 0,
                    "limit": 1,
                });
                if let Some(target) = context.target_device_id {
                    params["targetDeviceId"] = serde_json::Value::String(target);
                }
                let comparison = engine
                    .client()
                    .call(zeron_rpc::methods::LIST_GIT_HISTORY, params)
                    .await
                    .ok()
                    .and_then(|value| serde_json::from_value::<GitHistoryPage>(value).ok())
                    .and_then(|page| page.comparison);
                let _ = this.update(cx, |this, cx| {
                    this.summary_git_probes.remove(&chat_id);
                    this.summary_git.insert(chat_id, (key, comparison));
                    cx.notify();
                });
            }
        });
        self.summary_git_probes.insert(chat_id, task);
    }

    fn sidecar_actions_section(&self, theme: &Theme, cx: &App) -> Option<SidecarSection> {
        let status = self.project_actions.active_status()?.clone();
        let snapshot = self.project_actions.visible_snapshot()?;
        let key = self.project_actions.active.clone()?;
        let mut entries = Vec::new();
        match &status {
            ProjectActionsStatus::Idle | ProjectActionsStatus::Loading => {
                entries.push(SidecarEntry::new(
                    "actions-loading",
                    0,
                    icons::LUCIDE_REFRESH_CW,
                    "Loading actions…",
                ));
                return Some(SidecarSection::new(SidecarPart::Actions, entries));
            }
            ProjectActionsStatus::Unavailable { message, .. } => {
                entries.push(SidecarEntry::new(
                    "actions-unavailable",
                    0,
                    icons::INFO_CIRCLE,
                    message.clone(),
                ));
                entries.push(
                    SidecarEntry::new("actions-retry", 0, icons::LUCIDE_REFRESH_CW, "Retry")
                        .action(|this, _, cx| this.retry_project_actions(cx)),
                );
            }
            _ => {}
        }
        let can_run = status.can_run();
        let chat_id = self.state.read(cx).selected_chat.clone();
        let exits = self
            .terminal
            .as_ref()
            .map(|panel| panel.read(cx).exit_codes())
            .unwrap_or_default();
        for (ix, action) in snapshot.actions.into_iter().enumerate() {
            let run = chat_id
                .clone()
                .and_then(|chat| self.project_action_tabs.get(&(chat, action.id.clone())))
                .and_then(|tab| exits.iter().find(|(key, _)| key == tab))
                .map(|(_, exited)| match exited {
                    None => SidecarValue::Tone(
                        "Running".into(),
                        chat_status_tone(zeron_proto::ChatIndicator::Working, theme).2,
                    ),
                    Some(0) => SidecarValue::Tone(
                        "Done".into(),
                        chat_status_tone(zeron_proto::ChatIndicator::Completed, theme).2,
                    ),
                    Some(_) => SidecarValue::Tone("Failed".into(), theme.danger),
                });
            let mut entry = SidecarEntry::new(
                "action",
                ix,
                action_icon(action.icon),
                action_label(&action),
            )
            .value(run);
            if can_run {
                let key = key.clone();
                let run = action.clone();
                entry =
                    entry.action(move |this, _, cx| this.run_project_action(&key, run.clone(), cx));
            }
            entries.push(entry.trail(
                icons::SETTINGS_MINIMALISTIC,
                "Edit action",
                move |this, _, cx| this.open_project_action_editor(Some(action.clone()), None, cx),
            ));
        }
        for (ix, draft) in snapshot.importable_actions.into_iter().enumerate() {
            entries.push(
                SidecarEntry::new(
                    "import-action",
                    ix,
                    action_icon(draft.icon),
                    draft.name.clone(),
                )
                .value(Some(SidecarValue::Text("Import".into())))
                .action(move |this, _, cx| {
                    this.open_project_action_editor(None, Some(draft.clone()), cx)
                }),
            );
        }
        if let Some(issue) = snapshot.project_file_issue {
            entries.push(SidecarEntry::new(
                "actions-issue",
                0,
                icons::INFO_CIRCLE,
                issue,
            ));
        }
        entries.push(
            SidecarEntry::new("add-action", 0, icons::ASTRA_PLUS, "Add action")
                .action(|this, _, cx| this.open_project_action_editor(None, None, cx)),
        );
        Some(SidecarSection::new(SidecarPart::Actions, entries))
    }

    fn sidecar_sections(&mut self, theme: &Theme, cx: &mut Context<Self>) -> Vec<SidecarSection> {
        self.ensure_project_actions(cx);
        let chat = self.state.read(cx).selected_chat_row().cloned();
        let mut sections = vec![
            self.sidecar_workspace(theme, chat.as_ref(), cx),
            self.sidecar_beside(theme, chat.as_ref(), cx),
        ];
        sections.extend(self.sidecar_actions_section(theme, cx));
        sections
    }

    fn sidecar_workspace(
        &mut self,
        theme: &Theme,
        chat: Option<&zeron_proto::Chat>,
        cx: &mut Context<Self>,
    ) -> SidecarSection {
        let branch = chat.and_then(|chat| {
            chat.source_context
                .as_ref()
                .map(|ctx| ctx.branch.clone())
                .or_else(|| chat.branch.clone())
                .filter(|branch| !branch.is_empty())
        });
        let head = chat.and_then(|chat| {
            chat.source_context
                .as_ref()
                .and_then(|ctx| ctx.head_sha.as_deref())
                .map(|sha| sha.chars().take(7).collect::<String>())
        });
        let (device, path, git) = {
            let state = self.state.read(cx);
            let device =
                chat.and_then(|chat| state.device_name(&chat.device_id).map(str::to_owned));
            let local = chat.is_some_and(|chat| {
                state.local_device_id.as_deref() == Some(chat.device_id.as_str())
            });
            let home = local.then(|| std::env::var("HOME").ok()).flatten();
            let path = chat
                .and_then(|chat| chat.cwd.as_deref())
                .map(|cwd| zeron_proto::claude_panel::tilde(cwd, home.as_deref()));
            let git = chat
                .and_then(|chat| state.space_for_chat(chat))
                .is_some_and(|space| space.git_detected);
            (device, path, git)
        };
        let comparison = match chat.filter(|_| git) {
            Some(chat) => {
                let key = format!(
                    "{}@{}",
                    branch.as_deref().unwrap_or_default(),
                    head.as_deref().unwrap_or_default()
                );
                self.probe_summary_git(&chat.id, key, cx);
                self.summary_git
                    .get(&chat.id)
                    .and_then(|(_, comparison)| comparison.clone())
            }
            None => None,
        };

        let mut entries = vec![SidecarEntry::new(
            "machine",
            0,
            icons::ASTRA_LAPTOP,
            device.unwrap_or_else(|| "Local".into()),
        )];
        let path = path.unwrap_or_else(|| "No workspace".into());
        entries.push(SidecarEntry::new("path", 0, icons::ASTRA_FOLDER, path.clone()).hint(path));
        if git {
            let commit = commit_line(head.as_deref(), branch.as_deref(), comparison.as_ref());
            entries.push(
                SidecarEntry::new(
                    "branch",
                    0,
                    icons::ASTRA_GIT_BRANCH,
                    branch.unwrap_or_else(|| "No branch".into()),
                )
                .ink(theme.branch_ink()),
            );
            if let Some(commit) = commit {
                entries.push(
                    SidecarEntry::new("commit", 0, icons::LUCIDE_GIT_COMMIT_HORIZONTAL, commit)
                        .ink(theme.text_muted),
                );
            }
        }

        let computer = chat.and_then(|chat| computer::presence::rail_entry(&chat.id, cx));
        if let Some(thumbnail) = &self.computer_thumbnail {
            let chat_id = computer.as_ref().map(|entry| entry.chat_id.clone());
            thumbnail.update(cx, |thumbnail, cx| thumbnail.set_chat(chat_id, cx));
        }
        if let Some(entry) = computer {
            let dot = match computer::presence::tone(&entry.chat_id, cx) {
                Some(PresenceTone::Driving | PresenceTone::NeedsYou) => theme.accent,
                Some(PresenceTone::Paused) => theme.warning,
                _ => theme.text_faint,
            };
            let chat_id = entry.chat_id.clone();
            let mut row = SidecarEntry::new("computer", 0, icons::COMPUTER_CURSOR, "Computer")
                .value(entry.app.map(|app| SidecarValue::Dot(app.into(), dot)))
                .action(move |this, _, cx| this.toggle_computer_entry(&chat_id, cx));
            row.thumbnail = entry.thumbnail.then_some(entry.chat_id);
            entries.push(row);
        }
        if let Some(chat) = chat {
            entries.extend(
                self.sidecar_devices(&chat.id, cx)
                    .into_iter()
                    .take(SECTION_CAP)
                    .enumerate()
                    .map(|(ix, (target, name, platform, opener))| {
                        let glyph = match platform {
                            DevicePlatform::Ios => icons::APPLE,
                            DevicePlatform::Android => icons::ANDROID,
                        };
                        let opener = opener.map(|opener| match opener {
                            DeviceOpener::User => SidecarValue::Text("You".into()),
                            DeviceOpener::Agent => SidecarValue::Text("Agent".into()),
                        });
                        let chat_id = chat.id.clone();
                        SidecarEntry::new("device", ix, glyph, name)
                            .value(opener)
                            .action(move |this, window, cx| {
                                this.focus_device(&chat_id, target.clone(), window, cx)
                            })
                    }),
            );
        }
        SidecarSection::new(SidecarPart::Workspace, entries)
    }

    fn sidecar_beside(
        &mut self,
        theme: &Theme,
        chat: Option<&zeron_proto::Chat>,
        cx: &mut Context<Self>,
    ) -> SidecarSection {
        let mut section = SidecarSection::new(SidecarPart::Beside, Vec::new());
        let Some(chat) = chat else {
            return section;
        };
        let now = Utc::now();
        let now_ms = now.timestamp_millis();
        let working = chat_status_tone(zeron_proto::ChatIndicator::Working, theme).2;
        let open = self.open_children(cx);
        let agents = self.transcript.read(cx).subagent_summaries(cx);
        let pickers = self.composer.read(cx).pickers().read(cx);
        let tags: Vec<Option<String>> = agents
            .iter()
            .map(|agent| {
                agent_tag(agent, |harness, id| {
                    pickers
                        .model_label(harness, id)
                        .or_else(|| crate::pickers::curated_model_label(harness, id))
                })
            })
            .collect();
        let agents: Vec<_> = agents.into_iter().zip(tags).collect();
        let agent_ids: std::collections::HashSet<String> = agents
            .iter()
            .map(|(agent, _)| agent.doc_id.clone())
            .collect();
        let (running, previous) = split_subagents(agents);
        let next_tick = running
            .iter()
            .filter_map(|(agent, _)| agent.started_at)
            .map(|started| elapsed_next_change((now_ms - started) / 1000))
            .min();
        if let Some(secs) = next_tick
            && self.sidecar_clock.is_none()
        {
            self.sidecar_clock = Some(cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_secs(secs as u64))
                    .await;
                this.update(cx, |this, cx| {
                    this.sidecar_clock = None;
                    cx.notify();
                })
                .ok();
            }));
        }
        let agent_entry = |key: &'static str, ix: usize, (agent, tag): LabeledSubagent| {
            let value = if open.contains(&agent.doc_id) {
                Some(SidecarValue::Open)
            } else {
                match agent.status {
                    Some(zeron_doc::SubagentStatus::Running) => Some(SidecarValue::Dot(
                        agent.started_at.map_or("Working".into(), |started| {
                            short_elapsed((now_ms - started) / 1000).into()
                        }),
                        working,
                    )),
                    Some(zeron_doc::SubagentStatus::Failed) => {
                        Some(SidecarValue::Dot("Failed".into(), theme.danger))
                    }
                    Some(zeron_doc::SubagentStatus::Stopped) => {
                        Some(SidecarValue::Dot("Stopped".into(), theme.text_faint))
                    }
                    Some(zeron_doc::SubagentStatus::Done) => {
                        Some(SidecarValue::Dot("Done".into(), theme.success))
                    }
                    None => None,
                }
            };
            let hint = [
                Some(agent.title.to_string()),
                Some(ChildKind::Agent.label().into()),
                tag,
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
            SidecarEntry::new(key, ix, ChildKind::Agent.icon(), agent.title.clone())
                .hint(hint)
                .value(value)
                .action(move |this, _, cx| this.open_child(ChildRef::of_summary(&agent), cx))
        };
        section.running = running.len();
        section.entries = running
            .into_iter()
            .take(SECTION_CAP)
            .enumerate()
            .map(|(ix, agent)| agent_entry("subagent", ix, agent))
            .collect();
        let failed = previous
            .iter()
            .filter(|(agent, _)| agent.status == Some(zeron_doc::SubagentStatus::Failed))
            .count();
        section.fold = (!previous.is_empty()).then(|| SidecarFold {
            label: format!("Previous agents ({})", previous.len()).into(),
            failed,
            open: self.sidecar_previous_agents_open,
            entries: previous
                .into_iter()
                .enumerate()
                .map(|(ix, agent)| agent_entry("previous-agent", ix, agent))
                .collect(),
        });

        let state = self.state.read(cx);
        let mut children: Vec<&zeron_proto::Chat> = state
            .chats
            .iter()
            .filter(|child| {
                !child.archived
                    && !agent_ids.contains(&child.id)
                    && (child.opened_from() == Some(chat.id.as_str())
                        || child.delegated_by() == Some(chat.id.as_str()))
            })
            .collect();
        children.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        for (ix, child) in children.into_iter().take(SECTION_CAP).enumerate() {
            let kind = ChildKind::of(child);
            let status = state.display_status_for(child, now);
            let value = if open.contains(&child.id) {
                SidecarValue::Open
            } else if child.sidechat_expired(now) {
                SidecarValue::Dot("Expired".into(), theme.text_faint)
            } else {
                match chat_status_tone(status, theme) {
                    (Some(label), _, color) if status != zeron_proto::ChatIndicator::Completed => {
                        SidecarValue::Dot(label, color)
                    }
                    _ => {
                        let at = child.last_message_at.unwrap_or(child.created_at);
                        let ago = short_elapsed(now.signed_duration_since(at).num_seconds());
                        SidecarValue::Dot(format!("{ago} ago").into(), theme.text_faint)
                    }
                }
            };
            if status == zeron_proto::ChatIndicator::Working {
                section.running += 1;
            }
            let title = child
                .title
                .clone()
                .filter(|title| !title.trim().is_empty())
                .unwrap_or_else(|| kind.label().to_owned());
            let id = child.id.clone();
            section.entries.push(
                SidecarEntry::new("child", ix, kind.icon(), title.clone())
                    .hint(format!("{title} · {}", kind.label()))
                    .value(Some(value))
                    .action(move |this, _, cx| this.open_child(ChildRef::Chat(id.clone()), cx)),
            );
        }

        let shells = zeron_proto::background::shell_tasks(state.session_for(&chat.id));
        let stoppable = self.composer.read(cx).can_stop_shells(cx);
        section.running += shells.len();
        for (ix, task) in shells.into_iter().take(SECTION_CAP).enumerate() {
            let elapsed = task.started_at.map_or("Running".into(), |at| {
                short_elapsed(now.signed_duration_since(at).num_seconds())
            });
            let mut entry = SidecarEntry::new("shell", ix, icons::ASTRA_TERMINAL, task.label)
                .value(Some(SidecarValue::Dot(elapsed.into(), working)));
            if stoppable {
                let id = task.id;
                entry = entry.trail(icons::STOP, "Stop", move |this, _, cx| {
                    let id = id.clone();
                    this.composer
                        .update(cx, |composer, cx| composer.stop_background_task(id, cx));
                });
            }
            section.entries.push(entry);
        }

        if let Some(mission) = crate::tasks::mission_thread(state, &chat.id, now) {
            let zeron_proto::PlanProgress { done, total } = mission.progress;
            let step = (done + 1).min(total);
            let task_id = mission.task_id.clone();
            let selection = crate::tasks::mission_selection(&mission.role);
            section.entries.push(
                SidecarEntry::new(
                    "task",
                    0,
                    icons::CHECKLIST,
                    format!("Task: {}", mission.title),
                )
                .value(Some(SidecarValue::Text(
                    format!("step {step} of {total}").into(),
                )))
                .action(move |this, _, cx| {
                    this.open_mission(task_id.clone(), selection.clone(), cx)
                }),
            );
        }

        if self.composer.read(cx).can_stop_background(cx) {
            section.foot = Some(
                SidecarEntry::new("stop-all", 0, icons::STOP, "Stop all").action(|this, _, cx| {
                    this.composer
                        .update(cx, |composer, cx| composer.stop_all_background(cx))
                }),
            );
        }
        if section.entries.is_empty() && section.fold.is_none() {
            section.empty = Some("Nothing beside this thread".into());
        }
        section
    }

    pub(super) fn render_sidecar(&mut self, width: f32, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let sections = self.sidecar_sections(&theme, cx);
        let collapsed = self.sidecar_collapsed_now == Some(true);
        let (slide, fade) = match self.sidecar_flip {
            Some(flip) => (
                self.motion_progress(flip.started, motion::SIDECAR_LAYOUT),
                self.motion_progress(flip.started, motion::SIDECAR_FADE),
            ),
            None => (1.0, 1.0),
        };
        let (panel_opacity, panel_x, rail_opacity, rail_x) = if collapsed {
            (1.0 - fade, 18.0 * slide, fade, 12.0 * (1.0 - slide))
        } else {
            (fade, 18.0 * (1.0 - slide), 1.0 - fade, 12.0 * slide)
        };

        let mut root = div()
            .id("sidecar")
            .debug_selector(|| "sidecar".into())
            .role(gpui::Role::Group)
            .aria_label("Live state")
            .relative()
            .flex_none()
            .h_full()
            .w(px(width));
        if !collapsed {
            root = root.overflow_hidden();
        }
        if panel_opacity > 0.0 {
            root = root.child(self.sidecar_panel(
                &theme,
                &sections,
                panel_opacity,
                panel_x,
                !collapsed,
                cx,
            ));
        }
        if rail_opacity > 0.0 {
            root = root.child(self.sidecar_rail(
                &theme,
                &sections,
                rail_opacity,
                rail_x,
                collapsed,
                cx,
            ));
        }
        if let Some(popover) = self.sidecar_popover_card(&theme, &sections, cx) {
            root = root.child(popover);
        }
        div()
            .flex_none()
            .h_full()
            .w(px(width))
            .child(crate::terminal::dock::above_terminal(
                root,
                self.terminal_geometry.clone(),
            ))
            .into_any_element()
    }

    fn sidecar_panel(
        &self,
        theme: &Theme,
        sections: &[SidecarSection],
        opacity: f32,
        offset: f32,
        interactive: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let collapse = div()
            .id("sidecar-collapse")
            .debug_selector(|| "sidecar-collapse".into())
            .role(gpui::Role::Button)
            .aria_label("Fold the Sidecar")
            .size(px(20.0))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .group("sidecar-collapse")
            .when(interactive, |el| {
                el.cursor_pointer()
                    .hover(|s| s.bg(crate::theme::wash(0.06)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.set_sidecar_user_collapsed(true, cx);
                    }))
            })
            .child(
                icon(icons::ASTRA_CHEVRONS_RIGHT)
                    .size(px(12.0))
                    .text_color(theme.text_faint)
                    .group_hover("sidecar-collapse", |s| s.text_color(theme.text)),
            );
        let header = div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .justify_between()
            .h(px(24.0))
            .pl(px(12.0))
            .pr(px(10.0))
            .text_size(crate::typography::ui_rems(13.0))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(theme.text)
            .child("Live state")
            .child(collapse);
        div()
            .absolute()
            .top(px(SIDECAR_TOP))
            .bottom_0()
            .left(px(offset))
            .w(px(SIDECAR_WIDTH))
            .pr(px(8.0))
            .opacity(opacity)
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .id("sidecar-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .pt(px(2.0))
                    .px(px(4.0))
                    .pb(px(4.0))
                    .children(sections.iter().map(|section| {
                        self.sidecar_group(theme, section, SidecarSurface::Panel, interactive, cx)
                    })),
            )
            .into_any_element()
    }

    fn sidecar_group(
        &self,
        theme: &Theme,
        section: &SidecarSection,
        surface: SidecarSurface,
        interactive: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let heading = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .h(px(19.0))
            .px(px(8.0))
            .text_size(crate::typography::ui_rems(12.0))
            .line_height(px(19.0))
            .text_color(SidecarInk::of(theme).heading)
            .child(div().min_w_0().truncate().child(section.part.heading()))
            .when(section.running > 0, |el| {
                el.child(
                    div()
                        .flex_none()
                        .text_size(crate::typography::ui_rems(11.0))
                        .child(SharedString::from(format!("{} running", section.running))),
                )
            });
        div()
            .id(SharedString::from(format!(
                "sidecar-{}-section-{}",
                surface.tag(),
                section.part.tag()
            )))
            .role(gpui::Role::Group)
            .aria_label(section.label())
            .flex()
            .flex_col()
            .flex_none()
            .pt(px(8.0))
            .pb(px(4.0))
            .child(heading)
            .children(section.entries.iter().flat_map(|entry| {
                let thumbnail = entry
                    .thumbnail
                    .clone()
                    .filter(|_| surface == SidecarSurface::Panel)
                    .and_then(|chat| Some((chat, self.computer_thumbnail.clone()?)))
                    .map(|(chat, thumbnail)| {
                        div()
                            .id("sidecar-computer-thumbnail")
                            .debug_selector(|| "sidecar-computer-thumbnail".into())
                            .role(gpui::Role::Button)
                            .aria_label("Pop out computer preview")
                            .flex_none()
                            .mx(px(4.0))
                            .mt(px(4.0))
                            .mb(px(8.0))
                            .when(interactive, |el| {
                                el.cursor_pointer()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.pop_out_computer_preview(&chat, cx);
                                    }))
                            })
                            .child(thumbnail)
                            .into_any_element()
                    });
                std::iter::once(self.sidecar_row(theme, entry, surface, interactive, cx))
                    .chain(thumbnail)
            }))
            .children(section.empty.clone().map(|empty| {
                div()
                    .h(px(28.0))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_faint)
                    .child(empty)
            }))
            .when_some(section.fold.as_ref(), |el, fold| {
                el.child(self.sidecar_fold_footer(theme, fold, surface, interactive, cx))
                    .when(fold.open, |el| {
                        el.children(
                            fold.entries.iter().map(|entry| {
                                self.sidecar_row(theme, entry, surface, interactive, cx)
                            }),
                        )
                    })
            })
            .children(
                section
                    .foot
                    .as_ref()
                    .map(|foot| self.sidecar_row(theme, foot, surface, interactive, cx)),
            )
            .into_any_element()
    }

    fn sidecar_fold_footer(
        &self,
        theme: &Theme,
        fold: &SidecarFold,
        surface: SidecarSurface,
        interactive: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = format!("sidecar-{}-fold", surface.tag());
        let selector = id.clone();
        let open = fold.open;
        div()
            .id(SharedString::from(id))
            .debug_selector(move || selector.clone())
            .role(gpui::Role::Button)
            .aria_label(fold.label.clone())
            .aria_expanded(open)
            .when(fold.failed > 0, |el| {
                el.aria_description(format!("{} failed", fold.failed))
            })
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(8.0))
            .h(px(28.0))
            .px(px(8.0))
            .rounded(px(8.0))
            .text_size(crate::typography::ui_rems(12.0))
            .text_color(theme.text_faint)
            .whitespace_nowrap()
            .child(div().flex_none().child(fold.label.clone()))
            .child(div().flex_1().h(px(1.0)).bg(crate::theme::hairline(0.08)))
            .when(fold.failed > 0, |el| {
                el.child(
                    div()
                        .flex_none()
                        .text_color(theme.danger)
                        .child(SharedString::from(format!("{} failed", fold.failed))),
                )
            })
            .child(
                icon(icons::LUCIDE_CHEVRON_DOWN)
                    .size(px(12.0))
                    .text_color(theme.text_faint)
                    .when(open, |el| {
                        el.with_transformation(gpui::Transformation::rotate(gpui::percentage(0.5)))
                    }),
            )
            .when(interactive, |el| {
                el.cursor_pointer()
                    .hover(|s| s.bg(crate::theme::wash(0.06)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.sidecar_previous_agents_open = !open;
                        cx.notify();
                    }))
            })
            .into_any_element()
    }

    fn sidecar_row(
        &self,
        theme: &Theme,
        entry: &SidecarEntry,
        surface: SidecarSurface,
        interactive: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = format!("sidecar-{}-{}-{}", surface.tag(), entry.key, entry.ix);
        let group = SharedString::from(id.clone());
        let selector = id.clone();
        let value = entry
            .value
            .as_ref()
            .map(|value| sidecar_value(theme, value));
        let actionable = interactive && entry.action.is_some();
        let tooltip = entry.tooltip();
        let mut row = div()
            .id(SharedString::from(id))
            .debug_selector(move || selector.clone())
            .role(if actionable {
                gpui::Role::Button
            } else {
                gpui::Role::Group
            })
            .aria_label(entry.label.clone())
            .when_some(entry.value_text(), |el, value| el.aria_value(value))
            .group(group.clone())
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(8.0))
            .h(px(28.0))
            .px(px(8.0))
            .rounded(px(8.0))
            .text_size(crate::typography::ui_rems(13.0))
            .text_color(SidecarInk::of(theme).label)
            .whitespace_nowrap()
            .child(
                icon(entry.icon)
                    .flex_none()
                    .size(px(14.0))
                    .text_color(theme.text_muted)
                    .when(actionable, |el| {
                        el.group_hover(group.clone(), |s| s.text_color(theme.text))
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .when_some(entry.ink, |el, ink| el.text_color(ink))
                    .child(entry.label.clone()),
            )
            .children(value)
            .when_some(entry.trail.as_ref().filter(|_| interactive), |el, trail| {
                let id = format!("{group}-trail");
                let selector = id.clone();
                let action = trail.action.clone();
                el.child(
                    div()
                        .id(SharedString::from(id))
                        .debug_selector(move || selector.clone())
                        .role(gpui::Role::Button)
                        .aria_label(trail.label)
                        .size(px(22.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(5.0))
                        .invisible()
                        .group_hover(group.clone(), |style| style.visible())
                        .hover(|style| style.bg(crate::theme::wash(0.08)))
                        .tooltip(crate::settings::widgets::text_tooltip(trail.label))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            action(this, window, cx);
                            if surface == SidecarSurface::Popover {
                                this.set_sidecar_popover(None, cx);
                            }
                            cx.notify();
                        }))
                        .child(icon(trail.icon).size(px(14.0)).text_color(theme.text_muted)),
                )
            })
            .when(interactive, |el| {
                el.tooltip(crate::settings::widgets::text_tooltip(tooltip))
                    .tooltip_show_delay(Duration::from_millis(500))
            })
            .when(actionable, |el| {
                el.hover(|s| s.bg(crate::theme::wash(0.06)))
            });
        match entry.action.clone().filter(|_| interactive) {
            Some(action) => {
                row = row
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        action(this, window, cx);
                        if surface == SidecarSurface::Popover {
                            this.set_sidecar_popover(None, cx);
                        }
                        cx.notify();
                    }));
            }
            None => row = row.cursor_default(),
        }
        row.into_any_element()
    }

    fn sidecar_rail(
        &self,
        theme: &Theme,
        sections: &[SidecarSection],
        opacity: f32,
        offset: f32,
        interactive: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let up = self.sidecar_popover_part();
        let rail_button = |key: String, glyph: &'static str, tooltip: SharedString, on: bool| {
            let selector = key.clone();
            div()
                .id(SharedString::from(key))
                .debug_selector(move || selector.clone())
                .role(gpui::Role::Button)
                .aria_label(tooltip.clone())
                .relative()
                .size(px(RAIL_BUTTON))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded(px(8.0))
                .when(on, |el| el.bg(crate::theme::wash(0.06)))
                .when(interactive, |el| {
                    el.cursor_pointer()
                        .hover(|s| s.bg(crate::theme::wash(0.06)))
                        .tooltip(move |_, cx| {
                            cx.new(|_| SurfaceTabTooltip {
                                text: tooltip.clone(),
                            })
                            .into()
                        })
                        .tooltip_show_delay(Duration::from_millis(260))
                })
                .child(icon(glyph).size(px(17.0)).text_color(if on {
                    theme.text
                } else {
                    theme.text_muted
                }))
        };
        let mut expand = rail_button(
            "sidecar-expand".into(),
            icons::ASTRA_CHEVRONS_LEFT,
            "Expand the Sidecar".into(),
            false,
        );
        if interactive {
            expand = expand.on_click(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                this.expand_sidecar(cx);
            }));
        }
        let mut rail = div()
            .absolute()
            .top(px(SIDECAR_TOP))
            .right(px((SIDECAR_RAIL_WIDTH - RAIL_BUTTON) / 2.0 - offset))
            .w(px(RAIL_BUTTON))
            .flex()
            .flex_col()
            .gap(px(RAIL_GAP))
            .opacity(opacity)
            .child(expand);
        for section in sections {
            let part = section.part;
            let mut button = rail_button(
                format!("sidecar-rail-{}", part.tag()),
                part.icon(),
                section.label(),
                up == Some(part),
            );
            if part == SidecarPart::Beside && section.running > 0 {
                button = button.child(
                    div()
                        .debug_selector(|| "sidecar-rail-badge".into())
                        .absolute()
                        .top(px(1.0))
                        .right_0()
                        .min_w(px(14.0))
                        .h(px(14.0))
                        .px(px(3.0))
                        .rounded(px(7.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(theme.busy)
                        .text_color(gpui::white())
                        .text_size(crate::typography::ui_rems(9.5))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(SharedString::from(section.running.to_string())),
                );
            }
            if interactive {
                // The popover's mouse-down-out closes it on this same press, so the click checks what the press found.
                button = button
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, _| {
                            this.sidecar_press_found_popover = this.sidecar_popover_mounted();
                        }),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        let found = this.sidecar_press_found_popover.take();
                        if found != Some(part) {
                            this.set_sidecar_popover(Some(part), cx);
                        }
                    }));
            }
            rail = rail.child(button);
        }
        rail.into_any_element()
    }

    fn sidecar_popover_card(
        &self,
        theme: &Theme,
        sections: &[SidecarSection],
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let fade = self.sidecar_popover?;
        let slot = sections
            .iter()
            .position(|section| section.part == fade.part)?;
        let section = &sections[slot];
        let opacity = self.motion_progress(fade.started, motion::SIDECAR_POPOVER_FADE);
        let slide = self.motion_progress(fade.started, motion::SIDECAR_POPOVER_SLIDE);
        let (opacity, offset) = if fade.open {
            (opacity, 10.0 * (1.0 - slide))
        } else if slide >= 1.0 {
            return None;
        } else {
            (1.0 - opacity, 10.0 * slide)
        };
        let top = SIDECAR_TOP + (slot + 1) as f32 * (RAIL_BUTTON + RAIL_GAP) - 6.0;
        let max_h = (self.viewport_height - top - 16.0).max(120.0);
        let card = popover::popover_card_flush(theme)
            .w(px(POPOVER_WIDTH))
            .max_h(px(max_h))
            .flex()
            .flex_col()
            .rounded(px(12.0))
            .child(
                div()
                    .id("sidecar-popover-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .px(px(4.0))
                    .pb(px(4.0))
                    .child(self.sidecar_group(
                        theme,
                        section,
                        SidecarSurface::Popover,
                        fade.open,
                        cx,
                    )),
            );
        Some(
            div()
                .id("sidecar-popover")
                .debug_selector(|| "sidecar-popover".into())
                .absolute()
                .top(px(top))
                .right(px(SIDECAR_RAIL_WIDTH + 4.0 - offset))
                .w(px(POPOVER_WIDTH))
                .opacity(opacity)
                .occlude()
                .when(fade.open, |el| {
                    el.on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.set_sidecar_popover(None, cx);
                    }))
                })
                .child(crate::frost::frosted(
                    12.0,
                    crate::frost::MENU_BLUR * opacity,
                    card,
                ))
                .into_any_element(),
        )
    }
}

fn sidecar_value(theme: &Theme, value: &SidecarValue) -> AnyElement {
    let base = div()
        .flex_none()
        .max_w(px(120.0))
        .truncate()
        .text_size(crate::typography::ui_rems(12.0))
        .text_color(theme.text_faint);
    match value {
        SidecarValue::Text(text) => base.child(text.clone()).into_any_element(),
        SidecarValue::Tone(text, color) => base
            .text_color(*color)
            .child(text.clone())
            .into_any_element(),
        SidecarValue::Dot(text, color) => div()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(5.0))
            .max_w(px(120.0))
            .text_size(crate::typography::ui_rems(12.0))
            .text_color(theme.text_faint)
            .child(div().flex_none().size(px(6.0)).rounded_full().bg(*color))
            .child(div().truncate().child(text.clone()))
            .into_any_element(),
        SidecarValue::Open => base
            .text_size(crate::typography::ui_rems(11.0))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(theme.accent)
            .child("Open")
            .into_any_element(),
    }
}

/// On the integration branch itself "N ahead of main" would be wrong, so it gets the sha alone.
fn commit_line(
    sha: Option<&str>,
    branch: Option<&str>,
    comparison: Option<&GitHistoryComparison>,
) -> Option<String> {
    let status = comparison.and_then(|comparison| {
        let base = comparison
            .base
            .split_once('/')
            .map_or(comparison.base.as_str(), |(_, branch)| branch);
        let (ahead, behind) = (comparison.ahead, comparison.behind);
        if ahead == 0 && behind == 0 {
            return Some("up to date".to_string());
        }
        if branch == Some(base) {
            return None;
        }
        Some(match (ahead, behind) {
            (0, behind) => format!("{behind} behind {base}"),
            (ahead, 0) => format!("{ahead} ahead of {base}"),
            (ahead, behind) => format!("{ahead} ahead, {behind} behind {base}"),
        })
    });
    let parts: Vec<String> = sha.map(str::to_owned).into_iter().chain(status).collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

type LabeledSubagent = (transcript::SubagentSummary, Option<String>);

fn split_subagents(
    mut agents: Vec<LabeledSubagent>,
) -> (Vec<LabeledSubagent>, Vec<LabeledSubagent>) {
    agents.reverse();
    agents
        .into_iter()
        .partition(|(agent, _)| agent.status == Some(zeron_doc::SubagentStatus::Running))
}

fn agent_tag(
    agent: &transcript::SubagentSummary,
    model_label: impl Fn(zeron_proto::HarnessId, &str) -> Option<String>,
) -> Option<String> {
    match agent.model.as_deref() {
        Some(id) => Some(
            agent
                .harness
                .and_then(|harness| model_label(harness, id))
                .unwrap_or_else(|| id.to_owned()),
        ),
        None => agent.kind.as_ref().map(ToString::to_string),
    }
}

fn short_elapsed(secs: i64) -> String {
    match secs.max(0) {
        s if s < 60 => format!("{s}s"),
        s if s < 3_600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3_600),
        s => format!("{}d", s / 86_400),
    }
}

fn elapsed_next_change(secs: i64) -> i64 {
    let secs = secs.max(0);
    let unit = match secs {
        s if s < 60 => 1,
        s if s < 3_600 => 60,
        s if s < 86_400 => 3_600,
        _ => 86_400,
    };
    unit - secs % unit
}

#[cfg(test)]
mod sidecar_regressions {
    use super::*;
    use gpui::{AppContext, TestAppContext, VisualTestContext};

    struct SidecarHost {
        shell: Entity<Shell>,
        _data_dir: tempfile::TempDir,
    }

    impl Render for SidecarHost {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let sidecar = self.shell.update(cx, |shell, cx| {
                let width = shell.sidecar_layout(1000.0, false, cx);
                shell.render_sidecar(width, cx)
            });
            div()
                .w(px(1000.))
                .h(px(800.))
                .flex()
                .flex_row()
                .justify_end()
                .child(sidecar)
        }
    }

    fn setup(cx: &mut TestAppContext) -> (Entity<Shell>, &mut VisualTestContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            crate::settings::init(crate::settings::UiSettings::default(), dir.path(), cx);
            crate::draft_store::init(dir.path(), cx);
            crate::history::init(
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                cx,
            );
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            crate::app_menus::init(cx);
            crate::computer::preview::init(cx);
        });
        let (host, cx) = cx.add_window_view(|window, cx| {
            let shell = cx.new(|cx| {
                let state = cx.new(|_| AppState::new());
                let mut shell = Shell::new(
                    state,
                    EngineBootConfig {
                        data_dir: dir.path().into(),
                        ipc_port: 0,
                        edge_url: "http://127.0.0.1:1".into(),
                        edge_token: None,
                        org_id: None,
                        workos_client_id: None,
                        default_harness: zeron_proto::HarnessId::Mock,
                    },
                    cx,
                );
                shell.reduced_motion = true;
                shell.active_chat = "parent".into();
                shell.state.update(cx, |state, _| {
                    let source: zeron_proto::Chat = serde_json::from_value(serde_json::json!({
                        "id": "parent", "deviceId": "dev", "title": "Parent",
                        "archived": false, "createdAt": "2026-09-18T00:00:00Z"
                    }))
                    .unwrap();
                    state.chats.push(source.clone());
                    for id in ["side-a", "side-b"] {
                        let mut chat = source.clone();
                        chat.id = id.into();
                        chat.sidechat_source_chat_id = Some("parent".into());
                        state.chats.push(chat);
                    }
                    state.selected_chat = Some("parent".into());
                });
                shell.add_browser_surface(None, window, cx);
                if shell.right_pane_open(cx) {
                    shell.toggle_right_pane(cx);
                }
                shell
            });
            SidecarHost {
                shell,
                _data_dir: dir,
            }
        });
        let shell = host.read_with(cx, |host, _| host.shell.clone());
        cx.update(|window, cx| window.draw(cx).clear());
        (shell, cx)
    }

    fn click(cx: &mut VisualTestContext, selector: &'static str) {
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is on screen"));
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear());
    }

    #[gpui::test]
    fn panel_rows_open_each_sidechat_and_reuse_only_the_matching_tab(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        for (row, expected, count) in [
            ("sidecar-panel-child-0", "side-a", 1),
            ("sidecar-panel-child-1", "side-b", 2),
            ("sidecar-panel-child-0", "side-a", 2),
        ] {
            shell.update(cx, |shell, cx| {
                if shell.right_pane_open(cx) {
                    shell.toggle_right_pane(cx);
                }
            });
            cx.update(|window, cx| window.draw(cx).clear());
            click(cx, row);
            shell.read_with(cx, |shell, cx| {
                assert!(shell.right_pane_open(cx));
                assert_eq!(shell.sidechat_tabs.len(), count);
                let RightSurface::Sidechat(id) = shell.resolved_right_active(cx) else {
                    panic!("sidechat must be active");
                };
                assert_eq!(shell.sidechat_tabs[&id].chat_id, expected);
            });
        }
    }

    #[gpui::test]
    fn side_chat_rows_carry_their_kind_and_age(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                let chat = state.chats.iter_mut().find(|c| c.id == "side-a").unwrap();
                chat.created_at = Utc::now();
                chat.last_message_at = Some(Utc::now());
            });
            let theme = Theme::of(cx).clone();
            let sections = shell.sidecar_sections(&theme, cx);
            let parts: Vec<_> = sections.iter().map(|section| section.part).collect();
            assert_eq!(parts, [SidecarPart::Workspace, SidecarPart::Beside]);
            let beside = &sections[1];
            let hints: Vec<_> = beside.entries.iter().map(|e| e.tooltip()).collect();
            assert_eq!(hints, ["Parent · Side chat", "Parent · Side chat"]);
            assert_eq!(beside.entries[0].value_text().as_deref(), Some("Expired"));
            assert!(
                beside.entries[1]
                    .value_text()
                    .is_some_and(|value| value.ends_with(" ago"))
            );
            assert!(
                beside
                    .entries
                    .iter()
                    .all(|e| e.icon == ChildKind::SideChat.icon())
            );
            assert_eq!(beside.running, 0);
            assert!(beside.foot.is_none(), "nothing runs, so no Stop all");
        });
    }

    #[gpui::test]
    fn collapse_persists_and_the_rail_chevrons_bring_the_panel_back(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        assert!(cx.debug_bounds("sidecar-rail-beside").is_none());
        click(cx, "sidecar-collapse");
        shell.read_with(cx, |shell, cx| {
            assert!(shell.settings.sidecar_collapsed);
            assert!(crate::settings::current(cx).sidecar_collapsed);
            assert_eq!(shell.sidecar_width_now(), SIDECAR_RAIL_WIDTH);
        });
        assert!(cx.debug_bounds("sidecar-panel-machine-0").is_none());
        assert!(cx.debug_bounds("sidecar-rail-beside").is_some());
        click(cx, "sidecar-expand");
        shell.read_with(cx, |shell, cx| {
            assert!(!shell.settings.sidecar_collapsed);
            assert!(!crate::settings::current(cx).sidecar_collapsed);
            assert_eq!(shell.sidecar_width_now(), SIDECAR_WIDTH);
        });
        assert!(cx.debug_bounds("sidecar-panel-machine-0").is_some());
    }

    #[gpui::test]
    fn with_the_pane_open_each_rail_button_pops_its_section_and_a_row_closes_it(
        cx: &mut TestAppContext,
    ) {
        let (shell, cx) = setup(cx);
        shell.update(cx, |shell, cx| shell.toggle_right_pane(cx));
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(cx.debug_bounds("sidecar-rail-workspace").is_some());
        assert!(cx.debug_bounds("sidecar-rail-beside").is_some());
        assert!(cx.debug_bounds("sidecar-popover").is_none());
        click(cx, "sidecar-rail-beside");
        shell.read_with(cx, |shell, _| {
            assert_eq!(shell.sidecar_popover_part(), Some(SidecarPart::Beside));
        });
        assert!(cx.debug_bounds("sidecar-popover-machine-0").is_none());
        click(cx, "sidecar-popover-child-1");
        shell.read_with(cx, |shell, cx| {
            assert!(!shell.sidecar_popover_open());
            let RightSurface::Sidechat(id) = shell.resolved_right_active(cx) else {
                panic!("the side chat must be active");
            };
            assert_eq!(shell.sidechat_tabs[&id].chat_id, "side-b");
        });
        shell.update(cx, |shell, cx| {
            shell.set_sidecar_popover(Some(SidecarPart::Beside), cx);
            assert!(shell.capture_escape_surface(cx));
            assert!(!shell.sidecar_popover_open());
        });
        cx.update(|window, cx| window.draw(cx).clear());
        click(cx, "sidecar-rail-workspace");
        assert!(cx.debug_bounds("sidecar-popover-machine-0").is_some());
        click(cx, "sidecar-rail-beside");
        shell.read_with(cx, |shell, _| {
            assert_eq!(
                shell.sidecar_popover_part(),
                Some(SidecarPart::Beside),
                "another rail button swaps the section"
            );
        });
        click(cx, "sidecar-rail-beside");
        shell.read_with(cx, |shell, _| {
            assert!(
                !shell.sidecar_popover_open(),
                "the same button closes its popover instead of reopening it"
            );
        });
        click(cx, "sidecar-expand");
        shell.read_with(cx, |shell, cx| {
            assert!(!shell.right_pane_open(cx), "expand closes the pane");
            assert!(!shell.sidecar_popover_open());
        });
        cx.update(|window, cx| window.draw(cx).clear());
        shell.read_with(cx, |shell, _| {
            assert_eq!(shell.sidecar_width_now(), SIDECAR_WIDTH);
        });
    }

    #[gpui::test]
    fn the_flip_eases_the_column_between_panel_and_rail(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        shell.update(cx, |shell, cx| {
            shell.reduced_motion = false;
            shell.settings.sidecar_collapsed = true;
            shell.sidecar_layout(1000.0, false, cx);
            let now = std::time::Instant::now();
            shell.render_time = Some(now);
            shell.sidecar_flip.as_mut().unwrap().started = now - Duration::from_millis(100);
            let mid = shell.sidecar_width_now();
            assert!(
                mid > SIDECAR_RAIL_WIDTH && mid < SIDECAR_WIDTH,
                "mid-flight width {mid}"
            );
            assert!(shell.motion_active.get(), "a live flip keeps frames coming");
            shell.sidecar_flip.as_mut().unwrap().started = now - Duration::from_millis(400);
            assert_eq!(shell.sidecar_width_now(), SIDECAR_RAIL_WIDTH);
        });
    }

    #[gpui::test]
    fn the_computer_entry_appears_after_a_drive_and_toggles_its_thumbnail(cx: &mut TestAppContext) {
        use crate::computer::preview::tests::{driving, image, thread_state};
        use zeron_proto::computer::{ComputerEvent, ComputerSettings};
        let (shell, cx) = setup(cx);
        let store = cx.update(|_, cx| preview::store(cx).unwrap());
        let change =
            |cx: &mut VisualTestContext,
             change: &dyn Fn(&mut preview::ComputerPreviewStore) -> bool| {
                cx.update(|_, cx| preview::update_store(&store, cx, change));
                cx.update(|window, cx| window.draw(cx).clear());
            };
        let open =
            |cx: &mut VisualTestContext| cx.update(|_, cx| store.read(cx).view("parent").open);
        assert!(cx.debug_bounds("sidecar-panel-computer-0").is_none());
        change(cx, &|store| {
            store.set_events_live(true);
            driving(store, "parent");
            store.apply_event(&ComputerEvent::Action {
                window_id: Some("w1".into()),
                delivery: None,
                action: "computer_click".into(),
                ok: true,
                message: None,
                thread_id: Some("parent".into()),
                restored_window_id: None,
                restore_status: None,
                masked: false,
            });
            store.accept_tap_frame("parent", image(1, 960, 600))
        });
        assert!(cx.debug_bounds("sidecar-panel-computer-0").is_some());
        let thumb = cx
            .debug_bounds("sidecar-computer-thumbnail")
            .expect("rail placement shows the thumbnail");
        assert_eq!(f32::from(thumb.size.height).round(), 118.0);
        click(cx, "sidecar-panel-computer-0");
        assert!(cx.debug_bounds("sidecar-computer-thumbnail").is_none());
        click(cx, "sidecar-panel-computer-0");
        assert!(cx.debug_bounds("sidecar-computer-thumbnail").is_some());

        change(cx, &|store| {
            store.apply_event(&ComputerEvent::ThreadState {
                state: Box::new(thread_state("parent", false)),
            })
        });
        assert!(
            cx.debug_bounds("sidecar-panel-computer-0").is_some(),
            "the entry outlives the drive"
        );
        assert!(!open(cx));
        click(cx, "sidecar-computer-thumbnail");
        assert!(
            open(cx),
            "the thumbnail reopens the preview for the pop-out"
        );

        shell.update(cx, |shell, cx| shell.set_sidecar_user_collapsed(true, cx));
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(cx.debug_bounds("sidecar-computer-thumbnail").is_none());
        assert!(cx.debug_bounds("sidecar-rail-workspace").is_some());
        shell.update(cx, |shell, cx| shell.set_sidecar_user_collapsed(false, cx));

        change(cx, &|store| {
            store.set_settings(ComputerSettings {
                computer_preview_placement: ComputerPreviewPlacement::Card,
                ..ComputerSettings::default()
            })
        });
        assert!(cx.debug_bounds("sidecar-panel-computer-0").is_some());
        assert!(
            cx.debug_bounds("sidecar-computer-thumbnail").is_none(),
            "the card placement has no thumbnail"
        );
        assert!(open(cx));
        click(cx, "sidecar-panel-computer-0");
        assert!(!open(cx), "the entry hides an open card");
        click(cx, "sidecar-panel-computer-0");
        assert!(open(cx), "and reopens a hidden one");
    }

    #[gpui::test]
    fn a_device_row_shows_the_chats_open_device_and_focuses_its_tab_or_float(
        cx: &mut TestAppContext,
    ) {
        use zeron_proto::device_hub::{
            DeviceHubSettings, DeviceHubState, DeviceOpener, DevicePlatform, DeviceSession,
            HubDevice,
        };
        let (shell, cx) = setup(cx);
        assert!(cx.debug_bounds("sidecar-panel-device-0").is_none());
        let device = |id: &str, name: &str, platform| HubDevice {
            host_id: "local".into(),
            id: id.into(),
            name: name.into(),
            platform,
            booted: true,
            ..Default::default()
        };
        let session = |chat: &str, id: &str, opened_by| DeviceSession {
            chat_id: chat.into(),
            host_id: "local".into(),
            device_id: id.into(),
            opened_by,
            ..Default::default()
        };
        let hub = DeviceHubState {
            settings: DeviceHubSettings {
                enabled: true,
                onboarding_completed: true,
                ..Default::default()
            },
            devices: vec![
                device("DUO", "iPhone Duo", DevicePlatform::Ios),
                device("emulator-5554", "Pixel_Fold", DevicePlatform::Android),
            ],
            sessions: vec![
                session("side-a", "DUO", Some(DeviceOpener::User)),
                session("parent", "DUO", Some(DeviceOpener::Agent)),
                session("parent", "emulator-5554", None),
                session("parent", "GONE", Some(DeviceOpener::User)),
            ],
            ..Default::default()
        };
        shell.update(cx, |shell, cx| {
            shell
                .state
                .update(cx, |state, _| state.device_hub = Some(hub));
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear());
        let rows = shell.update(cx, |shell, cx| {
            shell
                .sidecar_devices("parent", cx)
                .into_iter()
                .map(|(target, name, platform, opener)| (target.1, name, platform, opener))
                .collect::<Vec<_>>()
        });
        assert_eq!(
            rows,
            [
                (
                    "DUO".to_string(),
                    "iPhone Duo".to_string(),
                    DevicePlatform::Ios,
                    Some(DeviceOpener::Agent)
                ),
                (
                    "emulator-5554".to_string(),
                    "Pixel_Fold".to_string(),
                    DevicePlatform::Android,
                    None
                ),
            ],
            "only this chat's sessions on listed devices"
        );
        assert!(cx.debug_bounds("sidecar-panel-device-0").is_some());
        assert!(cx.debug_bounds("sidecar-panel-device-1").is_some());
        assert!(cx.debug_bounds("sidecar-panel-device-2").is_none());

        let device_tabs = |shell: &Shell, cx: &App| {
            shell
                .right_surface_rows(cx)
                .into_iter()
                .filter_map(|(surface, ..)| match surface {
                    RightSurface::Device(id) => Some(id),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        click(cx, "sidecar-panel-device-0");
        assert!(cx.debug_bounds("sidecar-rail-workspace").is_some());
        let tab = shell.read_with(cx, |shell, cx| {
            assert!(shell.right_pane_open(cx));
            let RightSurface::Device(id) = shell.resolved_right_active(cx) else {
                panic!("the device tab must be active");
            };
            assert_eq!(
                shell.device_panels[&id].0.read(cx).target(),
                Some(&("local".to_string(), "DUO".to_string()))
            );
            id
        });
        cx.update(|window, cx| {
            shell.update(cx, |shell, cx| shell.add_browser_surface(None, window, cx))
        });
        cx.update(|window, cx| window.draw(cx).clear());
        assert_ne!(
            shell.read_with(cx, |shell, cx| shell.resolved_right_active(cx)),
            RightSurface::Device(tab)
        );
        click(cx, "sidecar-rail-workspace");
        click(cx, "sidecar-popover-device-0");
        shell.read_with(cx, |shell, cx| {
            assert_eq!(shell.resolved_right_active(cx), RightSurface::Device(tab));
            assert_eq!(device_tabs(shell, cx), [tab], "the shown tab is reused");
        });

        shell.update(cx, |shell, cx| {
            shell.toggle_right_pane(cx);
            shell.open_device_float(
                "parent".into(),
                ("local".into(), "emulator-5554".into()),
                cx,
            );
        });
        cx.update(|window, cx| window.draw(cx).clear());
        click(cx, "sidecar-panel-device-1");
        shell.read_with(cx, |shell, cx| {
            assert!(!shell.right_pane_open(cx), "the float is the surface");
            assert_eq!(device_tabs(shell, cx), [tab]);
        });

        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                if let Some(hub) = state.device_hub.as_mut() {
                    hub.sessions.retain(|session| session.chat_id != "parent");
                }
            });
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(cx.debug_bounds("sidecar-panel-device-0").is_none());
    }

    #[gpui::test]
    fn a_narrow_column_or_a_split_changes_the_shape(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        shell.update(cx, |shell, cx| {
            assert_eq!(shell.sidecar_layout(900.0, false, cx), SIDECAR_WIDTH);
            assert_eq!(shell.sidecar_layout(640.0, false, cx), SIDECAR_RAIL_WIDTH);
            assert_eq!(shell.sidecar_layout(900.0, true, cx), 0.0);
            assert_eq!(shell.sidecar_layout(20.0, false, cx), 20.0);
        });
    }

    #[gpui::test]
    fn beside_lists_agents_then_children_and_folds_the_finished_agents(cx: &mut TestAppContext) {
        use zeron_doc::{MessagePart, MessageStatus, SessionMessageEntry, SubagentStatus};
        let (shell, cx) = setup(cx);
        let spawn = |id: &str, call: zeron_proto::ToolCall, sub: &str, status| MessagePart::Tool {
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
            subagent_ref: Some(sub.into()),
            subagent_status: status,
            subagent_tail: None,
            html_render: None,
            delegate_route: None,
        };
        let agent = |id: &str, status| {
            spawn(
                id,
                zeron_proto::ToolCall::Unknown {
                    name: format!("Agent: task {id}"),
                    input: Some(serde_json::json!({ "subagent_type": "Explore" })),
                },
                &format!("parent--sub--{id}"),
                Some(status),
            )
        };
        let delegate = spawn(
            "d",
            zeron_proto::ToolCall::Mcp {
                server: "zeron".into(),
                tool: "delegate_task".into(),
                input: None,
            },
            "kid",
            None,
        );
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, cx| {
                let config = serde_json::json!({
                    "harness": "codex", "model": "gpt-5", "reasoning": null,
                    "sandbox": "workspace-write"
                });
                let parent = state.chats.iter_mut().find(|c| c.id == "parent").unwrap();
                parent.config = Some(serde_json::from_value(config).unwrap());
                let kid: zeron_proto::Chat = serde_json::from_value(serde_json::json!({
                    "id": "kid", "deviceId": "dev", "title": "Kid",
                    "archived": false, "createdAt": "2026-09-18T00:00:00Z",
                    "config": {
                        "harness": "opencode", "model": "anthropic/claude-opus-5",
                        "reasoning": null, "sandbox": "workspace-write"
                    },
                    "delegation": {
                        "delegatedByChatId": "parent", "depth": 1, "anchor": "m",
                        "wake": "always", "status": "running",
                        "startedAt": "2026-09-18T00:00:00Z"
                    }
                }))
                .unwrap();
                state.chats.push(kid);
                state.apply_transcript(vec![SessionMessageEntry {
                    id: "m1".into(),
                    role: zeron_doc::MessageRole::Assistant,
                    parts: vec![
                        agent("a", SubagentStatus::Running),
                        agent("b", SubagentStatus::Done),
                        agent("c", SubagentStatus::Failed),
                        delegate,
                    ],
                    created_at: 0,
                    device_id: "dev".into(),
                    status: Some(MessageStatus::Complete),
                    continuation_of: None,
                    from_chat_id: None,
                    fork_import: false,
                    answers_request_id: None,
                    duration_ms: None,
                }]);
                cx.notify();
            });
        });
        cx.update(|window, cx| window.draw(cx).clear());
        shell.update(cx, |shell, cx| {
            let theme = Theme::of(cx).clone();
            let sections = shell.sidecar_sections(&theme, cx);
            let section = sections
                .iter()
                .find(|section| section.part == SidecarPart::Beside)
                .expect("beside section");
            assert_eq!(section.label().as_ref(), "Beside this thread · 2 running");
            let labels: Vec<_> = section
                .entries
                .iter()
                .map(|e| e.label.to_string())
                .collect();
            assert_eq!(labels, ["Kid", "task a", "Parent", "Parent"]);
            let icons: Vec<_> = section.entries.iter().map(|e| e.icon).collect();
            assert_eq!(
                icons,
                [
                    ChildKind::Agent.icon(),
                    ChildKind::Agent.icon(),
                    ChildKind::SideChat.icon(),
                    ChildKind::SideChat.icon(),
                ]
            );
            assert_eq!(
                section.entries[0].tooltip().as_ref(),
                "Kid · Agent · anthropic/claude-opus-5"
            );
            assert_eq!(
                section.entries[1].tooltip().as_ref(),
                "task a · Agent · Explore"
            );
            let fold = section.fold.as_ref().expect("previous agents fold");
            assert_eq!(fold.label.as_ref(), "Previous agents (2)");
            assert_eq!(fold.failed, 1);
            assert_eq!(fold.entries[0].value_text().as_deref(), Some("Failed"));
            assert_eq!(fold.entries[1].value_text().as_deref(), Some("Done"));
        });
        assert!(cx.debug_bounds("sidecar-panel-subagent-1").is_some());
        assert!(cx.debug_bounds("sidecar-panel-previous-agent-0").is_none());
        click(cx, "sidecar-panel-fold");
        assert!(cx.debug_bounds("sidecar-panel-previous-agent-1").is_some());
        click(cx, "sidecar-panel-fold");
        assert!(cx.debug_bounds("sidecar-panel-previous-agent-0").is_none());
    }

    #[test]
    fn subagents_split_running_from_previous_newest_first() {
        use zeron_doc::SubagentStatus::{Done, Failed, Running};
        let agent = |id: &str, status| {
            let agent = crate::transcript::SubagentSummary {
                chat_id: "parent".into(),
                doc_id: id.into(),
                title: id.to_string().into(),
                status,
                delegated: false,
                harness: None,
                model: None,
                kind: None,
                started_at: None,
            };
            (agent, None)
        };
        let (running, previous) = split_subagents(vec![
            agent("a", Some(Done)),
            agent("b", Some(Running)),
            agent("c", Some(Failed)),
            agent("d", Some(Running)),
            agent("e", None),
        ]);
        let ids = |agents: &[LabeledSubagent]| {
            agents
                .iter()
                .map(|(a, _)| a.doc_id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&running), ["d", "b"]);
        assert_eq!(ids(&previous), ["e", "c", "a"]);
    }

    #[test]
    fn subagent_elapsed_keeps_one_unit() {
        assert_eq!(short_elapsed(-5), "0s");
        assert_eq!(short_elapsed(59), "59s");
        assert_eq!(short_elapsed(21 * 60 + 14), "21m");
        assert_eq!(short_elapsed(3 * 3_600 + 59 * 60), "3h");
        assert_eq!(short_elapsed(2 * 86_400 + 5), "2d");
        assert_eq!(elapsed_next_change(-5), 1);
        assert_eq!(elapsed_next_change(59), 1);
        assert_eq!(elapsed_next_change(21 * 60 + 14), 46);
        assert_eq!(elapsed_next_change(3 * 3_600 + 59 * 60), 60);
    }

    #[test]
    fn sidecar_labels_are_primary_text_and_headings_stay_muted() {
        for theme in [Theme::dark(), Theme::light()] {
            let ink = SidecarInk::of(&theme);
            assert_eq!(ink.label, theme.text);
            assert_eq!(ink.heading, theme.text_muted);
            assert_eq!(ink.label.l > ink.heading.l, theme.appearance.is_dark());
        }
    }

    fn attach_engine(
        shell: &Entity<Shell>,
        runtime: &tokio::runtime::Runtime,
        cx: &mut VisualTestContext,
    ) -> crate::device_panel::tests::Calls {
        let (engine, calls) = crate::device_panel::tests::fake_engine(runtime);
        shell.update(cx, |shell, cx| {
            shell
                .state
                .update(cx, |state, _| state.set_test_engine(engine))
        });
        cx.update(|window, cx| window.draw(cx).clear());
        calls
    }

    fn called(
        cx: &mut VisualTestContext,
        calls: &crate::device_panel::tests::Calls,
        method: &str,
        count: usize,
    ) -> Vec<serde_json::Value> {
        crate::device_panel::tests::wait(cx, |_| {
            crate::device_panel::tests::calls_of(calls, method).len() >= count
        });
        crate::device_panel::tests::calls_of(calls, method)
    }

    #[test]
    fn the_commit_row_never_says_ahead_of_the_branch_it_is_on() {
        let cmp = |ahead, behind| GitHistoryComparison {
            base: "origin/main".into(),
            ahead,
            behind,
        };
        let line = |branch, cmp: Option<GitHistoryComparison>| {
            commit_line(Some("a1b2c3d"), Some(branch), cmp.as_ref())
        };
        assert_eq!(
            line("checkout-stripe", Some(cmp(2, 0))).as_deref(),
            Some("a1b2c3d · 2 ahead of main")
        );
        assert_eq!(
            line("checkout-stripe", Some(cmp(2, 3))).as_deref(),
            Some("a1b2c3d · 2 ahead, 3 behind main")
        );
        assert_eq!(
            line("checkout-stripe", Some(cmp(0, 1))).as_deref(),
            Some("a1b2c3d · 1 behind main")
        );
        assert_eq!(line("main", Some(cmp(2, 1))).as_deref(), Some("a1b2c3d"));
        assert_eq!(
            line("main", Some(cmp(0, 0))).as_deref(),
            Some("a1b2c3d · up to date")
        );
        assert_eq!(line("main", None).as_deref(), Some("a1b2c3d"));
        assert_eq!(commit_line(None, Some("main"), None), None);
    }

    #[gpui::test]
    fn the_actions_section_runs_edits_imports_and_adds_project_actions(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        cx.executor().allow_parking();
        let (shell, cx) = setup(cx);
        let calls = attach_engine(&shell, &runtime, cx);
        let key = crate::project_actions::ProjectActionsKey {
            device_id: "dev".into(),
            space_id: "s1".into(),
        };
        let snapshot: zeron_proto::ProjectActionsSnapshot =
            serde_json::from_value(serde_json::json!({
                "spaceId": "s1",
                "actions": [{
                    "id": "dev", "name": "Dev", "command": "bun dev",
                    "icon": "play", "runOnWorktreeCreate": false
                }],
                "importableActions": [{
                    "name": "Lint", "command": "bun lint", "icon": "lint"
                }],
                "projectFileIssue": null
            }))
            .unwrap();
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state.spaces = vec![
                    serde_json::from_value(serde_json::json!({
                        "id": "s1", "deviceId": "dev", "path": "/repo",
                        "gitDetected": false, "createdAt": "2026-09-18T00:00:00Z"
                    }))
                    .unwrap(),
                ];
                let parent = state.chats.iter_mut().find(|c| c.id == "parent").unwrap();
                parent.space_id = Some("s1".into());
            });
            shell.project_actions.activate(Some(key.clone()));
            let generation = shell.project_actions.begin_load(&key);
            assert!(
                shell
                    .project_actions
                    .accept_load(&key, generation, Ok(snapshot))
            );
        });
        cx.update(|window, cx| window.draw(cx).clear());

        let parts = shell.update(cx, |shell, cx| {
            let theme = Theme::of(cx).clone();
            shell
                .sidecar_sections(&theme, cx)
                .iter()
                .map(|section| section.part)
                .collect::<Vec<_>>()
        });
        assert_eq!(
            parts,
            [
                SidecarPart::Workspace,
                SidecarPart::Beside,
                SidecarPart::Actions
            ]
        );
        let rows = [
            "sidecar-panel-action-0",
            "sidecar-panel-import-action-0",
            "sidecar-panel-add-action-0",
        ];
        let tops: Vec<_> = rows
            .iter()
            .map(|row| cx.debug_bounds(row).unwrap().top())
            .collect();
        assert!(tops.windows(2).all(|pair| pair[0] < pair[1]));

        click(cx, "sidecar-panel-add-action-0");
        shell.update(cx, |shell, _| {
            let editor = shell
                .project_actions
                .editor
                .take()
                .expect("Add action opens the editor");
            assert_eq!(editor.action_id, None);
        });
        click(cx, "sidecar-panel-import-action-0");
        shell.update(cx, |shell, cx| {
            let editor = shell
                .project_actions
                .editor
                .take()
                .expect("Import opens the editor");
            assert_eq!(editor.action_id, None);
            assert_eq!(editor.name.read(cx).text(), "Lint");
        });
        let row = cx.debug_bounds("sidecar-panel-action-0").unwrap();
        cx.simulate_mouse_move(row.center(), None, gpui::Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear());
        click(cx, "sidecar-panel-action-0-trail");
        shell.update(cx, |shell, _| {
            let editor = shell.project_actions.editor.take().expect("the gear edits");
            assert_eq!(editor.action_id.as_deref(), Some("dev"));
        });

        let action_tooltip = |cx: &mut VisualTestContext| {
            shell.update(cx, |shell, cx| {
                let theme = Theme::of(cx).clone();
                shell
                    .sidecar_sections(&theme, cx)
                    .into_iter()
                    .flat_map(|section| section.entries)
                    .find(|entry| entry.key == "action")
                    .unwrap()
                    .tooltip()
            })
        };
        assert_eq!(action_tooltip(cx).as_ref(), "Dev");
        click(cx, "sidecar-panel-action-0");
        assert_eq!(action_tooltip(cx).as_ref(), "Dev: Running");
        let runs = called(cx, &calls, zeron_rpc::methods::RUN_PROJECT_ACTION, 1);
        assert_eq!(runs[0]["actionId"], "dev");
        assert_eq!(runs[0]["spaceId"], "s1");
        assert_eq!(runs[0]["chatId"], "parent");

        crate::device_panel::tests::wait(cx, |cx| {
            shell.read_with(cx, |shell, _| {
                matches!(
                    shell.project_actions.active_status(),
                    Some(ProjectActionsStatus::Unavailable { .. })
                )
            })
        });
        assert_eq!(action_tooltip(cx).as_ref(), "Dev: Failed");
        shell.update(cx, |shell, _| {
            let generation = shell.project_actions.begin_load(&key);
            let snapshot = shell.project_actions.active_snapshot().cloned().unwrap();
            shell
                .project_actions
                .accept_load(&key, generation, Ok(snapshot));
        });
        click(cx, "sidecar-collapse");
        click(cx, "sidecar-rail-actions");
        assert!(cx.debug_bounds("sidecar-popover-add-action-0").is_some());
        click(cx, "sidecar-popover-action-0");
        called(cx, &calls, zeron_rpc::methods::RUN_PROJECT_ACTION, 2);
    }
}
