use super::*;
use crate::settings::widgets;
use zeron_engine::playbooks::{self, Playbook, PlaybookSource};

impl Shell {
    fn local_project_roots(&self, cx: &App) -> Vec<(String, PathBuf)> {
        let state = self.state.read(cx);
        let Some(local) = state.local_device_id.as_deref() else {
            return Vec::new();
        };
        state
            .spaces
            .iter()
            .filter(|space| space.device_id == local)
            .map(|space| (space.id.clone(), PathBuf::from(&space.path)))
            .collect()
    }

    pub(super) fn playbooks(&self, cx: &App) -> Vec<Playbook> {
        let roots: Vec<PathBuf> = self
            .local_project_roots(cx)
            .into_iter()
            .map(|(_, root)| root)
            .collect();
        playbooks::list(&self.data_dir, &roots)
    }

    pub(super) fn start_playbook(&mut self, playbook: &Playbook, cx: &mut Context<Self>) {
        let local = self.state.read(cx).local_device_id.clone();
        let (path, target) = match &playbook.source {
            PlaybookSource::BuiltIn => {
                let prepared = match zeron_engine::triage::prepare(&self.data_dir) {
                    Ok(prepared) => prepared,
                    Err(error) => {
                        self.show_notice(
                            crate::toast::ToastKind::Error,
                            format!("Could not start triage: {error}. Run zeron triage in a terminal instead."),
                            cx,
                        );
                        return;
                    }
                };
                zeron_engine::triage::spawn_app_background(
                    self.data_dir.clone(),
                    &prepared,
                    option_env!("ZERON_GIT_HASH"),
                );
                (prepared.playbook, local.map(NewSessionTarget::NoProject))
            }
            PlaybookSource::App => (playbook.path.clone().unwrap_or_default(), None),
            PlaybookSource::Project { root } => {
                let space = self
                    .local_project_roots(cx)
                    .into_iter()
                    .find(|(_, path)| path == root)
                    .map(|(id, _)| NewSessionTarget::Project(id));
                (playbook.path.clone().unwrap_or_default(), space)
            }
        };
        self.open_new_session(target, cx);
        let Some(draft) = self.state.read(cx).active_draft.clone() else {
            return;
        };
        let hero = (
            SharedString::from(playbook.question.clone()),
            playbook.subtitle.clone().map(SharedString::from),
        );
        let path = path.to_string_lossy().into_owned();
        self.composer.update(cx, |composer, cx| {
            composer.seed_playbook(&draft, &path, hero, cx)
        });
        self.schedule_save(cx);
    }

    fn new_playbook(&mut self, cx: &mut Context<Self>) {
        match playbooks::create(&playbooks::app_dir(&self.data_dir)) {
            Ok(path) => {
                cx.open_with_system(&path);
                cx.notify();
            }
            Err(error) => self.show_notice(
                crate::toast::ToastKind::Error,
                format!("Could not create a playbook: {error}"),
                cx,
            ),
        }
    }

    fn delete_playbook(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.playbook_delete_armed.as_ref() != Some(&path) {
            self.playbook_delete_armed = Some(path);
            cx.notify();
            return;
        }
        self.playbook_delete_armed = None;
        if let Err(error) = std::fs::remove_file(&path) {
            self.show_notice(
                crate::toast::ToastKind::Error,
                format!("Could not delete {}: {error}", path.display()),
                cx,
            );
        }
        cx.notify();
    }

    pub(super) fn render_new_playbook_button(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        widgets::action_button(theme, widgets::ActionTone::Solid)
            .id("new-playbook")
            .debug_selector(|| "new-playbook".into())
            .role(gpui::Role::Button)
            .aria_label("New playbook")
            .tab_index(0)
            .flex_none()
            .on_click(cx.listener(|this, _, _, cx| this.new_playbook(cx)))
            .child(
                icon(icons::ASTRA_PLUS)
                    .size(px(14.0))
                    .text_color(theme.on_solid),
            )
            .child("New playbook")
            .into_any_element()
    }

