//! Astra's sidecar (paseo-fork `Sidecar.tsx` + `sidecar.css`) beside the transcript.

use super::*;
use crate::computer::{self, presence::PresenceTone, preview};
use crate::files::client::{FilesRequestContext, WorkspaceFilesClient};
use crate::project_actions::{ProjectActionsStatus, action_icon, action_label};
use std::rc::Rc;
use zeron_proto::computer::ComputerPreviewPlacement;
use zeron_proto::device_hub::{DeviceOpener, DevicePlatform};

pub(super) const SIDECAR_WIDTH: f32 = 260.0;
pub(super) const SIDECAR_RAIL_WIDTH: f32 = 40.0;
const SIDECAR_AUTO_COLLAPSE_WIDTH: f32 = 640.0;
const SIDECAR_TOP: f32 = Theme::TITLEBAR_HEIGHT + 4.0;
const POPOVER_WIDTH: f32 = 276.0;
const SECTION_CAP: usize = 8;
/// Instructions files probed at the checkout root, first hit wins.
const INSTRUCTION_FILES: &[&str] = &["AGENTS.md", "CLAUDE.md"];

#[derive(Debug, Clone, Copy)]
pub(super) struct SidecarFlip {
    from: f32,
    started: std::time::Instant,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PopoverFade {
    open: bool,
    started: std::time::Instant,
}

type SidecarAction = Rc<dyn Fn(&mut Shell, &mut Window, &mut Context<Shell>)>;

enum SidecarValue {
    Text(SharedString),
    Mono(SharedString),
    Tone(SharedString, gpui::Hsla),
    Dot(Option<SharedString>, gpui::Hsla),
    Counts(usize, u32, u32),
    Meter(f32, gpui::Hsla),
}

struct SidecarEntry {
    key: &'static str,
    ix: usize,
    icon: &'static str,
    tint: Option<gpui::Hsla>,
    badge: Option<gpui::Hsla>,
    ink: Option<gpui::Hsla>,
    label: SharedString,
    suffix: Option<SharedString>,
    value: Option<SidecarValue>,
    action: Option<SidecarAction>,
    edit: Option<SidecarAction>,
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
            tint: None,
            badge: None,
            ink: None,
            label: label.into(),
            suffix: None,
            value: None,
            action: None,
            edit: None,
            thumbnail: None,
        }
    }

    fn tint(mut self, tint: Option<gpui::Hsla>) -> Self {
        self.tint = tint;
        self
    }

    fn badge(mut self, badge: Option<gpui::Hsla>) -> Self {
        self.badge = badge;
        self
    }

    fn ink(mut self, ink: gpui::Hsla) -> Self {
        self.ink = Some(ink);
        self
    }

    fn suffix(mut self, suffix: Option<SharedString>) -> Self {
        self.suffix = suffix;
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

    fn edit(
        mut self,
        edit: impl Fn(&mut Shell, &mut Window, &mut Context<Shell>) + 'static,
    ) -> Self {
        self.edit = Some(Rc::new(edit));
        self
    }

    fn name(&self) -> SharedString {
        match &self.suffix {
            Some(suffix) => format!("{}{suffix}", self.label).into(),
            None => self.label.clone(),
        }
    }

    fn value_text(&self) -> Option<SharedString> {
        match self.value.as_ref()? {
            SidecarValue::Text(v)
            | SidecarValue::Mono(v)
            | SidecarValue::Tone(v, _)
            | SidecarValue::Dot(Some(v), _) => Some(v.clone()),
            SidecarValue::Meter(fraction, _) => {
                Some(format!("{}%", (fraction * 100.0).round() as u32).into())
            }
            SidecarValue::Counts(files, add, del) => {
                let files = if *files == 1 {
                    "1 file".to_string()
                } else {
                    format!("{files} files")
                };
                Some(format!("{files} +{add} −{del}").into())
            }
            SidecarValue::Dot(None, _) => None,
        }
    }

    fn tooltip(&self) -> SharedString {
        let label = self.name();
        match (&self.value, self.value_text()) {
            (Some(SidecarValue::Counts(..)), _) | (_, None) => label,
            (_, Some(value)) => format!("{label}: {value}").into(),
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
    heading: SharedString,
    entries: Vec<SidecarEntry>,
    fold: Option<SidecarFold>,
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
        let popover_mode = self.right_pane_shown(cx);
        let collapsed = self.settings.sidecar_collapsed
            || popover_mode
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
        if !(popover_mode && collapsed) && self.sidecar_popover_open() {
            self.sidecar_popover = Some(PopoverFade {
                open: false,
                started: motion::now(),
            });
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
        matches!(self.sidecar_popover, Some(PopoverFade { open: true, .. }))
    }

    fn sidecar_popover_mounted(&self) -> bool {
        self.sidecar_popover.is_some_and(|fade| {
            fade.open
                || motion::since(fade.started)
                    < motion::SIDECAR_POPOVER_SLIDE
                        .total()
                        .mul_f32(motion::speed_scale())
        })
    }

    pub(super) fn set_sidecar_popover(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.sidecar_popover_open() == open {
            return;
        }
        self.sidecar_popover = Some(PopoverFade {
            open,
            started: motion::now(),
        });
        cx.notify();
    }

    fn set_sidecar_user_collapsed(&mut self, collapsed: bool, cx: &mut Context<Self>) {
        if self.settings.sidecar_collapsed != collapsed {
            self.settings.sidecar_collapsed = collapsed;
            self.schedule_save(cx);
        }
        cx.notify();
    }

    /// Bring one right-pane surface on screen: open the pane if it is closed,
    /// then make that tab active.
    fn focus_right_surface(&mut self, surface: RightSurface, cx: &mut Context<Self>) {
        if !self.right_pane_open(cx) {
            self.toggle_right_pane(cx);
        }
        self.set_right_active(surface, cx);
    }

    fn open_right_pane(&mut self, cx: &mut Context<Self>) {
        if !self.right_pane_open(cx) {
            self.toggle_right_pane(cx);
        }
    }

    fn first_right_surface(
        &self,
        cx: &App,
        matches: fn(&RightSurface) -> bool,
    ) -> Option<RightSurface> {
        self.right_surface_rows(cx)
            .into_iter()
            .map(|(surface, ..)| surface)
            .find(matches)
    }

    /// The Changes row: focus the working-tree diff tab, else the first diff
    /// tab, else open one.
    pub(super) fn focus_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.right_surface_rows(cx);
        let diffs: Vec<u64> = rows
            .iter()
            .filter_map(|(surface, ..)| match surface {
                RightSurface::Diff(id) => Some(*id),
                _ => None,
            })
            .collect();
        let target = diffs
            .iter()
            .copied()
            .find(|id| {
                self.diffs
                    .get(id)
                    .is_some_and(|changes| changes.read(cx).is_working_tree())
            })
            .or(diffs.first().copied());
        match target {
            Some(id) => self.focus_right_surface(RightSurface::Diff(id), cx),
            None => {
                self.open_right_pane(cx);
                self.add_diff_surface(window, cx);
            }
        }
    }

    fn focus_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.first_right_surface(cx, |s| matches!(s, RightSurface::Browser(_))) {
            Some(surface) => self.focus_right_surface(surface, cx),
            None => {
                self.open_right_pane(cx);
                self.add_browser_surface(None, window, cx);
            }
        }
    }

    fn focus_terminal(&mut self, cx: &mut Context<Self>) {
        match self.first_right_surface(cx, |s| matches!(s, RightSurface::Terminal(_))) {
            Some(surface) => self.focus_right_surface(surface, cx),
            None => {
                self.open_right_pane(cx);
                self.add_terminal_surface(cx);
            }
        }
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

    /// Look for an instructions file at the chat's checkout root, once per chat.
    fn probe_summary_instructions(&mut self, chat_id: String, cx: &mut Context<Self>) {
        if self.summary_instruction_probes.contains_key(&chat_id)
            || self.summary_instructions.contains_key(&chat_id)
        {
            return;
        }
        let state = self.state.read(cx);
        let Some(context) = FilesRequestContext::for_chat(state, &chat_id) else {
            return;
        };
        let Some(engine) = state.engine().cloned() else {
            return;
        };
        let key = chat_id.clone();
        let task = cx.spawn(async move |this, cx| {
            let client = WorkspaceFilesClient::new(engine, context.clone());
            let mut found = None;
            for name in INSTRUCTION_FILES {
                let request = zeron_proto::ReadWorkspaceFileRequest {
                    target: context.target.clone(),
                    path: (*name).into(),
                };
                match client.read_file(request).await {
                    Ok(_) => {
                        found = Some((*name).to_string());
                        break;
                    }
                    Err(error) if error.retryable() => break,
                    Err(_) => continue,
                }
            }
            let _ = this.update(cx, |this, cx| {
                this.summary_instructions.insert(chat_id.clone(), found);
                this.summary_instruction_probes.remove(&chat_id);
                cx.notify();
            });
        });
        self.summary_instruction_probes.insert(key, task);
    }

    /// Working-tree counts from the first diff tab that carries the selected
    /// chat's watch. Without a diff tab there is no watch, so no counts.
    fn summary_change_counts(&self, cx: &App) -> Option<(usize, u32, u32)> {
        self.right_surface_rows(cx)
            .iter()
            .filter_map(|(surface, ..)| match surface {
                RightSurface::Diff(id) => self.diffs.get(id),
                _ => None,
            })
            .find_map(|changes| {
                let changes = changes.read(cx);
                changes
                    .is_working_tree()
                    .then(|| changes.working_tree_counts(cx))
                    .flatten()
            })
    }

    /// The most-used usage window of the active account on the chat's
    /// harness: `(label, fraction)`.
    fn summary_usage(&self, chat: &zeron_proto::Chat, cx: &App) -> Option<(String, f32)> {
        let harness = chat.config.as_ref()?.harness;
        crate::account_usage::local_snapshot(cx)?
            .busiest_window(harness)
            .map(|(label, fraction)| (label.to_owned(), fraction))
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
                return Some(SidecarSection {
                    heading: "Actions".into(),
                    entries,
                    fold: None,
                });
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
            entries.push(entry.edit(move |this, _, cx| {
                this.open_project_action_editor(Some(action.clone()), None, cx)
            }));
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
        Some(SidecarSection {
            heading: "Actions".into(),
            entries,
            fold: None,
        })
    }

    fn sidecar_sections(&mut self, theme: &Theme, cx: &mut Context<Self>) -> Vec<SidecarSection> {
        self.ensure_project_actions(cx);
        let chat = self.state.read(cx).selected_chat_row().cloned();
        if let Some(chat) = chat.as_ref() {
            self.probe_summary_instructions(chat.id.clone(), cx);
        }
        let surfaces = self.right_surface_rows(cx);
        let browsers: Vec<(RightSurface, SharedString, Option<SharedString>)> = surfaces
            .iter()
            .filter(|(surface, ..)| matches!(surface, RightSurface::Browser(_)))
            .map(|(surface, title, _, url)| (*surface, title.clone(), url.clone()))
            .collect();
        let files: Vec<(RightSurface, SharedString)> = surfaces
            .iter()
            .filter(|(surface, ..)| matches!(surface, RightSurface::File(_)))
            .map(|(surface, title, ..)| (*surface, title.clone()))
            .collect();
        let source = self.sidechat_source_chat(cx);
        let mut sidechats: Vec<_> = self
            .state
            .read(cx)
            .chats
            .iter()
            .filter(|chat| {
                !chat.archived
                    && source.as_deref().is_some_and(|source| {
                        chat.sidechat_source_chat_id.as_deref() == Some(source)
                    })
            })
            .cloned()
            .collect();
        sidechats.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        let pickers = self.composer.read(cx).pickers().read(cx);
        let subagents: Vec<_> = self
            .transcript
            .read(cx)
            .subagent_summaries(cx)
            .into_iter()
            .map(|agent| {
                let suffix = agent_tag_suffix(&agent, |harness, id| {
                    pickers
                        .model_label(harness, id)
                        .or_else(|| crate::pickers::curated_model_label(harness, id))
                });
                (agent, suffix)
            })
            .collect();
        let sources = self.transcript.read(cx).user_attachments();
        let instructions = chat
            .as_ref()
            .and_then(|chat| self.summary_instructions.get(&chat.id).cloned())
            .flatten();
        let branch = chat.as_ref().and_then(|chat| {
            chat.source_context
                .as_ref()
                .map(|ctx| ctx.branch.clone())
                .or_else(|| chat.branch.clone())
                .filter(|branch| !branch.is_empty())
        });
        let head = chat.as_ref().and_then(|chat| {
            chat.source_context
                .as_ref()
                .and_then(|ctx| ctx.head_sha.as_deref())
                .map(|sha| sha.chars().take(7).collect::<String>())
        });
        let state = self.state.read(cx);
        let device = chat
            .as_ref()
            .and_then(|chat| state.device_name(&chat.device_id).map(str::to_string));
        let space = chat
            .as_ref()
            .and_then(|chat| state.space_for_chat(chat))
            .cloned();
        // Folders need not be repos: no git, no Changes or branch rows.
        let git = space.as_ref().is_some_and(|space| space.git_detected);
        let project = space.as_ref().map(|space| {
            space
                .name
                .clone()
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| basename(&space.path))
        });
        let cwd = chat
            .as_ref()
            .and_then(|chat| chat.cwd.as_deref())
            .map(basename);
        let counts = self.summary_change_counts(cx);
        let usage = chat.as_ref().and_then(|chat| self.summary_usage(chat, cx));
        let working = chat_status_tone(zeron_proto::ChatIndicator::Working, theme).2;

        let mut sections = Vec::new();

        let mut on = Vec::new();
        if git {
            on.push(
                SidecarEntry::new(
                    "changes",
                    0,
                    icons::ASTRA_PLUS_MINUS,
                    zeron_proto::badges::CHANGES_TITLE,
                )
                .value(counts.map(|(files, add, del)| SidecarValue::Counts(files, add, del)))
                .action(|this, window, cx| this.focus_changes(window, cx)),
            );
        }
        on.push(
            SidecarEntry::new("browser", 0, icons::ASTRA_GLOBE, "Browser")
                .action(|this, window, cx| this.focus_browser(window, cx)),
        );
        let live_terminals = self.right_terminal.as_ref().map_or(0, |panel| {
            panel
                .read(cx)
                .tab_summaries(cx)
                .iter()
                .filter(|(_, _, exited)| !exited)
                .count()
        });
        on.push(
            SidecarEntry::new("terminal", 0, icons::ASTRA_TERMINAL, "Terminal")
                .value((live_terminals > 0).then(|| {
                    SidecarValue::Dot(Some(format!("{live_terminals} running").into()), working)
                }))
                .action(|this, _, cx| this.focus_terminal(cx)),
        );
        on.push(
            SidecarEntry::new("files", 0, icons::ASTRA_FILE, "Files")
                .action(|this, window, cx| this.add_files_surface(window, cx)),
        );
        let computer = chat
            .as_ref()
            .and_then(|chat| computer::presence::rail_entry(&chat.id, cx));
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
                .value(Some(SidecarValue::Dot(entry.app.map(Into::into), dot)))
                .action(move |this, _, cx| this.toggle_computer_entry(&chat_id, cx));
            row.thumbnail = entry.thumbnail.then_some(entry.chat_id);
            on.push(row);
        }
        if let Some(chat) = chat.as_ref() {
            on.extend(
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
        sections.push(SidecarSection {
            heading: format!("On {}", project.as_deref().unwrap_or("this session")).into(),
            entries: on,
            fold: None,
        });

        let mut workspace = vec![
            SidecarEntry::new(
                "local",
                0,
                icons::ASTRA_LAPTOP,
                device.unwrap_or_else(|| "Local".into()),
            )
            .value(Some(SidecarValue::Text(
                cwd.unwrap_or_else(|| "No workspace".into()).into(),
            ))),
        ];
        if git {
            workspace.push(
                SidecarEntry::new(
                    "branch",
                    0,
                    icons::ASTRA_GIT_BRANCH,
                    branch.unwrap_or_else(|| "No branch".into()),
                )
                .ink(theme.branch_ink())
                .value(head.map(|sha| SidecarValue::Mono(sha.into()))),
            );
        }
        sections.push(SidecarSection {
            heading: "Workspace".into(),
            entries: workspace,
            fold: None,
        });

        if !browsers.is_empty() {
            sections.push(SidecarSection {
                heading: "Local servers".into(),
                entries: browsers
                    .into_iter()
                    .take(SECTION_CAP)
                    .enumerate()
                    .map(|(ix, (surface, title, url))| {
                        let host = url.as_deref().map(url_host).filter(|host| !host.is_empty());
                        SidecarEntry::new("server", ix, icons::ASTRA_GLOBE, title)
                            .value(host.map(|host| SidecarValue::Text(host.into())))
                            .action(move |this, _, cx| this.focus_right_surface(surface, cx))
                    })
                    .collect(),
                fold: None,
            });
        }

        if chat.is_some() {
            let mut entries: Vec<SidecarEntry> = sidechats
                .into_iter()
                .take(SECTION_CAP)
                .enumerate()
                .map(|(ix, chat)| {
                    let now = Utc::now();
                    let status = if chat.sidechat_expired(now) {
                        Some(SidecarValue::Text("Expired".into()))
                    } else {
                        let (label, _, color) = chat_status_tone(
                            self.state.read(cx).display_status_for(&chat, now),
                            theme,
                        );
                        label.map(|label| SidecarValue::Tone(label, color))
                    };
                    let title = chat.title.clone().unwrap_or_else(|| "Sidechat".into());
                    SidecarEntry::new("sidechat", ix, icons::ASTRA_CHAT_BUBBLE, title)
                        .value(status)
                        .action(move |this, _, cx| {
                            this.add_sidechat_surface(&chat, cx);
                        })
                })
                .collect();
            if let Some(source) = source {
                entries.push(
                    SidecarEntry::new(
                        "new-sidechat",
                        0,
                        icons::ASTRA_PLUS,
                        zeron_proto::sidechat::NEW_SIDECHAT,
                    )
                    .action(move |this, _, cx| {
                        this.create_sidechat(source.clone(), None, Default::default(), cx)
                    }),
                );
            }
            entries.push(
                SidecarEntry::new("fork-chat", 0, icons::FORK, "Fork chat")
                    .action(|this, _, cx| this.create_side_chat(cx)),
            );
            sections.push(SidecarSection {
                heading: "Side chats".into(),
                entries,
                fold: None,
            });
        }

        sections.extend(self.sidecar_actions_section(theme, cx));

        if !subagents.is_empty() {
            let (running, previous) = split_subagents(subagents);
            let failed = previous
                .iter()
                .filter(|(agent, _)| agent.status == Some(zeron_doc::SubagentStatus::Failed))
                .count();
            let now = Utc::now().timestamp_millis();
            let next_tick = running
                .iter()
                .filter_map(|(agent, _)| agent.started_at)
                .map(|started| elapsed_next_change((now - started) / 1000))
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
            let entry = |key: &'static str, ix: usize, (agent, suffix): LabeledSubagent| {
                let running = agent.status == Some(zeron_doc::SubagentStatus::Running);
                let badge = match agent.status {
                    Some(zeron_doc::SubagentStatus::Running) => Some(theme.busy),
                    Some(zeron_doc::SubagentStatus::Failed) => Some(theme.danger),
                    _ => None,
                };
                let status = match agent.status {
                    Some(zeron_doc::SubagentStatus::Running) => Some(SidecarValue::Tone(
                        agent.started_at.map_or("Working".into(), |started| {
                            short_elapsed((now - started) / 1000).into()
                        }),
                        working,
                    )),
                    Some(zeron_doc::SubagentStatus::Failed) => {
                        Some(SidecarValue::Tone("Failed".into(), theme.danger))
                    }
                    Some(zeron_doc::SubagentStatus::Stopped) => {
                        Some(SidecarValue::Tone("Stopped".into(), theme.text_muted))
                    }
                    _ => None,
                };
                let (mark, tint) =
                    crate::pickers::agent_brand_icon(agent.harness, agent.model.as_deref());
                SidecarEntry::new(key, ix, mark, agent.title.clone())
                    .suffix(suffix)
                    .tint(tint)
                    .badge(badge)
                    .value(status)
                    .action(move |this, _, cx| {
                        if agent.delegated {
                            this.open_child_chat_tab(&agent.doc_id, cx);
                            return;
                        }
                        this.add_subagent_surface(
                            agent.chat_id.clone(),
                            agent.doc_id.clone(),
                            agent.title.to_string(),
                            !running,
                            cx,
                        )
                    })
            };
            let heading = match running.len() {
                0 => "Subagents".to_string(),
                n => format!("Subagents · {n} running"),
            };
            let fold = (!previous.is_empty()).then(|| SidecarFold {
                label: format!("Previous agents ({})", previous.len()).into(),
                failed,
                open: self.sidecar_previous_agents_open,
                entries: previous
                    .into_iter()
                    .enumerate()
                    .map(|(ix, agent)| entry("previous-agent", ix, agent))
                    .collect(),
            });
            sections.push(SidecarSection {
                heading: heading.into(),
                entries: running
                    .into_iter()
                    .take(SECTION_CAP)
                    .enumerate()
                    .map(|(ix, agent)| entry("subagent", ix, agent))
                    .collect(),
                fold,
            });
        }

        if !sources.is_empty() {
            sections.push(SidecarSection {
                heading: "Sources".into(),
                entries: sources
                    .into_iter()
                    .take(SECTION_CAP)
                    .enumerate()
                    .map(|(ix, attachment)| {
                        SidecarEntry::new("source", ix, icons::ASTRA_FILE, attachment.name.clone())
                            .action(move |this, window, cx| {
                                this.transcript.update(cx, |transcript, cx| {
                                    transcript.preview_attachment(&attachment, window, cx);
                                })
                            })
                    })
                    .collect(),
                fold: None,
            });
        }

        let usage_entry = match usage {
            Some((window, fraction)) => {
                let tone = accounts::usage_color(accounts::usage_level(fraction), theme);
                SidecarEntry::new("usage", 0, icons::ASTRA_GRAPH, window)
                    .value(Some(SidecarValue::Meter(fraction, tone)))
            }
            None => SidecarEntry::new("usage", 0, icons::ASTRA_GRAPH, "Usage unavailable"),
        };
        sections.push(SidecarSection {
            heading: "Usage".into(),
            entries: vec![usage_entry],
            fold: None,
        });

        let mut project_entries = vec![SidecarEntry::new(
            "repository",
            0,
            icons::ASTRA_FOLDER,
            project.unwrap_or_else(|| "No repository".into()),
        )];
        if let Some(name) = instructions {
            let path = name.clone();
            project_entries.push(
                SidecarEntry::new("instructions", 0, icons::ASTRA_BOOK, "Project instructions")
                    .value(Some(SidecarValue::Tone(name.into(), theme.warning)))
                    .action(move |this, window, cx| {
                        this.open_right_pane(cx);
                        this.add_file_surface(path.clone(), window, cx);
                    }),
            );
        }
        for (ix, (surface, title)) in files.into_iter().take(SECTION_CAP).enumerate() {
            project_entries.push(
                SidecarEntry::new("file", ix, icons::ASTRA_FILE, title)
                    .action(move |this, _, cx| this.focus_right_surface(surface, cx)),
            );
        }
        sections.push(SidecarSection {
            heading: "Project".into(),
            entries: project_entries,
            fold: None,
        });

        sections
    }

    pub(super) fn render_sidecar(&mut self, width: f32, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let sections = self.sidecar_sections(&theme, cx);
        let collapsed = self.sidecar_collapsed_now == Some(true);
        let popover_mode = self.right_pane_shown(cx);
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
        if !popover_mode {
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
                popover_mode,
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
            .aria_label("Collapse live state")
            .absolute()
            .top_0()
            .right(px(10.0))
            .size(px(20.0))
            .flex()
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
        div()
            .absolute()
            .top(px(SIDECAR_TOP))
            .bottom_0()
            .left(px(offset))
            .w(px(SIDECAR_WIDTH))
            .pr(px(8.0))
            .opacity(opacity)
            .child(
                div()
                    .id("sidecar-scroll")
                    .size_full()
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
            .child(collapse)
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
        div()
            .id(SharedString::from(format!(
                "sidecar-{}-section-{}",
                surface.tag(),
                section.heading
            )))
            .role(gpui::Role::Group)
            .aria_label(section.heading.clone())
            .flex()
            .flex_col()
            .flex_none()
            .pb(px(4.0))
            .child(
                div()
                    .h(px(19.0))
                    .px(px(8.0))
                    .text_size(crate::typography::ui_rems(12.0))
                    .line_height(px(19.0))
                    .text_color(SidecarInk::of(theme).heading)
                    .truncate()
                    .child(section.heading.clone()),
            )
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
        let mut row = div()
            .id(SharedString::from(id))
            .debug_selector(move || selector.clone())
            .role(if actionable {
                gpui::Role::Button
            } else {
                gpui::Role::Group
            })
            .aria_label(entry.name())
            .when_some(entry.value_text(), |el, value| el.aria_value(value))
            .group(group.clone())
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(6.0))
            .h(px(28.0))
            .px(px(8.0))
            .rounded(px(8.0))
            .text_size(crate::typography::ui_rems(13.0))
            .text_color(SidecarInk::of(theme).label)
            .whitespace_nowrap()
            .child(
                div()
                    .relative()
                    .flex_none()
                    .size(px(14.0))
                    .child(
                        icon(entry.icon)
                            .size(px(14.0))
                            .text_color(entry.tint.unwrap_or(theme.text_muted))
                            .when(interactive && entry.tint.is_none(), |el| {
                                el.group_hover(group.clone(), |s| s.text_color(theme.text))
                            }),
                    )
                    .when_some(entry.badge, |el, color| {
                        el.child(
                            div()
                                .absolute()
                                .right(px(-2.0))
                                .bottom(px(-2.0))
                                .size(px(6.0))
                                .rounded_full()
                                .bg(color),
                        )
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .when_some(entry.ink, |el, ink| el.text_color(ink))
                            .child(entry.label.clone()),
                    )
                    .children(
                        entry
                            .suffix
                            .clone()
                            .map(|suffix| div().flex_none().child(suffix)),
                    ),
            )
            .children(value)
            .when_some(entry.edit.clone().filter(|_| interactive), |el, edit| {
                let id = format!("{group}-edit");
                let selector = id.clone();
                el.child(
                    div()
                        .id(SharedString::from(id))
                        .debug_selector(move || selector.clone())
                        .role(gpui::Role::Button)
                        .aria_label("Edit action")
                        .size(px(22.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(5.0))
                        .invisible()
                        .group_hover(group.clone(), |style| style.visible())
                        .hover(|style| style.bg(crate::theme::wash(0.08)))
                        .tooltip(crate::settings::widgets::text_tooltip("Edit action"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            edit(this, window, cx);
                            if surface == SidecarSurface::Popover {
                                this.set_sidecar_popover(false, cx);
                            }
                            cx.notify();
                        }))
                        .child(
                            icon(icons::SETTINGS_MINIMALISTIC)
                                .size(px(14.0))
                                .text_color(theme.text_muted),
                        ),
                )
            })
            .when(interactive, |el| {
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
                            this.set_sidecar_popover(false, cx);
                        }
                        cx.notify();
                    }));
            }
            None => row = row.cursor_default(),
        }
        row.into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn sidecar_rail(
        &self,
        theme: &Theme,
        sections: &[SidecarSection],
        opacity: f32,
        offset: f32,
        interactive: bool,
        popover_mode: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let glyph_color = if theme.appearance.is_dark() {
            gpui::white()
        } else {
            theme.text
        };
        let rail_icon =
            |key: String, glyph: &'static str, tint: Option<gpui::Hsla>, tooltip: SharedString| {
                let selector = key.clone();
                div()
                    .id(SharedString::from(key))
                    .debug_selector(move || selector.clone())
                    .role(gpui::Role::Button)
                    .aria_label(tooltip.clone())
                    .size(px(24.0))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.0))
                    .when(interactive, |el| {
                        el.cursor_pointer()
                            .hover(|s| s.bg(crate::theme::wash(0.06)))
                    })
                    .when(interactive, |el| {
                        el.tooltip(move |_, cx| {
                            cx.new(|_| SurfaceTabTooltip {
                                text: tooltip.clone(),
                            })
                            .into()
                        })
                        .tooltip_show_delay(Duration::from_millis(260))
                    })
                    .child(
                        icon(glyph)
                            .size(px(16.0))
                            .text_color(tint.unwrap_or(glyph_color)),
                    )
            };
        let mut pill = div()
            .id("sidecar-pill")
            .flex()
            .flex_col()
            .gap(px(1.0))
            .w(px(32.0))
            .p(px(4.0))
            .rounded(px(10.0))
            .max_h(px((self.viewport_height - SIDECAR_TOP - 16.0).max(64.0)))
            .overflow_y_scroll();
        for (ix, section) in sections
            .iter()
            .filter(|section| !section.entries.is_empty())
            .enumerate()
        {
            let mut group = div()
                .flex()
                .flex_col()
                .flex_none()
                .gap(px(1.0))
                .when(ix > 0, |el| {
                    el.pt(px(3.0))
                        .mt(px(2.0))
                        .border_t_1()
                        .border_color(crate::theme::hairline(0.08))
                });
            for entry in &section.entries {
                let key = format!("sidecar-rail-{}-{}", entry.key, entry.ix);
                let mut button = rail_icon(key, entry.icon, entry.tint, entry.tooltip());
                if interactive {
                    let action = entry.action.clone();
                    button = button.on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        match action.as_ref() {
                            Some(action) => action(this, window, cx),
                            None => this.set_sidecar_user_collapsed(false, cx),
                        }
                        cx.notify();
                    }));
                }
                group = group.child(button);
            }
            pill = pill.child(group);
        }
        let expand_tooltip: SharedString = if self.sidecar_popover_open() {
            "Close live state".into()
        } else {
            "Expand live state".into()
        };
        let mut expand = rail_icon(
            "sidecar-expand".into(),
            icons::ASTRA_CHEVRONS_LEFT,
            None,
            expand_tooltip,
        );
        if interactive {
            // The popover's mouse-down-out closes it on this same press, so the
            // click must know whether the press found it up (see `Popup::note_trigger_press`).
            expand = expand
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, _| {
                        this.sidecar_press_found_popover = this.sidecar_popover_mounted();
                    }),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    let found = std::mem::take(&mut this.sidecar_press_found_popover);
                    if popover_mode {
                        this.set_sidecar_popover(!found, cx);
                    } else {
                        this.set_sidecar_user_collapsed(false, cx);
                    }
                }));
        }
        pill = pill.child(expand);
        div()
            .absolute()
            .top(px(SIDECAR_TOP))
            .right(px(8.0 - offset))
            .w(px(32.0))
            .opacity(opacity)
            .child(pill)
            .into_any_element()
    }

    fn sidecar_popover_card(
        &self,
        theme: &Theme,
        sections: &[SidecarSection],
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let fade = self.sidecar_popover?;
        let opacity = self.motion_progress(fade.started, motion::SIDECAR_POPOVER_FADE);
        let slide = self.motion_progress(fade.started, motion::SIDECAR_POPOVER_SLIDE);
        let (opacity, offset) = if fade.open {
            (opacity, 10.0 * (1.0 - slide))
        } else if slide >= 1.0 {
            return None;
        } else {
            (1.0 - opacity, 10.0 * slide)
        };
        let header = div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .justify_between()
            .h(px(34.0))
            .pl(px(12.0))
            .pr(px(8.0))
            .border_b_1()
            .border_color(crate::theme::hairline(0.08))
            .text_size(crate::typography::ui_rems(12.0))
            .text_color(theme.text_muted)
            .child("Live state")
            .child(
                div()
                    .id("sidecar-popover-close")
                    .debug_selector(|| "sidecar-popover-close".into())
                    .role(gpui::Role::Button)
                    .aria_label("Close live state")
                    .group("sidecar-popover-close")
                    .size(px(24.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(7.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(crate::theme::wash(0.06)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.set_sidecar_popover(false, cx);
                    }))
                    .child(
                        icon(icons::ASTRA_X)
                            .size(px(12.0))
                            .text_color(theme.text_faint)
                            .group_hover("sidecar-popover-close", |s| s.text_color(theme.text)),
                    ),
            );
        let max_h = (self.viewport_height - SIDECAR_TOP - 16.0).max(120.0);
        let card = popover::popover_card_flush(theme)
            .w(px(POPOVER_WIDTH))
            .max_h(px(max_h))
            .flex()
            .flex_col()
            .rounded(px(12.0))
            .child(header)
            .child(
                div()
                    .id("sidecar-popover-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .pt(px(2.0))
                    .px(px(4.0))
                    .pb(px(4.0))
                    .children(sections.iter().map(|section| {
                        self.sidecar_group(theme, section, SidecarSurface::Popover, fade.open, cx)
                    })),
            );
        Some(
            div()
                .id("sidecar-popover")
                .debug_selector(|| "sidecar-popover".into())
                .absolute()
                .top(px(SIDECAR_TOP))
                .right(px(48.0 - offset))
                .w(px(POPOVER_WIDTH))
                .opacity(opacity)
                .occlude()
                .when(fade.open, |el| {
                    el.on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.set_sidecar_popover(false, cx);
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
        .text_color(theme.text_faint);
    match value {
        SidecarValue::Text(text) => base.child(text.clone()).into_any_element(),
        SidecarValue::Mono(text) => base
            .font_family(theme.font_mono.clone())
            .text_size(crate::typography::ui_rems(12.0))
            .child(text.clone())
            .into_any_element(),
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
            .text_color(theme.text_faint)
            .children(text.clone().map(|text| div().truncate().child(text)))
            .child(div().flex_none().size(px(6.0)).rounded_full().bg(*color))
            .into_any_element(),
        SidecarValue::Meter(fraction, color) => div()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .text_color(*color)
            .child(
                div()
                    .w(px(36.0))
                    .h(px(4.0))
                    .rounded(px(2.0))
                    .overflow_hidden()
                    .bg(crate::theme::hairline(0.1))
                    .child(div().h_full().w(px(36.0 * fraction)).bg(*color)),
            )
            .child(SharedString::from(format!(
                "{}%",
                (fraction * 100.0).round() as u32
            )))
            .into_any_element(),
        SidecarValue::Counts(files, add, del) => {
            let files = zeron_proto::badges::files_label(*files);
            div()
                .flex_none()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(4.0))
                .font_family(theme.font_mono.clone())
                .text_size(crate::typography::ui_rems(12.0))
                .text_color(theme.text_faint)
                .child(SharedString::from(files))
                .child(
                    div()
                        .text_color(theme.success)
                        .child(SharedString::from(format!("+{add}"))),
                )
                .child(
                    div()
                        .text_color(theme.danger)
                        .child(SharedString::from(format!("−{del}"))),
                )
                .into_any_element()
        }
    }
}

type LabeledSubagent = (crate::transcript::SubagentSummary, Option<SharedString>);

fn split_subagents(
    mut agents: Vec<LabeledSubagent>,
) -> (Vec<LabeledSubagent>, Vec<LabeledSubagent>) {
    agents.reverse();
    agents
        .into_iter()
        .partition(|(agent, _)| agent.status == Some(zeron_doc::SubagentStatus::Running))
}

fn agent_tag_suffix(
    agent: &crate::transcript::SubagentSummary,
    model_label: impl Fn(zeron_proto::HarnessId, &str) -> Option<String>,
) -> Option<SharedString> {
    let tag = match agent.model.as_deref() {
        Some(id) => agent
            .harness
            .and_then(|harness| model_label(harness, id))
            .unwrap_or_else(|| id.to_owned()),
        None => agent.kind.as_ref()?.to_string(),
    };
    if agent.title.to_lowercase().contains(&tag.to_lowercase()) {
        return None;
    }
    Some(format!(": {tag}").into())
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

fn basename(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    trimmed
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(trimmed)
        .to_string()
}

/// `scheme://host:port/path` → `host:port`.
fn url_host(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    rest.split('/').next().unwrap_or(rest).to_string()
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
        assert!(cx.debug_bounds("sidecar-panel-new-sidechat-0").is_some());
        for (row, expected, count) in [
            ("sidecar-panel-sidechat-0", "side-a", 1),
            ("sidecar-panel-sidechat-1", "side-b", 2),
            ("sidecar-panel-sidechat-0", "side-a", 2),
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
    fn sidechat_rows_carry_the_sidebar_status(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                let chat = state.chats.iter_mut().find(|c| c.id == "side-a").unwrap();
                chat.created_at = Utc::now();
                chat.last_message_at = Some(Utc::now());
            });
            let theme = Theme::of(cx).clone();
            let sections = shell.sidecar_sections(&theme, cx);
            let side = sections
                .iter()
                .find(|section| section.heading.as_ref() == "Side chats")
                .unwrap();
            let tooltips: Vec<_> = side.entries.iter().map(|e| e.tooltip()).collect();
            assert_eq!(tooltips[0].as_ref(), "Parent: Expired");
            assert_eq!(tooltips[1].as_ref(), "Parent: Done");
        });
    }

    #[gpui::test]
    fn collapse_persists_and_the_rail_chevrons_bring_the_panel_back(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        assert!(cx.debug_bounds("sidecar-rail-browser-0").is_none());
        click(cx, "sidecar-collapse");
        shell.read_with(cx, |shell, cx| {
            assert!(shell.settings.sidecar_collapsed);
            assert!(crate::settings::current(cx).sidecar_collapsed);
            assert_eq!(shell.sidecar_width_now(), SIDECAR_RAIL_WIDTH);
        });
        assert!(cx.debug_bounds("sidecar-panel-browser-0").is_none());
        assert!(cx.debug_bounds("sidecar-rail-browser-0").is_some());
        click(cx, "sidecar-expand");
        shell.read_with(cx, |shell, cx| {
            assert!(!shell.settings.sidecar_collapsed);
            assert!(!crate::settings::current(cx).sidecar_collapsed);
            assert_eq!(shell.sidecar_width_now(), SIDECAR_WIDTH);
        });
        assert!(cx.debug_bounds("sidecar-panel-browser-0").is_some());
    }

    #[gpui::test]
    fn with_the_pane_open_the_rail_opens_live_state_and_a_row_closes_it(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        shell.update(cx, |shell, cx| shell.toggle_right_pane(cx));
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(cx.debug_bounds("sidecar-rail-server-0").is_some());
        assert!(cx.debug_bounds("sidecar-popover").is_none());
        click(cx, "sidecar-expand");
        shell.read_with(cx, |shell, _| {
            assert!(shell.sidecar_popover_open());
            assert!(!shell.settings.sidecar_collapsed);
        });
        click(cx, "sidecar-popover-server-0");
        shell.read_with(cx, |shell, cx| {
            assert!(!shell.sidecar_popover_open());
            assert_eq!(shell.resolved_right_active(cx), RightSurface::Browser(1));
        });
        shell.update(cx, |shell, cx| {
            shell.set_sidecar_popover(true, cx);
            assert!(shell.capture_escape_surface(cx));
            assert!(!shell.sidecar_popover_open());
        });
        shell.update(cx, |shell, cx| shell.set_sidecar_popover(true, cx));
        cx.update(|window, cx| window.draw(cx).clear());
        click(cx, "sidecar-expand");
        shell.read_with(cx, |shell, _| {
            assert!(
                !shell.sidecar_popover_open(),
                "the chevrons close an open popover instead of reopening it"
            );
        });
        shell.update(cx, |shell, cx| {
            shell.set_sidecar_popover(true, cx);
            shell.toggle_right_pane(cx);
        });
        cx.update(|window, cx| window.draw(cx).clear());
        shell.read_with(cx, |shell, _| {
            assert!(
                !shell.sidecar_popover_open(),
                "closing the pane drops the popover"
            );
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
        assert!(cx.debug_bounds("sidecar-rail-computer-0").is_some());
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
        assert!(cx.debug_bounds("sidecar-rail-device-0").is_some());
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
        click(cx, "sidecar-expand");
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
    fn subagents_wear_their_brand_and_fold_the_finished_ones(cx: &mut TestAppContext) {
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
                .find(|section| section.heading.starts_with("Subagents"))
                .expect("subagents section");
            assert_eq!(section.heading.as_ref(), "Subagents · 2 running");
            let marks: Vec<_> = section.entries.iter().map(|e| e.icon).collect();
            assert_eq!(marks, [icons::CLAUDE_MARK, icons::OPENAI_MARK]);
            let labels: Vec<_> = section
                .entries
                .iter()
                .map(|e| format!("{}{}", e.label, e.suffix.as_deref().unwrap_or_default()))
                .collect();
            assert_eq!(labels, ["Kid: anthropic/claude-opus-5", "task a: Explore"]);
            assert!(section.entries.iter().all(|e| e.badge == Some(theme.busy)));
            let fold = section.fold.as_ref().expect("previous agents fold");
            assert_eq!(fold.label.as_ref(), "Previous agents (2)");
            assert_eq!(fold.failed, 1);
            assert_eq!(fold.entries[0].badge, Some(theme.danger));
            assert_eq!(fold.entries[1].badge, None);
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

    #[gpui::test]
    fn fork_chat_follows_new_sidechat_and_forks_from_panel_and_rail(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        cx.executor().allow_parking();
        let (shell, cx) = setup(cx);
        let calls = attach_engine(&shell, &runtime, cx);
        let new = cx.debug_bounds("sidecar-panel-new-sidechat-0").unwrap();
        let fork = cx.debug_bounds("sidecar-panel-fork-chat-0").unwrap();
        assert!(
            fork.top() >= new.bottom(),
            "Fork chat sits under New sidechat"
        );
        click(cx, "sidecar-panel-fork-chat-0");
        let forks = called(cx, &calls, zeron_rpc::methods::FORK_SIDE_CHAT, 1);
        assert_eq!(forks[0]["sourceChatId"], "parent");
        assert_eq!(forks[0]["createdByChatId"], "parent");

        // The fork's progress tab opens the right pane, which folds the sidecar to its rail.
        click(cx, "sidecar-rail-fork-chat-0");
        called(cx, &calls, zeron_rpc::methods::FORK_SIDE_CHAT, 2);
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

        let headings = shell.update(cx, |shell, cx| {
            let theme = Theme::of(cx).clone();
            shell
                .sidecar_sections(&theme, cx)
                .iter()
                .map(|section| section.heading.to_string())
                .collect::<Vec<_>>()
        });
        let side = headings.iter().position(|h| h == "Side chats").unwrap();
        assert_eq!(headings[side + 1], "Actions");
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
        click(cx, "sidecar-panel-action-0-edit");
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
        assert!(cx.debug_bounds("sidecar-rail-add-action-0").is_some());
        click(cx, "sidecar-rail-action-0");
        called(cx, &calls, zeron_rpc::methods::RUN_PROJECT_ACTION, 2);
    }
}
