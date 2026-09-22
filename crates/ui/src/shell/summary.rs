//! The pinned summary: a titlebar card, left of the right-pane toggle, that
//! answers "where is this session and what did it produce" — the checkout
//! (working-tree changes, local servers, branch), the sidechats, the
//! subagents, the images the user attached, and the project
//! (instructions file, open files). Each row focuses the thing it names;
//! sections fold, and the folds persist. The card itself is a transient
//! popover.

use super::*;
use crate::files::client::{FilesRequestContext, WorkspaceFilesClient};

const CARD_WIDTH: f32 = 300.0;
/// Rows shown per section before the tail folds into "+N more".
const SECTION_CAP: usize = 8;
/// Instructions files probed at the checkout root, first hit wins.
const INSTRUCTION_FILES: &[&str] = &["AGENTS.md", "CLAUDE.md"];

const SECTION_ENVIRONMENT: &str = "environment";
const SECTION_SIDECHATS: &str = "sidechats";
const SECTION_SUBAGENTS: &str = "subagents";
const SECTION_SOURCES: &str = "sources";
const SECTION_PROJECT: &str = "project";

impl Shell {
    pub(super) fn render_summary_trigger(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let open = self.summary_menu.get().is_some();
        let fade_key = "header-icon-pinned-summary";
        let mut trigger = div()
            .id("pinned-summary")
            .debug_selector(|| "pinned-summary".into())
            .size(px(28.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .bg(motion::hover_blend(
                fade_key,
                crate::theme::wash(0.0),
                crate::theme::wash(0.11),
            ))
            .on_hover(motion::hover_listener(fade_key))
            .tooltip(|_, cx| {
                cx.new(|_| SurfaceTabTooltip {
                    text: "Toggle pinned summary".into(),
                })
                .into()
            })
            .tooltip_show_delay(Duration::from_millis(350))
            // Sits inside the titlebar drag region like [`header_icon_button`].
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, _| {
                    window.prevent_default();
                    this.summary_menu.note_trigger_press();
                }),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                if this.summary_menu.take_press_was_open() {
                    this.close_summary_menu(cx);
                } else {
                    this.open_summary_menu(cx);
                }
            }))
            .child(icon(icons::CHECKLIST).size(px(16.0)).text_color(if open {
                theme.text
            } else {
                theme.text_muted
            }));
        if open {
            let closing = self.summary_menu.closing_since();
            let card = self.render_summary_card(&theme, cx);
            trigger = trigger.relative().child(popover::anchored_menu_below_end(
                "pinned-summary-menu",
                card,
                closing,
            ));
        }
        trigger.into_any_element()
    }

    pub(super) fn open_summary_menu(&mut self, cx: &mut Context<Self>) {
        self.summary_menu.open(());
        if let Some(chat_id) = self.state.read(cx).selected_chat.clone() {
            self.probe_summary_instructions(chat_id, cx);
        }
        cx.notify();
    }

    pub(super) fn close_summary_menu(&mut self, cx: &mut Context<Self>) {
        if self.summary_menu.begin_close() {
            popover::reap_popup(cx, |shell: &mut Self| &mut shell.summary_menu);
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

    /// The Changes row: focus the working-tree diff tab, else the first diff
    /// tab, else open one.
    fn focus_changes(&mut self, cx: &mut Context<Self>) {
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
                if !self.right_pane_open(cx) {
                    self.toggle_right_pane(cx);
                }
                self.add_diff_surface(cx);
            }
        }
    }

    fn summary_section_collapsed(&self, key: &str) -> bool {
        self.settings.summary_collapsed.iter().any(|k| k == key)
    }

    fn toggle_summary_section(&mut self, key: &'static str, cx: &mut Context<Self>) {
        let list = &mut self.settings.summary_collapsed;
        match list.iter().position(|k| k == key) {
            Some(ix) => {
                list.remove(ix);
            }
            None => list.push(key.into()),
        }
        self.schedule_save(cx);
        cx.notify();
    }

    /// Look for an instructions file at the chat's checkout root. One probe
    /// per chat at a time; the cached answer stays on screen until the fresh
    /// one lands.
    fn probe_summary_instructions(&mut self, chat_id: String, cx: &mut Context<Self>) {
        if self.summary_instruction_probes.contains_key(&chat_id) {
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

    fn render_summary_card(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let muted = theme.text_muted;
        let mut card = popover::popover_card(theme)
            .id("pinned-summary-scroll")
            .max_h(px(480.0))
            .overflow_y_scroll()
            .w(px(CARD_WIDTH))
            .flex()
            .flex_col()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_summary_menu(cx)));

        // The surfaces are keyed by panel, not by chat row, so they still list
        // when the row has not synced in yet; only the chat-derived rows wait.
        let chat = self.state.read(cx).selected_chat_row().cloned();

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
        let device = chat.as_ref().and_then(|chat| {
            self.state
                .read(cx)
                .device_name(&chat.device_id)
                .map(str::to_string)
        });
        // Folders need not be repos: no git, no Changes or branch rows.
        let git = chat.as_ref().is_some_and(|chat| {
            self.state
                .read(cx)
                .space_for_chat(chat)
                .is_some_and(|space| space.git_detected)
        });
        let counts = self.summary_change_counts(cx);
        let usage = chat.as_ref().and_then(|chat| self.summary_usage(chat));

        let row = |theme: &Theme, key: &'static str, ix: usize, icon_path: &'static str| {
            popover::menu_row(theme, false, format!("{key}-{ix}"))
                .id((key, ix))
                .debug_selector(|| format!("{key}-{ix}"))
                .child(icon(icon_path).size(px(13.0)).text_color(muted))
        };
        let label = |text: SharedString| div().flex_1().min_w_0().truncate().child(text);
        let value = |theme: &Theme, text: String| {
            div()
                .flex_none()
                .text_size(crate::typography::ui_rems(11.0))
                .text_color(theme.text_muted)
                .child(SharedString::from(text))
        };
        let more = |theme: &Theme, hidden: usize| {
            div()
                .px(px(8.0))
                .py(px(4.0))
                .text_size(crate::typography::ui_rems(11.0))
                .text_color(theme.text_muted)
                .child(SharedString::from(format!("+{hidden} more")))
        };

        // ---- Environment ----
        let env_open = !self.summary_section_collapsed(SECTION_ENVIRONMENT);
        card = card.child(self.summary_heading(
            theme,
            SECTION_ENVIRONMENT,
            "Environment",
            device,
            env_open,
            cx,
        ));
        if env_open {
            let changes_value = counts.map(|(files, add, del)| {
                let files = if files == 1 {
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
                    .text_size(crate::typography::ui_rems(11.0))
                    .font_family(theme.font_mono.clone())
                    .text_color(theme.text_muted)
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
            });
            card = card.when(git, |card| {
                card.child(
                    row(theme, "pinned-summary-changes", 0, icons::LIST)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.focus_changes(cx);
                            this.close_summary_menu(cx);
                        }))
                        .child(label("Changes".into()))
                        .children(changes_value),
                )
            });

            let servers_open = self.summary_servers_open && !browsers.is_empty();
            card = card.child(
                row(theme, "pinned-summary-servers", 0, icons::GLOBE)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.summary_servers_open = !this.summary_servers_open;
                        cx.notify();
                    }))
                    .child(label("Local Servers".into()))
                    .child(value(theme, browsers.len().to_string()))
                    .child(
                        icon(if servers_open {
                            icons::ALT_ARROW_DOWN
                        } else {
                            icons::ALT_ARROW_RIGHT
                        })
                        .size(px(12.0))
                        .text_color(theme.text_muted.opacity(0.6)),
                    ),
            );
            if servers_open {
                let hidden = browsers.len().saturating_sub(SECTION_CAP);
                for (ix, (surface, title, url)) in
                    browsers.into_iter().take(SECTION_CAP).enumerate()
                {
                    let host = url.as_deref().map(url_host).unwrap_or_default();
                    card = card.child(
                        row(theme, "pinned-summary-server", ix, icons::GLOBE)
                            .pl(px(31.0))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.focus_right_surface(surface, cx);
                                this.close_summary_menu(cx);
                            }))
                            .child(label(title))
                            .when(!host.is_empty(), |row| row.child(value(theme, host))),
                    );
                }
                card = card.when(hidden > 0, |card| card.child(more(theme, hidden)));
            }

            if let Some(branch) = branch.filter(|_| git) {
                card = card.child(
                    popover::menu_row(theme, false, "pinned-summary-branch")
                        .id("pinned-summary-branch")
                        .cursor_default()
                        .child(icon(icons::GIT_BRANCH).size(px(13.0)).text_color(muted))
                        .child(label(branch.into()))
                        .children(
                            head.map(|sha| value(theme, sha).font_family(theme.font_mono.clone())),
                        ),
                );
            }
        }

        // ---- Sidechats ----
        if source.is_some() {
            let sidechats_open = !self.summary_section_collapsed(SECTION_SIDECHATS);
            card = card
                .child(popover::menu_separator())
                .child(self.summary_heading(
                    theme,
                    SECTION_SIDECHATS,
                    "Sidechats",
                    None,
                    sidechats_open,
                    cx,
                ));
            if sidechats_open {
                for (ix, chat) in sidechats.into_iter().enumerate() {
                    let expired = chat.sidechat_expired(Utc::now());
                    let title = chat.title.clone().unwrap_or_else(|| "Sidechat".into());
                    card = card.child(
                        row(theme, "pinned-summary-sidechat", ix, icons::CHAT_ROUND_LINE)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.add_sidechat_surface(&chat, cx);
                                this.close_summary_menu(cx);
                            }))
                            .child(label(title.into()))
                            .when(expired, |row| row.child(value(theme, "Expired".into()))),
                    );
                }
                card = card.child(
                    row(theme, "pinned-summary-new-sidechat", 0, icons::PLUS)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(source) = source.clone() {
                                this.create_sidechat(source, None, cx);
                            }
                            this.close_summary_menu(cx);
                        }))
                        .child(label("New sidechat".into())),
                );
            }
        }

        // ---- Subagents ----
        if !subagents.is_empty() {
            let subagents_open = !self.summary_section_collapsed(SECTION_SUBAGENTS);
            card = card
                .child(popover::menu_separator())
                .child(self.summary_heading(
                    theme,
                    SECTION_SUBAGENTS,
                    "Subagents",
                    None,
                    subagents_open,
                    cx,
                ));
            if subagents_open {
                let hidden = subagents.len().saturating_sub(SECTION_CAP);
                for (ix, agent) in subagents.into_iter().take(SECTION_CAP).enumerate() {
                    let (state, tone) = match agent.status {
                        Some(zeron_doc::SubagentStatus::Running) => ("working", theme.warning),
                        Some(zeron_doc::SubagentStatus::Failed) => ("failed", theme.danger),
                        Some(zeron_doc::SubagentStatus::Done) | None => ("done", muted),
                    };
                    let frozen = !matches!(agent.status, Some(zeron_doc::SubagentStatus::Running));
                    let title = agent.title.clone();
                    card = card.child(
                        row(theme, "pinned-summary-subagent", ix, icons::BOT)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.add_subagent_surface(
                                    agent.chat_id.clone(),
                                    agent.doc_id.clone(),
                                    agent.title.to_string(),
                                    frozen,
                                    cx,
                                );
                                this.close_summary_menu(cx);
                            }))
                            .child(label(title))
                            .child(value(theme, state.into()).text_color(tone)),
                    );
                }
                card = card.when(hidden > 0, |card| card.child(more(theme, hidden)));
            }
        }

        // ---- Sources ----
        if !sources.is_empty() {
            let sources_open = !self.summary_section_collapsed(SECTION_SOURCES);
            card = card
                .child(popover::menu_separator())
                .child(self.summary_heading(
                    theme,
                    SECTION_SOURCES,
                    "Sources",
                    None,
                    sources_open,
                    cx,
                ));
            if sources_open {
                let hidden = sources.len().saturating_sub(SECTION_CAP);
                for (ix, attachment) in sources.into_iter().take(SECTION_CAP).enumerate() {
                    let name = SharedString::from(attachment.name.clone());
                    card = card.child(
                        row(theme, "pinned-summary-source", ix, icons::FILE_IMAGE)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.transcript.update(cx, |transcript, cx| {
                                    transcript.preview_attachment(&attachment, window, cx);
                                });
                                this.close_summary_menu(cx);
                            }))
                            .child(label(name)),
                    );
                }
                card = card.when(hidden > 0, |card| card.child(more(theme, hidden)));
            }
        }

        // ---- Project ----
        if instructions.is_some() || !files.is_empty() {
            let project_open = !self.summary_section_collapsed(SECTION_PROJECT);
            card = card
                .child(popover::menu_separator())
                .child(self.summary_heading(
                    theme,
                    SECTION_PROJECT,
                    "Project",
                    None,
                    project_open,
                    cx,
                ));
            if project_open {
                if let Some(name) = instructions {
                    let path = name.clone();
                    card = card.child(
                        row(theme, "pinned-summary-instructions", 0, icons::DOCUMENT)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if !this.right_pane_open(cx) {
                                    this.toggle_right_pane(cx);
                                }
                                this.add_file_surface(path.clone(), window, cx);
                                this.close_summary_menu(cx);
                            }))
                            .child(label("Project instructions".into()))
                            .child(value(theme, name).font_family(theme.font_mono.clone())),
                    );
                }
                let hidden = files.len().saturating_sub(SECTION_CAP);
                for (ix, (surface, title)) in files.into_iter().take(SECTION_CAP).enumerate() {
                    card = card.child(
                        row(theme, "pinned-summary-file", ix, icons::DOCUMENT)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.focus_right_surface(surface, cx);
                                this.close_summary_menu(cx);
                            }))
                            .child(label(title)),
                    );
                }
                card = card.when(hidden > 0, |card| card.child(more(theme, hidden)));
            }
        }

        // ---- footer: View all · usage ----
        card = card.child(popover::menu_separator()).child(
            popover::menu_row(theme, false, "pinned-summary-view-all")
                .id("pinned-summary-view-all")
                .on_click(cx.listener(|this, _, _, cx| {
                    if !this.right_pane_open(cx) {
                        this.toggle_right_pane(cx);
                    }
                    this.close_summary_menu(cx);
                }))
                .child(
                    icon(icons::SIDEBAR_MINIMALISTIC)
                        .size(px(13.0))
                        .text_color(muted),
                )
                .child(label("View all".into()))
                .children(usage.map(|(window, fraction)| {
                    let level = accounts::usage_level(fraction);
                    let pct_color = match level {
                        accounts::UsageLevel::Normal => theme.text_muted,
                        _ => accounts::usage_color(level, theme),
                    };
                    div()
                        .flex_none()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .text_size(crate::typography::ui_rems(11.0))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(window))
                        .child(
                            div()
                                .text_color(pct_color)
                                .child(SharedString::from(format!(
                                    "{}%",
                                    (fraction * 100.0).round() as u32
                                ))),
                        )
                        .child(accounts::usage_bar(fraction, theme).w(px(34.0)).h(px(3.0)))
                })),
        );
        card.into_any_element()
    }

    /// A foldable section heading: the label, an optional muted suffix, and
    /// the chevron. Clicking anywhere on it toggles the fold.
    fn summary_heading(
        &self,
        theme: &Theme,
        key: &'static str,
        label: &str,
        suffix: Option<String>,
        open: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(SharedString::from(format!("pinned-summary-section-{key}")))
            .debug_selector(|| format!("pinned-summary-section-{key}"))
            .flex()
            .flex_row()
            .items_center()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_summary_section(key, cx);
            }))
            .child(popover::menu_heading(theme, label).flex_none())
            .children(suffix.map(|suffix| {
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(crate::typography::ui_rems(10.0))
                    .text_color(theme.text_muted.opacity(0.6))
                    .child(SharedString::from(format!("· {suffix}")))
            }))
            .child(div().flex_1())
            .child(
                div().pr(px(8.0)).child(
                    icon(if open {
                        icons::ALT_ARROW_DOWN
                    } else {
                        icons::ALT_ARROW_RIGHT
                    })
                    .size(px(12.0))
                    .text_color(theme.text_muted.opacity(0.6)),
                ),
            )
    }
}