    pub(super) fn render_playbooks_page(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).for_settings_surface();
        let all = self.playbooks(cx);
        let projects: Vec<(PathBuf, String)> = {
            let state = self.state.read(cx);
            self.local_project_roots(cx)
                .into_iter()
                .filter_map(|(id, root)| {
                    Some((root, state.space_row(&id)?.display_name().to_owned()))
                })
                .collect()
        };
        let card = all.iter().enumerate().fold(
            widgets::section_card(&theme).mt_0(),
            |card, (ix, playbook)| {
                card.child(self.render_playbook_row(&theme, ix, playbook, &projects, cx))
            },
        );
        let folder = playbooks::app_dir(&self.data_dir);
        let note = format!(
            "Playbooks are markdown files with title and question in their frontmatter. Files in {} are offered everywhere; a project's own go in .zeron/playbooks.",
            folder.display()
        );
        let reveal = widgets::text_action(&theme, widgets::ActionTone::Filled, "Reveal folder")
            .id("playbooks-reveal-folder")
            .debug_selector(|| "playbooks-reveal-folder".into())
            .role(gpui::Role::Button)
            .tab_index(0)
            .flex_none()
            .on_click(cx.listener(move |this, _, _, cx| {
                let folder = playbooks::app_dir(&this.data_dir);
                let _ = std::fs::create_dir_all(&folder);
                cx.reveal_path(&folder);
            }));
        let column = widgets::settings_page().child(card).child(
            div()
                .mt(px(12.0))
                .flex()
                .flex_row()
                .items_start()
                .gap(px(12.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(widgets::page_note(&theme, note)),
                )
                .child(reveal),
        );
        div()
            .id("playbooks-page-host")
            .relative()
            .size_full()
            .child(
                crate::edge_fade::edge_faded(
                    16.0,
                    true,
                    true,
                    div()
                        .id("playbooks-page")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.playbooks_scroll.scroll)
                        .child(column),
                )
                .fade_overflow_y(&self.playbooks_scroll.scroll),
            )
            .into_any_element()
    }

    fn render_playbook_row(
        &self,
        theme: &Theme,
        ix: usize,
        playbook: &Playbook,
        projects: &[(PathBuf, String)],
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let source = match &playbook.source {
            PlaybookSource::Project { root } => projects
                .iter()
                .find(|(path, _)| path == root)
                .map(|(_, name)| name.clone())
                .unwrap_or_else(|| root.display().to_string()),
            source => source.label().to_string(),
        };
        let mut meta = vec![div().child(source).into_any_element()];
        if let Some(path) = &playbook.path {
            meta.push(
                div()
                    .truncate()
                    .child(SharedString::from(
                        path.file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                    ))
                    .into_any_element(),
            );
        }
        let start = {
            let playbook = playbook.clone();
            widgets::text_action(theme, widgets::ActionTone::Filled, "Start")
                .id(("playbook-start", ix))
                .debug_selector(move || format!("playbook-start-{ix}"))
                .role(gpui::Role::Button)
                .aria_label(SharedString::from(format!("Start {}", playbook.title)))
                .tab_index(0)
                .on_click(cx.listener(move |this, _, _, cx| this.start_playbook(&playbook, cx)))
        };
        let title = playbook.title.clone();
        let file_actions = playbook.path.clone().map(|path| {
            let armed = self.playbook_delete_armed.as_ref() == Some(&path);
            let edit_path = path.clone();
            div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .child(
                    widgets::text_action(theme, widgets::ActionTone::Quiet, "Edit")
                        .id(("playbook-edit", ix))
                        .role(gpui::Role::Button)
                        .aria_label(SharedString::from(format!("Edit {title}")))
                        .tab_index(0)
                        .on_click(move |_, _, cx| cx.open_with_system(&edit_path)),
                )
                .child(
                    widgets::text_action(
                        theme,
                        if armed {
                            widgets::ActionTone::Danger
                        } else {
                            widgets::ActionTone::Quiet
                        },
                        if armed { "Confirm delete" } else { "Delete" },
                    )
                    .id(("playbook-delete", ix))
                    .role(gpui::Role::Button)
                    .aria_label(SharedString::from(format!("Delete {title}")))
                    .tab_index(0)
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.delete_playbook(path.clone(), cx)),
                    ),
                )
        });
        widgets::card_row(theme, ix == 0)
            .child(widgets::row_tile(
                theme,
                if playbook.source == PlaybookSource::BuiltIn {
                    icons::ACTION_DEBUG
                } else {
                    icons::LUCIDE_BOOK_OPEN
                },
            ))
            .child(
                div()
                    .flex_1()
                    .min_w(px(200.0))
                    .child(widgets::row_title(theme, playbook.title.clone()))
                    .child(widgets::meta_line(theme, meta)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .children(file_actions)
                    .child(start),
            )
    }
}
