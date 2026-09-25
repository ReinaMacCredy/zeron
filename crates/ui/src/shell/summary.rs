//! Astra's sidecar (paseo-fork `Sidecar.tsx` + `sidecar.css`) beside the transcript.

use super::*;
use crate::files::client::{FilesRequestContext, WorkspaceFilesClient};
use std::rc::Rc;

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
    Counts(usize, u32, u32),
}

struct SidecarEntry {
    key: &'static str,
    ix: usize,
    icon: &'static str,
    label: SharedString,
    value: Option<SidecarValue>,
    action: Option<SidecarAction>,
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
            label: label.into(),
            value: None,
            action: None,
        }
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

    fn tooltip(&self) -> SharedString {
        match &self.value {
            Some(SidecarValue::Text(v) | SidecarValue::Mono(v) | SidecarValue::Tone(v, _)) => {
                format!("{}: {v}", self.label).into()
            }
            _ => self.label.clone(),
        }
    }
}

struct SidecarSection {
    heading: SharedString,
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
        let popover_mode = self.right_pane_open(cx);
        let collapsed = self.settings.sidecar_collapsed
            || popover_mode
            || full_column <= SIDECAR_AUTO_COLLAPSE_WIDTH;
        match self.sidecar_collapsed_now {
            Some(now) if now != collapsed => {
                let from = self.sidecar_width_now();
                self.sidecar_collapsed_now = Some(collapsed);
                self.sidecar_flip = Some(SidecarFlip {
                    from,
                    started: self.render_time.unwrap_or_else(std::time::Instant::now),
                });
            }
            Some(_) => {}
            None => self.sidecar_collapsed_now = Some(collapsed),
        }
        if !(popover_mode && collapsed) && self.sidecar_popover_open() {
            self.sidecar_popover = Some(PopoverFade {
                open: false,
                started: std::time::Instant::now(),
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
                || fade.started.elapsed()
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
            started: std::time::Instant::now(),
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
    fn focus_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
    fn summary_usage(&self, chat: &zeron_proto::Chat) -> Option<(String, f32)> {
        let harness = chat.config.as_ref()?.harness;
        let account = self
            .account_usage
            .as_ref()?
            .accounts
            .iter()
            .find(|account| account.active && account.harness == harness)?;
        account
            .usage_windows
            .iter()
            .max_by(|a, b| a.used_fraction.total_cmp(&b.used_fraction))
            .map(|window| (window.label.clone(), window.used_fraction.clamp(0.0, 1.0)))
    }

    fn sidecar_sections(&mut self, theme: &Theme, cx: &mut Context<Self>) -> Vec<SidecarSection> {
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
        let subagents = self.transcript.read(cx).subagent_summaries();
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
        let usage = chat.as_ref().and_then(|chat| self.summary_usage(chat));

        let mut sections = Vec::new();

        let mut on = Vec::new();
        if git {
            on.push(
                SidecarEntry::new("changes", 0, icons::ASTRA_PLUS_MINUS, "Changes")
                    .value(counts.map(|(files, add, del)| SidecarValue::Counts(files, add, del)))
                    .action(|this, window, cx| this.focus_changes(window, cx)),
            );
        }
        on.push(
            SidecarEntry::new("browser", 0, icons::ASTRA_GLOBE, "Browser")
                .action(|this, window, cx| this.focus_browser(window, cx)),
        );
        on.push(
            SidecarEntry::new("terminal", 0, icons::ASTRA_TERMINAL, "Terminal")
                .action(|this, _, cx| this.focus_terminal(cx)),
        );
        on.push(
            SidecarEntry::new("files", 0, icons::ASTRA_FILE, "Files")
                .action(|this, window, cx| this.add_files_surface(window, cx)),
        );
        sections.push(SidecarSection {
            heading: format!("On {}", project.as_deref().unwrap_or("this session")).into(),
            entries: on,
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
                .value(head.map(|sha| SidecarValue::Mono(sha.into()))),
            );
        }
        sections.push(SidecarSection {
            heading: "Workspace".into(),
            entries: workspace,
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
            });
        }

        if let Some(source) = source {
            let mut entries: Vec<SidecarEntry> = sidechats
                .into_iter()
                .take(SECTION_CAP)
                .enumerate()
                .map(|(ix, chat)| {
                    let expired = chat.sidechat_expired(Utc::now());
                    let title = chat.title.clone().unwrap_or_else(|| "Sidechat".into());
                    SidecarEntry::new("sidechat", ix, icons::ASTRA_CHAT_BUBBLE, title)
                        .value(expired.then(|| SidecarValue::Text("Expired".into())))
                        .action(move |this, _, cx| {
                            this.add_sidechat_surface(&chat, cx);
                        })
                })
                .collect();
            entries.push(
                SidecarEntry::new("new-sidechat", 0, icons::ASTRA_PLUS, "New sidechat").action(
                    move |this, _, cx| {
                        this.create_sidechat(source.clone(), None, Default::default(), cx)
                    },
                ),
            );
            sections.push(SidecarSection {
                heading: "Side chats".into(),
                entries,
            });
        }

        if !subagents.is_empty() {
            sections.push(SidecarSection {
                heading: "Subagents".into(),
                entries: subagents
                    .into_iter()
                    .take(SECTION_CAP)
                    .enumerate()
                    .map(|(ix, agent)| {
                        let (status, tone) = match agent.status {
                            Some(zeron_doc::SubagentStatus::Running) => ("working", theme.warning),
                            Some(zeron_doc::SubagentStatus::Failed) => ("failed", theme.danger),
                            Some(zeron_doc::SubagentStatus::Done) | None => {
                                ("done", theme.text_faint)
                            }
                        };
                        let frozen =
                            !matches!(agent.status, Some(zeron_doc::SubagentStatus::Running));
                        SidecarEntry::new("subagent", ix, icons::ASTRA_AGENT, agent.title.clone())
                            .value(Some(SidecarValue::Tone(status.into(), tone)))
                            .action(move |this, _, cx| {
                                this.add_subagent_surface(
                                    agent.chat_id.clone(),
                                    agent.doc_id.clone(),
                                    agent.title.to_string(),
                                    frozen,
                                    cx,
                                )
                            })
                    })
                    .collect(),
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
            });
        }

        let usage_entry = match usage {
            Some((window, fraction)) => {
                let level = accounts::usage_level(fraction);
                let tone = match level {
                    accounts::UsageLevel::Normal => theme.text_faint,
                    _ => accounts::usage_color(level, theme),
                };
                SidecarEntry::new("usage", 0, icons::ASTRA_GRAPH, window).value(Some(
                    SidecarValue::Tone(
                        format!("{}%", (fraction * 100.0).round() as u32).into(),
                        tone,
                    ),
                ))
            }
            None => SidecarEntry::new("usage", 0, icons::ASTRA_GRAPH, "Usage unavailable"),
        };
        sections.push(SidecarSection {
            heading: "Usage".into(),
            entries: vec![usage_entry],
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
                    .value(Some(SidecarValue::Mono(name.into())))
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
        });

        sections
    }

    pub(super) fn render_sidecar(&mut self, width: f32, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let sections = self.sidecar_sections(&theme, cx);
        let collapsed = self.sidecar_collapsed_now == Some(true);
        let popover_mode = self.right_pane_open(cx);
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
        root.into_any_element()
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
                    .text_color(theme.text_faint)
                    .truncate()
                    .child(section.heading.clone()),
            )
            .children(
                section
                    .entries
                    .iter()
                    .map(|entry| self.sidecar_row(theme, entry, surface, interactive, cx)),
            )
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
        let mut row = div()
            .id(SharedString::from(id))
            .debug_selector(move || selector.clone())
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
            .text_color(theme.text_muted)
            .whitespace_nowrap()
            .child(
                icon(entry.icon)
                    .size(px(14.0))
                    .text_color(theme.text_muted)
                    .when(interactive, |el| {
                        el.group_hover(group.clone(), |s| s.text_color(theme.text))
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(entry.label.clone()),
            )
            .children(value)
            .when(interactive, |el| {
                el.hover(|s| s.bg(crate::theme::wash(0.06)).text_color(theme.text))
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
        let rail_icon = |key: String, glyph: &'static str, tooltip: SharedString| {
            let group = SharedString::from(key.clone());
            let selector = key.clone();
            div()
                .id(SharedString::from(key))
                .debug_selector(move || selector.clone())
                .group(group.clone())
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
                        .size(px(14.0))
                        .text_color(theme.text_muted)
                        .when(interactive, |el| {
                            el.group_hover(group, |s| s.text_color(theme.text))
                        }),
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
            .bg(theme.surface_raised.opacity(0.84))
            .border_1()
            .border_color(crate::theme::hairline(0.08))
            .max_h(px((self.viewport_height - SIDECAR_TOP - 16.0).max(64.0)))
            .overflow_y_scroll();
        for (ix, section) in sections.iter().enumerate() {
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
                let mut button = rail_icon(key, entry.icon, entry.tooltip());
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
        SidecarValue::Counts(files, add, del) => {
            let files = if *files == 1 {
                "1 file".to_string()
            } else {
                format!("{files} files")
            };
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
    fn a_narrow_column_or_a_split_changes_the_shape(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        shell.update(cx, |shell, cx| {
            assert_eq!(shell.sidecar_layout(900.0, false, cx), SIDECAR_WIDTH);
            assert_eq!(shell.sidecar_layout(640.0, false, cx), SIDECAR_RAIL_WIDTH);
            assert_eq!(shell.sidecar_layout(900.0, true, cx), 0.0);
            assert_eq!(shell.sidecar_layout(20.0, false, cx), 20.0);
        });
    }
}
