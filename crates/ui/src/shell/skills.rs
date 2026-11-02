use super::*;
use crate::settings::widgets;
use zeron_engine::registry::HarnessDescriptor;
use zeron_proto::invocation::{Invocation, Skill, SkillCommand, native_skill_identity};

const GRID_GAP: f32 = 8.0;
const PAGE_PAD_X: f32 = 28.0;
const PAGE_MAX_WIDTH: f32 = 880.0;
const TWO_COLUMN_MIN: f32 = 560.0;

#[derive(Clone, Debug)]
pub(super) struct SkillEntry {
    pub name: String,
    pub path: String,
    pub description: String,
    pub harnesses: Vec<HarnessId>,
    pub commands: Vec<SkillCommand>,
}

impl SkillEntry {
    pub(super) fn on_disk(&self) -> bool {
        !native_skill_identity(&self.path)
    }

    pub(super) fn title(&self) -> String {
        zeron_proto::command_catalog::skill_display_name(&self.name)
    }
}

#[derive(Default)]
pub(super) struct SkillsPage {
    pub filter: Option<HarnessId>,
    pub harnesses: Vec<(HarnessId, String)>,
    pub entries: Option<Vec<SkillEntry>>,
    pub error: Option<String>,
    pub dialog: popover::Popup<SkillDialog>,
    load: Option<Task<()>>,
}

pub(super) struct SkillDialog {
    path: String,
    opened_at: std::time::Instant,
    text: Option<Result<String, String>>,
    tree: Option<crate::markdown::parser::BlockTree>,
    menu: popover::Popup<()>,
    confirm_uninstall: bool,
}

impl SkillsPage {
    pub(super) fn visible(&self) -> Vec<&SkillEntry> {
        self.entries
            .iter()
            .flatten()
            .filter(|entry| self.filter.is_none_or(|h| entry.harnesses.contains(&h)))
            .collect()
    }

    fn count(&self, harness: Option<HarnessId>) -> Option<usize> {
        let entries = self.entries.as_ref()?;
        Some(
            entries
                .iter()
                .filter(|entry| harness.is_none_or(|h| entry.harnesses.contains(&h)))
                .count(),
        )
    }

    fn entry(&self, path: &str) -> Option<&SkillEntry> {
        self.entries
            .iter()
            .flatten()
            .find(|entry| entry.path == path)
    }

    pub(super) fn harness_name(&self, harness: HarnessId) -> String {
        self.harnesses
            .iter()
            .find(|(id, _)| *id == harness)
            .map_or_else(
                || harness.display_name().to_string(),
                |(_, name)| name.clone(),
            )
    }
}

fn group(per_harness: Vec<(HarnessId, Vec<Skill>)>) -> Vec<SkillEntry> {
    let mut entries: Vec<SkillEntry> = Vec::new();
    for (harness, skills) in per_harness {
        for skill in skills {
            match entries.iter_mut().find(|entry| entry.path == skill.path) {
                Some(entry) => {
                    if !entry.harnesses.contains(&harness) {
                        entry.harnesses.push(harness);
                    }
                    entry.commands.extend(skill.command);
                }
                None => entries.push(SkillEntry {
                    name: skill.name,
                    path: skill.path,
                    description: skill.description,
                    harnesses: vec![harness],
                    commands: skill.command.into_iter().collect(),
                }),
            }
        }
    }
    entries.sort_by_key(|entry| entry.name.to_lowercase());
    entries
}

impl Shell {
    pub(super) fn open_skills(&mut self, cx: &mut Context<Self>) {
        if matches!(self.route, Route::Skills) {
            return;
        }
        self.enter_skills_route(cx);
        self.nav.push(NavEntry::Skills);
    }

    pub(super) fn enter_skills_route(&mut self, cx: &mut Context<Self>) {
        self.set_route(Route::Skills, cx);
        self.close_chat_menu(cx);
        self.load_skills(cx);
        cx.notify();
    }