/// `scheme://host:port/path` → `host:port`.
fn url_host(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    rest.split('/').next().unwrap_or(rest).to_string()
}

#[cfg(test)]
mod summary_fold_regressions {
    use super::*;
    use gpui::{AppContext, TestAppContext, VisualTestContext};

    struct TriggerHost {
        shell: Entity<Shell>,
        _data_dir: tempfile::TempDir,
    }

    impl Render for TriggerHost {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.shell.update(cx, |shell, cx| {
                let trigger = shell.render_summary_trigger(cx);
                shell.titlebar_drag_region(
                    "pinned-summary-fold-titlebar",
                    div().w(px(400.)).h(px(40.)).child(trigger),
                    cx,
                )
            })
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
        let (host, cx) = cx.add_window_view(|_, cx| {
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
                shell.active_chat = "parent".into();
                shell.state.update(cx, |state, _| {
                    let source: zeron_proto::Chat = serde_json::from_value(serde_json::json!({
                        "id": "parent", "deviceId": "dev", "title": "Parent",
                        "archived": false, "createdAt": "2026-09-18T00:00:00Z"
                    }))
                    .unwrap();
                    state.chats.push(source.clone());
                    let mut side = source;
                    side.id = "side-a".into();
                    side.sidechat_source_chat_id = Some("parent".into());
                    state.chats.push(side);
                    state.selected_chat = Some("parent".into());
                });
                shell
            });
            TriggerHost {
                shell,
                _data_dir: dir,
            }
        });
        let shell = host.read_with(cx, |host, _| host.shell.clone());
        cx.update(|window, cx| window.draw(cx).clear());
        (shell, cx)
    }

    #[gpui::test]
    fn folding_sidechats_hides_its_rows_and_reaches_the_settings_store(cx: &mut TestAppContext) {
        let (shell, cx) = setup(cx);
        shell.update(cx, |shell, cx| shell.open_summary_menu(cx));
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(cx.debug_bounds("pinned-summary-sidechat-0").is_some());
        let heading = cx
            .debug_bounds("pinned-summary-section-sidechats")
            .expect("Sidechats heading")
            .center();
        cx.simulate_click(heading, gpui::Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear());
        shell.read_with(cx, |shell, cx| {
            assert!(shell.summary_menu.is_open(), "folding keeps the card open");
            assert_eq!(
                shell.settings.summary_collapsed,
                vec!["sidechats".to_string()]
            );
            assert_eq!(
                crate::settings::current(cx).summary_collapsed,
                vec!["sidechats".to_string()]
            );
        });
        assert!(cx.debug_bounds("pinned-summary-sidechat-0").is_none());
        assert!(cx.debug_bounds("pinned-summary-new-sidechat-0").is_none());
        cx.simulate_click(heading, gpui::Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(cx.debug_bounds("pinned-summary-sidechat-0").is_some());
        shell.read_with(cx, |shell, cx| {
            assert!(shell.settings.summary_collapsed.is_empty());
            assert!(crate::settings::current(cx).summary_collapsed.is_empty());
        });
    }
}