    pub(super) fn load_skills(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.skills.load = Some(cx.spawn(async move |this, cx| {
            let client = engine.client();
            let harnesses = match client
                .call(methods::LIST_HARNESSES, serde_json::json!({}))
                .await
                .map_err(|error| error.to_string())
                .and_then(|value| {
                    serde_json::from_value::<Vec<HarnessDescriptor>>(value)
                        .map_err(|error| error.to_string())
                }) {
                Ok(list) => crate::pickers::offered_harnesses(&list),
                Err(error) => {
                    this.update(cx, |shell, cx| {
                        shell.skills.error = Some(error);
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };
            let calls = harnesses.iter().map(|descriptor| {
                let harness = descriptor.id;
                async move {
                    let skills = client
                        .call(
                            methods::LIST_SKILLS,
                            serde_json::json!({ "harness": harness }),
                        )
                        .await
                        .ok()
                        .and_then(|value| serde_json::from_value::<Option<Vec<Skill>>>(value).ok())
                        .flatten();
                    skills.map(|skills| (harness, skills))
                }
            });
            let per_harness: Vec<(HarnessId, Vec<Skill>)> = futures::future::join_all(calls)
                .await
                .into_iter()
                .flatten()
                .collect();
            let names = harnesses
                .iter()
                .filter(|d| per_harness.iter().any(|(id, _)| *id == d.id))
                .map(|d| (d.id, d.name.clone()))
                .collect();
            let entries = group(per_harness);
            this.update(cx, |shell, cx| {
                shell.skills.harnesses = names;
                shell.skills.entries = Some(entries);
                shell.skills.error = None;
                if let Some(filter) = shell.skills.filter
                    && !shell.skills.harnesses.iter().any(|(id, _)| *id == filter)
                {
                    shell.skills.filter = None;
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn set_skills_filter(&mut self, filter: Option<HarnessId>, cx: &mut Context<Self>) {
        if self.skills.filter != filter {
            self.skills.filter = filter;
            cx.notify();
        }
    }

    pub(super) fn render_skills_sidebar(
        &mut self,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let reduced = motion::reduced_motion(cx);
        let selected = self.skills.filter;
        let mut tab = |key: String,
                       target: Option<HarnessId>,
                       glyph: (&'static str, Option<gpui::Hsla>),
                       label: String,
                       count: Option<usize>| {
            let is_selected = selected == target;
            let hover_key = format!("{key}-hover");
            let t =
                widgets::tab_selection_t(window, format!("{key}-selection"), is_selected, reduced);
            let (mark, tint) = glyph;
            let ink = motion::hover_blend(
                &hover_key,
                motion::mix(theme.text_muted, theme.text, t),
                theme.text,
            );
            widgets::section_tab(theme, is_selected, t, key, hover_key)
                .role(gpui::Role::Tab)
                .aria_label(SharedString::from(label.clone()))
                .aria_selected(is_selected)
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.set_skills_filter(target, cx)))
                .child(
                    icon(mark)
                        .flex_none()
                        .size(px(16.0))
                        .text_color(tint.unwrap_or(ink)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(SharedString::from(label)),
                )
                .when_some(count, |el, count| {
                    el.child(
                        div()
                            .flex_none()
                            .text_size(crate::typography::ui_rems(12.0))
                            .text_color(theme.text_faint)
                            .child(SharedString::from(count.to_string())),
                    )
                })
                .into_any_element()
        };
        let mut rows = vec![tab(
            "skills-tab-all".into(),
            None,
            (icons::MAGIC_STICK_3, None),
            "All skills".into(),
            self.skills.count(None),
        )];
        let harnesses = self.skills.harnesses.clone();
        if !harnesses.is_empty() {
            rows.push(
                div()
                    .mt(px(Theme::SPACE_MD))
                    .px(px(Theme::SPACE_SM))
                    .py(px(4.0))
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_faint)
                    .child("Harnesses")
                    .into_any_element(),
            );
        }
        for (harness, name) in harnesses {
            rows.push(tab(
                format!("skills-tab-{}", harness.as_str()),
                Some(harness),
                crate::pickers::harness_brand_icon(harness),
                name,
                self.skills.count(Some(harness)),
            ));
        }
        let pane = div()
            .w(px(self.sidebar_content_width()))
            .h_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .h(px(36.0))
                    .px(px(Theme::SPACE_LG))
                    .flex()
                    .items_center()
                    .text_size(crate::typography::ui_rems(13.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.text)
                    .child("Skills"),
            )
            .child(
                div().flex_1().min_h_0().child(widgets::scroll_faded(
                    "skills-tabs-scroll",
                    div()
                        .id("skills-tabs")
                        .role(gpui::Role::TabList)
                        .aria_label("Skill harnesses")
                        .size_full()
                        .overflow_y_scroll()
                        .px(px(Theme::SPACE_SM))
                        .pb(px(Theme::SPACE_LG))
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .children(rows),
                )),
            );
        if reduced {
            pane.into_any_element()
        } else {
            motion::fade_quick("skills-sidebar", pane).into_any_element()
        }
    }

    pub(super) fn render_skills_page(&mut self, width: f32, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let title = match self.skills.filter {
            Some(harness) => self.skills.harness_name(harness),
            None => "All skills".to_string(),
        };
        let visible = self.skills.visible();
        let subtitle = match (&self.skills.entries, &self.skills.error) {
            (Some(_), _) => match visible.len() {
                1 => "1 skill".to_string(),
                n => format!("{n} skills"),
            },
            (None, Some(error)) => format!("Could not load skills: {error}"),
            (None, None) => "Loading skills…".to_string(),
        };
        let columns: u16 = if width.min(PAGE_MAX_WIDTH) - 2.0 * PAGE_PAD_X >= TWO_COLUMN_MIN {
            2
        } else {
            1
        };
        let cards: Vec<AnyElement> = visible
            .into_iter()
            .map(|entry| self.render_skill_card(entry, &theme, cx))
            .collect();
        let empty = cards.is_empty() && self.skills.entries.is_some();
        let filter_key = self.skills.filter.map_or("all", HarnessId::as_str);
        let grid = div()
            .grid()
            .grid_cols(columns)
            .gap(px(GRID_GAP))
            .children(cards);
        let grid = if motion::reduced_motion(cx) {
            grid.into_any_element()
        } else {
            motion::fade_quick(
                SharedString::from(format!("skills-grid-{filter_key}")),
                grid,
            )
            .into_any_element()
        };
        let column = widgets::page_column()
            .max_w(px(PAGE_MAX_WIDTH))
            .px(px(PAGE_PAD_X))
            .gap(px(20.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(widgets::page_header(&theme, &title, None))
                    .child(widgets::page_subtitle(&theme, subtitle)),
            )
            .when(empty, |el| {
                el.child(
                    div()
                        .text_size(crate::typography::ui_rems(13.0))
                        .text_color(theme.text_muted)
                        .child("No skills installed for this harness."),
                )
            })
            .child(grid);
        div()
            .id("skills-page")
            .debug_selector(|| "skills-page".into())
            .flex_1()
            .min_w_0()
            .h_full()
            .child(widgets::scroll_faded(
                "skills-scroll",
                div()
                    .id("skills-content")
                    .size_full()
                    .overflow_y_scroll()
                    .child(column),
            ))
            .into_any_element()
    }

    fn render_skill_card(
        &self,
        entry: &SkillEntry,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let hover = format!("skills-card-{}", entry.path);
        let path = entry.path.clone();
        let title = entry.title();
        div()
            .id(SharedString::from(hover.clone()))
            .role(gpui::Role::Button)
            .aria_label(SharedString::from(title.clone()))
            .min_w_0()
            .p(px(12.0))
            .rounded(px(12.0))
            .bg(motion::hover_blend(
                &hover,
                widgets::block_fill(theme),
                theme.wash(0.08),
            ))
            .cursor_pointer()
            .on_hover(motion::hover_listener(hover))
            .on_click(cx.listener(move |this, _, _, cx| this.open_skill_dialog(&path, cx)))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.0))
            .child(skill_tile(theme, 34.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        div()
                            .truncate()
                            .text_size(crate::typography::ui_rems(13.0))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child(SharedString::from(title)),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(crate::typography::ui_rems(12.0))
                            .text_color(theme.text_muted)
                            .child(SharedString::from(entry.description.clone())),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn open_skill_dialog(&mut self, path: &str, cx: &mut Context<Self>) {
        self.skills.dialog.open(SkillDialog {
            path: path.to_owned(),
            opened_at: motion::now(),
            text: None,
            tree: None,
            menu: popover::Popup::default(),
            confirm_uninstall: false,
        });
        cx.notify();
        if native_skill_identity(path) {
            return;
        }
        let file = PathBuf::from(path);
        let path = path.to_owned();
        cx.spawn(async move |this, cx| {
            let text = cx
                .background_spawn(async move { std::fs::read_to_string(file) })
                .await
                .map_err(|error| error.to_string());
            this.update(cx, |shell, cx| {
                let Some(dialog) = shell.skills.dialog.get_mut() else {
                    return;
                };
                if dialog.path != path {
                    return;
                }
                if let Ok(text) = &text {
                    let body = zeron_engine::playbooks::parse(text).1;
                    dialog.tree = Some(crate::markdown::parse_full(body.trim_start()));
                }
                dialog.text = Some(text);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn close_skill_dialog(&mut self, cx: &mut Context<Self>) {
        if self.skills.dialog.begin_close() {
            popover::reap_popup(cx, |shell: &mut Self| &mut shell.skills.dialog);
            cx.notify();
        }
    }

    pub(super) fn escape_skill_dialog(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(dialog) = self.skills.dialog.open_mut() else {
            return false;
        };
        if dialog.menu.is_open() {
            self.close_skill_menu(cx);
        } else if dialog.confirm_uninstall {
            dialog.confirm_uninstall = false;
            cx.notify();
        } else {
            self.close_skill_dialog(cx);
        }
        true
    }

    fn toggle_skill_menu(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.skills.dialog.open_mut() else {
            return;
        };
        if dialog.menu.take_press_was_open() {
            return;
        }
        if dialog.menu.is_open() {
            self.close_skill_menu(cx);
        } else {
            dialog.menu.open(());
            cx.notify();
        }
    }

    fn close_skill_menu(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.skills.dialog.get_mut() else {
            return;
        };
        if dialog.menu.begin_close() {
            popover::reap_popup(cx, |shell: &mut Self| {
                &mut shell
                    .skills
                    .dialog
                    .get_mut()
                    .expect("menu reaped inside a mounted dialog")
                    .menu
            });
            cx.notify();
        }
    }

    fn run_skill_menu(&mut self, action: MenuAction, cx: &mut Context<Self>) {
        self.close_skill_menu(cx);
        let Some(dialog) = self.skills.dialog.as_open() else {
            return;
        };
        let path = std::path::Path::new(&dialog.path);
        match action {
            MenuAction::Open => cx.open_with_system(path),
            MenuAction::Reveal => cx.reveal_path(path),
            MenuAction::Copy => {
                if let Some(Ok(text)) = &dialog.text {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.clone()));
                    self.show_notice(
                        crate::toast::ToastKind::Success,
                        "Copied SKILL.md".to_string(),
                        cx,
                    );
                }
            }
        }
    }

    fn toggle_skill_enabled(&mut self, path: String, cx: &mut Context<Self>) {
        if !self.settings.disabled_skills.remove(&path) {
            self.settings.disabled_skills.insert(path);
        }
        self.schedule_save(cx);
        cx.notify();
    }

    fn try_skill(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self
            .skills
            .dialog
            .as_open()
            .and_then(|dialog| self.skills.entry(&dialog.path))
            .cloned()
        else {
            return;
        };
        let allowed: Vec<HarnessId> = match self.skills.filter {
            Some(filter) if entry.harnesses.contains(&filter) => vec![filter],
            _ => entry.harnesses.clone(),
        };
        self.close_skill_dialog(cx);
        self.open_new_session(None, cx);
        let Some(draft) = self.state.read(cx).active_draft.clone() else {
            return;
        };
        let pickers = self.composer.read(cx).pickers().clone();
        let harness = pickers.update(cx, |pickers, cx| pickers.ensure_draft_harness(&allowed, cx));
        let command = entry
            .commands
            .iter()
            .find(|command| Some(command.harness) == harness)
            .cloned();
        let link = Invocation::Skill {
            name: entry.name.clone(),
            path: entry.path.clone(),
            command,
        }
        .link();
        self.composer
            .update(cx, |composer, cx| composer.seed_skill(&draft, &link, cx));
        self.schedule_save(cx);
    }

    fn uninstall_skill(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self
            .skills
            .dialog
            .as_open()
            .map(|dialog| dialog.path.clone())
        else {
            return;
        };
        match move_skill_to_trash(std::path::Path::new(&path)) {
            Ok(()) => {
                self.settings.disabled_skills.remove(&path);
                self.schedule_save(cx);
                if let Some(entries) = self.skills.entries.as_mut() {
                    entries.retain(|entry| entry.path != path);
                }
                self.close_skill_dialog(cx);
                self.load_skills(cx);
            }
            Err(error) => self.show_notice(
                crate::toast::ToastKind::Error,
                format!("Could not move the skill to the Trash: {error}"),
                cx,
            ),
        }
    }

    pub(super) fn render_skill_dialog(
        &mut self,
        viewport: gpui::Size<Pixels>,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let closing = self.skills.dialog.closing_since();
        let reduced = motion::reduced_motion(cx);
        if closing.is_some() && reduced {
            return None;
        }
        let dialog = self.skills.dialog.get()?;
        let entry = self.skills.entry(&dialog.path)?.clone();
        let shown = match closing {
            Some(since) => 1.0 - motion::MENU_OUT.progress_since(since),
            None if reduced => 1.0,
            None => motion::DIALOG_IN.progress_since(dialog.opened_at),
        };
        let on_disk = entry.on_disk();
        let enabled = !self.settings.disabled_skills.contains(&entry.path);
        let menu_open = dialog.menu.is_open();
        let menu_closing = dialog.menu.closing_since();
        let menu_mounted = dialog.menu.get().is_some();
        let confirm = dialog.confirm_uninstall;
        let body_max = (f32::from(viewport.height) * 0.42).clamp(160.0, 380.0);

        let icon_button = |id: &'static str, glyph: &'static str, label: &'static str| {
            div()
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(label)
                .size(px(28.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .cursor_pointer()
                .hover(|s| s.bg(theme.glass_hover()))
                .child(icon(glyph).size(px(16.0)).text_color(theme.text_muted))
        };
        let toggle_path = entry.path.clone();
        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .child(skill_tile(theme, 36.0))
            .child(div().flex_1())
            .child(
                div()
                    .id("skill-dialog-toggle")
                    .flex_none()
                    .tab_index(0)
                    .role(gpui::Role::Switch)
                    .aria_label("Use this skill")
                    .aria_toggled(if enabled {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .mr(px(4.0))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_skill_enabled(toggle_path.clone(), cx)
                    }))
                    .child(widgets::toggle_switch(
                        theme,
                        enabled,
                        "skill-dialog-toggle",
                    )),
            )
            .when(on_disk, |el| {
                el.child(
                    icon_button("skill-dialog-more", icons::LUCIDE_ELLIPSIS, "More options")
                        .aria_expanded(menu_open)
                        .when(menu_open, |el| el.bg(theme.glass_hover()))
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(|this, _, _, _| {
                                if let Some(dialog) = this.skills.dialog.get_mut() {
                                    dialog.menu.note_trigger_press();
                                }
                            }),
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_skill_menu(cx))),
                )
            })
            .child(
                icon_button("skill-dialog-close", icons::LUCIDE_X, "Close")
                    .on_click(cx.listener(|this, _, _, cx| this.close_skill_dialog(cx))),
            );

        let badges = entry.harnesses.iter().map(|harness| {
            let (mark, tint) = crate::pickers::harness_brand_icon(*harness);
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(5.0))
                .h(px(22.0))
                .px(px(8.0))
                .rounded(px(6.0))
                .bg(theme.wash(0.06))
                .text_size(crate::typography::ui_rems(12.0))
                .text_color(theme.text_muted)
                .child(
                    icon(mark)
                        .size(px(12.0))
                        .text_color(tint.unwrap_or(theme.text_muted)),
                )
                .child(SharedString::from(self.skills.harness_name(*harness)))
        });

        let body: AnyElement = match (&dialog.text, &dialog.tree) {
            (_, Some(tree)) => {
                let options = crate::markdown::render::RenderOptions::settled(SharedString::from(
                    format!("skill-doc-{}", entry.path),
                ));
                crate::markdown::render::render_tree(tree, &options, theme, window, &|_| None)
            }
            (Some(Err(error)), _) => div()
                .text_color(theme.text_muted)
                .child(SharedString::from(format!(
                    "Could not read SKILL.md: {error}"
                )))
                .into_any_element(),
            (None, None) if on_disk => div()
                .text_color(theme.text_muted)
                .child("Loading SKILL.md…")
                .into_any_element(),
            _ => div()
                .text_color(theme.text_muted)
                .child("This skill comes from the harness and has no SKILL.md on disk.")
                .into_any_element(),
        };

        let footer = if confirm {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(crate::typography::ui_rems(12.5))
                        .text_color(theme.text_muted)
                        .child(
                            "Moves the skill to the Trash. Every harness that loads it loses it.",
                        ),
                )
                .child(
                    popover::btn_ghost(theme, "Cancel", "skill-uninstall-cancel")
                        .id("skill-uninstall-cancel")
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(dialog) = this.skills.dialog.open_mut() {
                                dialog.confirm_uninstall = false;
                                cx.notify();
                            }
                        })),
                )
                .child(
                    popover::btn_danger(theme, "Move to Trash")
                        .id("skill-uninstall-confirm")
                        .on_click(cx.listener(|this, _, _, cx| this.uninstall_skill(cx))),
                )
        } else {
            div()
                .flex()
                .flex_row()
                .items_center()
                .when(on_disk, |el| {
                    el.child(
                        div()
                            .id("skill-uninstall")
                            .role(gpui::Role::Button)
                            .px(px(4.0))
                            .py(px(6.0))
                            .text_size(crate::typography::ui_rems(13.0))
                            .text_color(theme.danger)
                            .cursor_pointer()
                            .hover(|s| s.opacity(0.8))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(dialog) = this.skills.dialog.open_mut() {
                                    dialog.confirm_uninstall = true;
                                    cx.notify();
                                }
                            }))
                            .child("Uninstall"),
                    )
                })
                .child(div().flex_1())
                .child(
                    popover::btn_primary(theme, "Try now")
                        .id("skill-try-now")
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .rounded_full()
                        .px(px(14.0))
                        .on_click(cx.listener(|this, _, _, cx| this.try_skill(cx)))
                        .child(
                            icon(icons::MAGIC_STICK_3)
                                .size(px(14.0))
                                .text_color(theme.on_solid),
                        ),
                )
        };

        let card = popover::dialog_card(theme)
            .w(px(580.0))
            .max_w(viewport.width - px(32.0))
            .px(px(24.0))
            .pt(px(22.0))
            .pb(px(20.0))
            .child(header)
            .child(
                div()
                    .mt(px(16.0))
                    .flex()
                    .flex_row()
                    .items_baseline()
                    .gap(px(8.0))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(crate::typography::ui_rems(20.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child(SharedString::from(entry.title())),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(crate::typography::ui_rems(13.0))
                            .text_color(theme.text_faint)
                            .child("Skill"),
                    ),
            )
            .when(!entry.description.is_empty(), |el| {
                el.child(
                    div()
                        .mt(px(6.0))
                        .text_size(crate::typography::ui_rems(13.0))
                        .line_height(px(19.0))
                        .text_color(theme.text_muted)
                        .line_clamp(3)
                        .child(SharedString::from(entry.description.clone())),
                )
            })
            .child(
                div()
                    .mt(px(12.0))
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap(px(6.0))
                    .children(badges)
                    .when(on_disk, |el| {
                        el.child(
                            div()
                                .min_w_0()
                                .truncate()
                                .ml(px(4.0))
                                .text_size(crate::typography::ui_rems(12.0))
                                .text_color(theme.text_faint)
                                .child(SharedString::from(tilde_path(&entry.path))),
                        )
                    }),
            )
            .child(
                div()
                    .id("skill-doc")
                    .mt(px(16.0))
                    .max_h(px(body_max))
                    .overflow_y_scroll()
                    .px(px(16.0))
                    .py(px(14.0))
                    .rounded(px(12.0))
                    .bg(widgets::block_fill(theme))
                    .text_size(crate::typography::ui_rems(13.0))
                    .child(body),
            )
            .child(div().mt(px(18.0)).child(footer));

        let menu = menu_mounted.then(|| self.render_skill_menu(theme, menu_closing, cx));
        let card = div()
            .id("skill-dialog")
            .role(gpui::Role::Dialog)
            .aria_label(SharedString::from(entry.title()))
            .relative()
            .child(card)
            .children(menu);
        Some(command_palette::palette_overlay(
            viewport,
            card,
            command_palette::PaletteMotion {
                shown,
                leaving: closing.is_some(),
            },
            true,
        ))
    }

    fn render_skill_menu(
        &self,
        theme: &Theme,
        closing: Option<std::time::Instant>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row =
            |id: &'static str, glyph: &'static str, label: &'static str, action: MenuAction| {
                popover::menu_row(theme, false, id)
                    .id(id)
                    .role(gpui::Role::MenuItem)
                    .pr(px(24.0))
                    .child(icon(glyph).size(px(15.0)).text_color(theme.text_muted))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.run_skill_menu(action, cx);
                    }))
            };
        let card = popover::popover_card(theme)
            .flex()
            .flex_col()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_skill_menu(cx)))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(row(
                "skill-menu-open",
                icons::LUCIDE_EXTERNAL_LINK,
                "Open",
                MenuAction::Open,
            ))
            .child(row(
                "skill-menu-reveal",
                icons::FOLDER,
                "Reveal in Finder",
                MenuAction::Reveal,
            ))
            .child(row(
                "skill-menu-copy",
                icons::COPY,
                "Copy Markdown",
                MenuAction::Copy,
            ));
        let blur = match closing {
            Some(since) => crate::frost::MENU_BLUR * (1.0 - motion::MENU_OUT.progress_since(since)),
            None => crate::frost::MENU_BLUR,
        };
        let card = crate::frost::frosted(12.0, blur, card);
        let layer = div().absolute().top(px(56.0)).right(px(52.0));
        match closing {
            Some(since) => motion::menu_out_toward(
                "skill-menu-out",
                -4.0,
                motion::MENU_OUT.progress_since(since),
                layer.child(card),
            )
            .into_any_element(),
            None => {
                motion::menu_in_from("skill-menu-in", -4.0, layer.child(card)).into_any_element()
            }
        }
    }
}

pub(super) fn skill_tile(theme: &Theme, size: f32) -> gpui::Div {
    div()
        .flex_none()
        .size(px(size))
        .rounded(px(size * 0.3))
        .bg(theme.wash(0.06))
        .border_1()
        .border_color(theme.hairline(0.06))
        .flex()
        .items_center()
        .justify_center()
        .child(
            icon(icons::MAGIC_STICK_3)
                .size(px((size * 0.47).round()))
                .text_color(theme.branch_ink()),
        )
}

#[derive(Clone, Copy)]
enum MenuAction {
    Open,
    Reveal,
    Copy,
}

fn tilde_path(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_string(),
    }
}

/// `rename` moves a symlink itself, never what it points at.
fn move_skill_to_trash(path: &std::path::Path) -> std::io::Result<()> {
    let target = match path.parent() {
        Some(dir) if path.file_name().is_some_and(|name| name == "SKILL.md") => dir,
        _ => path,
    };
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| std::io::Error::other("HOME is not set"))?;
    let trash = home.join(".Trash");
    let name = target
        .file_name()
        .ok_or_else(|| std::io::Error::other("the skill path has no name"))?;
    let mut dest = trash.join(name);
    let mut n = 2;
    while dest.symlink_metadata().is_ok() {
        dest = trash.join(format!("{} {n}", name.to_string_lossy()));
        n += 1;
    }
    std::fs::rename(target, dest)
}
