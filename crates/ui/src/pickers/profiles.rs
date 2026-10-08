use super::*;

use zeron_proto::{AgentProfile, AgentProfileDraft};

use crate::agent_profiles::{ProfileWrite, brand_badge, mode_label, summary};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ProfilePick {
    harness: HarnessId,
    model: Option<String>,
    reasoning: Option<ReasoningLevel>,
    permission_mode: Option<PermissionMode>,
    model_options: serde_json::Map<String, serde_json::Value>,
    instructions: String,
}

impl ProfilePick {
    fn of(profile: &AgentProfile) -> Self {
        Self {
            harness: profile.harness,
            model: profile.model.clone(),
            reasoning: profile.reasoning,
            permission_mode: profile.permission_mode,
            model_options: profile.model_options.clone(),
            instructions: profile.instructions.clone(),
        }
    }

    pub(super) fn write(&self, config: &mut ChatConfig) {
        if config.harness != self.harness {
            config.harness = self.harness;
            config.model = None;
            config.model_options.clear();
        }
        if let Some(model) = &self.model {
            config.model = Some(model.clone());
        }
        if let Some(level) = self.reasoning {
            config.reasoning = Some(level);
        }
        if let Some(mode) = self.permission_mode {
            config.permission_mode = mode;
        }
        if !self.model_options.is_empty() {
            config.model_options = self.model_options.clone();
        }
        config.instructions = self.instructions.clone();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EditField {
    Harness,
    Model,
    Mode,
    Effort,
}

pub(super) struct ProfileEdit {
    profile_id: Option<String>,
    draft: AgentProfileDraft,
    name: Entity<ComposerInput>,
    notes: Entity<ComposerInput>,
    pub(super) choosing: Option<EditField>,
    saving: bool,
    error: Option<SharedString>,
    _events: [Subscription; 2],
}

fn filter_profiles(profiles: &[AgentProfile], query: &str) -> Vec<AgentProfile> {
    let query = query.trim().to_lowercase();
    profiles
        .iter()
        .filter(|profile| {
            query.is_empty()
                || profile.name.to_lowercase().contains(&query)
                || profile.notes.to_lowercase().contains(&query)
        })
        .cloned()
        .collect()
}

impl Pickers {
    pub(super) fn ensure_profiles(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.engine(cx) else {
            return;
        };
        let target = self.space_target(cx).map(serde_json::Value::String);
        let generation = self.target_generation;
        if matches!(self.profiles, Loadable::Idle) {
            self.profiles = Loadable::Loading;
        }
        self.profiles_task = Some(cx.spawn(async move |this, cx| {
            let result = crate::agent_profiles::list(&engine, target).await;
            this.update(cx, |pickers, cx| {
                if pickers.target_generation != generation {
                    return;
                }
                pickers.profiles = match result {
                    Ok(profiles) => Loadable::Ready(profiles),
                    Err(error) => Loadable::Error(error),
                };
                cx.notify();
            })
            .ok();
        }));
    }

    pub(super) fn visible_profiles(&self, cx: &App) -> Vec<AgentProfile> {
        let query = self.search.read(cx).text().to_string();
        self.profiles
            .ready()
            .map(|profiles| filter_profiles(profiles, &query))
            .unwrap_or_default()
    }

    pub(super) fn profile_rows_len(&self, cx: &App) -> usize {
        self.visible_profiles(cx).len() + 1
    }

    pub(super) fn activate_profile_index(&mut self, ix: usize, cx: &mut Context<Self>) {
        let profiles = self.visible_profiles(cx);
        match profiles.get(ix) {
            Some(profile) => self.apply_profile(profile, cx),
            None => self.save_current_as_profile(cx),
        }
    }

    fn profile_model_label(&self, harness: HarnessId, model: &str) -> String {
        self.model_label(harness, model)
            .unwrap_or_else(|| model.to_string())
    }

    fn profile_summary(&self, profile: &AgentProfile) -> String {
        let model = profile
            .model
            .as_deref()
            .map(|model| self.profile_model_label(profile.harness, model));
        summary(
            profile,
            harness_display_name(profile.harness),
            model.as_deref(),
        )
    }

    pub(super) fn apply_profile(&mut self, profile: &AgentProfile, cx: &mut Context<Self>) {
        let pick = ProfilePick::of(profile);
        self.ensure_models(pick.harness, false, cx);
        if self.chat_id(cx).is_some() {
            let current_harness = self.effective_harness(cx);
            let current_model = self
                .selected_model(cx)
                .map(|m| m.id.clone())
                .or_else(|| self.effective_model_id(cx).map(str::to_owned));
            let harness_changes = current_harness != Some(pick.harness);
            let model_changes = pick
                .model
                .as_ref()
                .is_some_and(|model| current_model.as_ref() != Some(model));
            if self.harness_is_committed(cx)
                && self.chat_has_history(cx)
                && (harness_changes || model_changes)
            {
                let from = if harness_changes {
                    current_harness
                        .map(harness_display_name)
                        .unwrap_or_default()
                        .to_string()
                } else {
                    current_model
                        .as_deref()
                        .map(|model| self.profile_model_label(pick.harness, model))
                        .unwrap_or_default()
                };
                let to = pick
                    .model
                    .as_deref()
                    .map(|model| self.profile_model_label(pick.harness, model))
                    .unwrap_or_else(|| harness_display_name(pick.harness).to_string());
                self.pending_model_switch = Some(PendingModelSwitch {
                    model_id: pick.model.clone().unwrap_or_default(),
                    from,
                    to,
                    harness: harness_changes.then_some(pick.harness),
                    profile: Some(pick),
                });
                cx.notify();
                return;
            }
            self.update_chat_config(cx, None, move |config| pick.write(config));
        } else {
            if self.effective_harness(cx) != Some(pick.harness) {
                self.pick_harness(pick.harness, cx);
            }
            if let Some(model) = &pick.model {
                self.config.model = Some(model.clone());
                self.config.reasoning = None;
                let label = self.profile_model_label(pick.harness, model);
                self.defaults
                    .remember_model(pick.harness, model.clone(), label);
            }
            if let Some(level) = pick.reasoning {
                self.config.reasoning = Some(level);
                let model = pick
                    .model
                    .clone()
                    .or_else(|| self.effective_model_id(cx).map(str::to_owned));
                self.defaults
                    .remember_reasoning(pick.harness, model.as_deref(), level);
            }
            if let Some(mode) = pick.permission_mode {
                self.config.permission_mode = mode;
            }
            self.config.instructions = pick.instructions.clone();
            if !pick.model_options.is_empty()
                && let Some(model) = pick
                    .model
                    .clone()
                    .or_else(|| self.effective_model_id(cx).map(str::to_owned))
            {
                *self.defaults.model_options_mut(pick.harness, &model) = pick.model_options;
            }
            self.save_defaults();
        }
        self.animate_close(cx);
        cx.notify();
    }

    fn current_draft(&self, cx: &App) -> AgentProfileDraft {
        let resolved = self.resolved(cx);
        let mut draft = crate::agent_profiles::blank_draft(
            resolved
                .harness
                .or_else(|| self.effective_harness(cx))
                .unwrap_or(HarnessId::ClaudeCode),
        );
        draft.model = resolved.model;
        draft.reasoning = resolved.reasoning;
        draft.permission_mode = Some(resolved.permission_mode);
        draft.model_options = resolved.model_options;
        draft.instructions = resolved.instructions;
        draft
    }

    pub(super) fn save_current_as_profile(&mut self, cx: &mut Context<Self>) {
        let draft = self.current_draft(cx);
        self.open_profile_edit(None, draft, cx);
    }

    pub(super) fn open_profile_edit(
        &mut self,
        profile_id: Option<String>,
        draft: AgentProfileDraft,
        cx: &mut Context<Self>,
    ) {
        let name = cx.new(|cx| {
            ComposerInput::with_context("Profile name", "PaletteSearch", cx).with_single_line()
        });
        name.update(cx, |input, cx| input.set_text(draft.name.clone(), cx));
        let notes = cx.new(|cx| ComposerInput::new("When to use this profile", cx));
        notes.update(cx, |input, cx| input.set_text(draft.notes.clone(), cx));
        let name_events = cx.subscribe(&name, |this: &mut Self, _, event, cx| {
            if matches!(event, ComposerInputEvent::Submitted) {
                this.save_profile_edit(cx);
            }
        });
        let notes_events = cx.subscribe(&notes, |_: &mut Self, _, _: &ComposerInputEvent, cx| {
            cx.notify()
        });
        self.ensure_models(draft.harness, false, cx);
        self.setting_menu = None;
        self.profile_focus_pending = true;
        self.profile_edit = Some(ProfileEdit {
            profile_id,
            draft,
            name,
            notes,
            choosing: None,
            saving: false,
            error: None,
            _events: [name_events, notes_events],
        });
        cx.notify();
    }

    pub(super) fn close_profile_edit(&mut self, cx: &mut Context<Self>) {
        if self.profile_edit.take().is_some() {
            self.profile_focus_pending = false;
            self.focus_on_mount = true;
            cx.notify();
        }
    }

    fn edit_profile_draft(
        &mut self,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut AgentProfileDraft),
    ) {
        if let Some(edit) = self.profile_edit.as_mut() {
            change(&mut edit.draft);
            edit.choosing = None;
            edit.error = None;
        }
        cx.notify();
    }

    fn save_profile_edit(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.engine(cx) else {
            return;
        };
        let target = self.space_target(cx);
        let Some(edit) = self.profile_edit.as_mut() else {
            return;
        };
        if edit.saving {
            return;
        }
        let mut draft = edit.draft.clone();
        draft.name = edit.name.read(cx).text().trim().to_string();
        draft.notes = edit.notes.read(cx).text().trim().to_string();
        if draft.name.is_empty() {
            edit.error = Some("Give the profile a name.".into());
            cx.notify();
            return;
        }
        edit.saving = true;
        let write = ProfileWrite::Upsert {
            profile_id: edit.profile_id.clone(),
            draft,
        };
        self.profiles_task = Some(cx.spawn(async move |this, cx| {
            let result = match target {
                Some(device) => engine
                    .client()
                    .call_as::<zeron_proto::AgentProfilesSnapshot>(
                        methods::UPSERT_AGENT_PROFILE,
                        match write {
                            ProfileWrite::Upsert { profile_id, draft } => serde_json::json!({
                                "profileId": profile_id,
                                "profile": draft,
                                "targetDeviceId": device,
                            }),
                            _ => unreachable!("quick edit only upserts"),
                        },
                    )
                    .await
                    .map(|snapshot| snapshot.profiles)
                    .map_err(|err| err.to_string()),
                None => crate::agent_profiles::write(&engine, write).await,
            };
            this.update(cx, |pickers, cx| {
                match result {
                    Ok(profiles) => {
                        pickers.profiles = Loadable::Ready(profiles);
                        pickers.close_profile_edit(cx);
                        crate::agent_profiles::bump_agent_profiles(cx);
                    }
                    Err(error) => {
                        if let Some(edit) = pickers.profile_edit.as_mut() {
                            edit.saving = false;
                            edit.error = Some(error.into());
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn open_profile_in_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.profile_edit.as_ref() else {
            return;
        };
        let mut draft = edit.draft.clone();
        draft.name = edit.name.read(cx).text().trim().to_string();
        draft.notes = edit.notes.read(cx).text().trim().to_string();
        cx.set_global(crate::agent_profiles::PendingProfileEdit(Some((
            edit.profile_id.clone(),
            draft,
        ))));
        self.profile_edit = None;
        self.animate_close(cx);
        window.dispatch_action(Box::new(crate::agent_profiles::OpenAgentProfileEditor), cx);
    }

    pub(super) fn render_profile_list(
        &mut self,
        max_height: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).for_popup();
        let profiles = self.visible_profiles(cx);
        let switching = self
            .harness_is_committed(cx)
            .then(|| self.effective_harness(cx))
            .flatten();
        let mut list = div()
            .id("profile-menu-scroll")
            .w_full()
            .max_h(px(max_height))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .px(px(popover::CARD_INSET));
        if let Loadable::Error(error) = &self.profiles {
            list = list.child(empty_list_note(
                &theme,
                &format!("Profiles unavailable: {error}"),
            ));
        } else if profiles.is_empty() && !self.search.read(cx).text().trim().is_empty() {
            list = list.child(empty_list_note(&theme, "No profiles found"));
        }
        for (ix, profile) in profiles.iter().enumerate() {
            let is_active = ix == self.active;
            let line = self.profile_summary(profile);
            let switches = switching.is_some_and(|harness| harness != profile.harness);
            let edit_id = profile.id.clone();
            let edit_draft = crate::agent_profiles::draft_of(profile);
            let row_hover = SharedString::from(format!("profile-row-{ix}"));
            list = list.child(
                div()
                    .id(("profile-row", ix))
                    .group(row_hover.clone())
                    .px(px(8.0))
                    .py(px(5.0))
                    .rounded(px(popover::MENU_ITEM_RADIUS))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .cursor_pointer()
                    .when(is_active, |el| el.bg(crate::theme::ink(0.05)))
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered && this.active != ix {
                            this.active = ix;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.activate_profile_index(ix, cx);
                    }))
                    .child(brand_badge(profile.harness, 24.0, theme.text))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(1.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.0))
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(crate::typography::ui_rems(12.5))
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .text_color(theme.text)
                                            .child(SharedString::from(profile.name.clone())),
                                    )
                                    .when(profile.by_agent, |el| {
                                        el.child(
                                            div()
                                                .flex_none()
                                                .px(px(5.0))
                                                .rounded(px(4.0))
                                                .bg(crate::theme::ink(0.08))
                                                .text_size(crate::typography::ui_rems(10.0))
                                                .text_color(theme.text_muted)
                                                .child("by agent"),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(crate::typography::ui_rems(11.0))
                                    .text_color(theme.text_muted)
                                    .child(SharedString::from(line)),
                            ),
                    )
                    .when(switches, |el| {
                        el.child(
                            div()
                                .flex_none()
                                .text_size(crate::typography::ui_rems(11.0))
                                .text_color(theme.text_muted)
                                .child("switch"),
                        )
                    })
                    .child(
                        div()
                            .id(("profile-quick-edit", ix))
                            .flex_none()
                            .size(px(24.0))
                            .rounded(px(6.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .invisible()
                            .group_hover(row_hover, |s| s.visible())
                            .hover(|s| s.bg(crate::theme::ink(0.08)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.open_profile_edit(
                                    Some(edit_id.clone()),
                                    edit_draft.clone(),
                                    cx,
                                );
                            }))
                            .child(
                                crate::icons::icon(crate::icons::PEN)
                                    .size(px(13.0))
                                    .text_color(theme.text_muted),
                            ),
                    ),
            );
        }
        let save_ix = profiles.len();
        list = list
            .when(!profiles.is_empty(), |el| {
                el.child(
                    div()
                        .my(px(2.0))
                        .h(px(1.0))
                        .bg(crate::theme::hairline(0.08)),
                )
            })
            .child(
                div()
                    .id("profile-save-current")
                    .px(px(8.0))
                    .py(px(5.0))
                    .rounded(px(popover::MENU_ITEM_RADIUS))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .cursor_pointer()
                    .when(self.active == save_ix, |el| el.bg(crate::theme::ink(0.05)))
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered && this.active != save_ix {
                            this.active = save_ix;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(|this, _, _, cx| this.save_current_as_profile(cx)))
                    .child(
                        div()
                            .flex_none()
                            .size(px(24.0))
                            .rounded(px(7.0))
                            .bg(crate::theme::ink(0.06))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                crate::icons::icon(crate::icons::PLUS)
                                    .size(px(13.0))
                                    .text_color(theme.text_muted),
                            ),
                    )
                    .child(
                        div()
                            .text_size(crate::typography::ui_rems(12.5))
                            .text_color(theme.text_muted)
                            .child("Save current as profile"),
                    ),
            );
        list.into_any_element()
    }

    fn edit_choices(
        &self,
        field: EditField,
        draft: &AgentProfileDraft,
    ) -> Vec<(SharedString, bool)> {
        match field {
            EditField::Harness => self
                .harnesses
                .ready()
                .map(|list| self.offered(list))
                .unwrap_or_default()
                .iter()
                .map(|d| (SharedString::from(d.name.clone()), d.id == draft.harness))
                .collect(),
            EditField::Model => std::iter::once(("Keep current".into(), draft.model.is_none()))
                .chain(self.edit_models(draft).iter().map(|m| {
                    (
                        SharedString::from(m.label.clone()),
                        draft.model.as_deref() == Some(m.id.as_str()),
                    )
                }))
                .collect(),
            EditField::Mode => {
                std::iter::once(("Keep current".into(), draft.permission_mode.is_none()))
                    .chain(PERMISSION_MODES.iter().map(|mode| {
                        (
                            SharedString::from(mode_label(*mode)),
                            draft.permission_mode == Some(*mode),
                        )
                    }))
                    .collect()
            }
            EditField::Effort => {
                std::iter::once(("Keep current".into(), draft.reasoning.is_none()))
                    .chain(self.edit_levels(draft).iter().map(|level| {
                        (
                            SharedString::from(reasoning_label(*level)),
                            draft.reasoning == Some(*level),
                        )
                    }))
                    .collect()
            }
        }
    }

    fn edit_models(&self, draft: &AgentProfileDraft) -> Vec<Model> {
        self.models
            .get(&draft.harness)
            .and_then(|l| l.ready())
            .cloned()
            .unwrap_or_default()
    }

    fn edit_levels(&self, draft: &AgentProfileDraft) -> Vec<ReasoningLevel> {
        let models = self.edit_models(draft);
        let levels = draft
            .model
            .as_deref()
            .and_then(|id| models.iter().find(|m| m.id == id))
            .map(|m| m.reasoning_levels.clone())
            .unwrap_or_default();
        if !levels.is_empty() {
            return levels;
        }
        self.harnesses
            .ready()
            .and_then(|list| list.iter().find(|d| d.id == draft.harness))
            .map(|d| d.reasoning_levels.clone())
            .unwrap_or_default()
    }

    fn choose(&mut self, field: EditField, ix: usize, cx: &mut Context<Self>) {
        let Some(draft) = self.profile_edit.as_ref().map(|edit| edit.draft.clone()) else {
            return;
        };
        match field {
            EditField::Harness => {
                let Some(harness) = self
                    .harnesses
                    .ready()
                    .map(|list| self.offered(list))
                    .and_then(|list| list.get(ix).map(|d| d.id))
                else {
                    return;
                };
                self.ensure_models(harness, false, cx);
                self.edit_profile_draft(cx, |draft| {
                    if draft.harness != harness {
                        draft.harness = harness;
                        draft.model = None;
                        draft.model_options.clear();
                    }
                });
            }
            EditField::Model => {
                let model = ix
                    .checked_sub(1)
                    .and_then(|ix| self.edit_models(&draft).get(ix).map(|m| m.id.clone()));
                self.edit_profile_draft(cx, |draft| {
                    draft.model = model;
                    draft.model_options.clear();
                });
            }
            EditField::Mode => {
                let mode = ix
                    .checked_sub(1)
                    .and_then(|ix| PERMISSION_MODES.get(ix).copied());
                self.edit_profile_draft(cx, |draft| draft.permission_mode = mode);
            }
            EditField::Effort => {
                let level = ix
                    .checked_sub(1)
                    .and_then(|ix| self.edit_levels(&draft).get(ix).copied());
                self.edit_profile_draft(cx, |draft| draft.reasoning = level);
            }
        }
    }

    pub(super) fn render_profile_edit(
        &mut self,
        height: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let theme = Theme::of(cx).for_popup();
        let edit = self.profile_edit.as_ref()?;
        if self.profile_focus_pending {
            let handle = edit.name.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
            self.profile_focus_pending = false;
        }
        let edit = self.profile_edit.as_ref()?;
        let draft = edit.draft.clone();
        let title = if edit.profile_id.is_some() {
            format!("Edit {}", draft.name)
        } else {
            "New profile".to_string()
        };
        let (brand, tint) = harness_brand_icon(draft.harness);
        let back = div()
            .id("profile-edit-back")
            .flex_none()
            .size(px(28.0))
            .rounded(px(popover::MENU_ITEM_RADIUS))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|s| s.bg(crate::theme::ink(0.05)))
            .on_click(cx.listener(|this, _, _, cx| {
                let choosing = this.profile_edit.as_ref().and_then(|edit| edit.choosing);
                match choosing {
                    Some(_) => {
                        if let Some(edit) = this.profile_edit.as_mut() {
                            edit.choosing = None;
                        }
                        cx.notify();
                    }
                    None => this.close_profile_edit(cx),
                }
            }))
            .child(
                crate::icons::icon(crate::icons::ALT_ARROW_LEFT)
                    .size(px(14.0))
                    .text_color(theme.text_muted),
            );
        let header = div()
            .flex_none()
            .h(px(40.0))
            .px(px(popover::CARD_INSET))
            .border_b_1()
            .border_color(crate::theme::hairline(0.08))
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(back)
            .child(brand_badge(draft.harness, 20.0, theme.text))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(crate::typography::ui_rems(13.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(SharedString::from(title)),
            );

        if let Some(field) = edit.choosing {
            let choices = self.edit_choices(field, &draft);
            let mut list = div()
                .id("profile-edit-choices")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p(px(popover::CARD_INSET))
                .flex()
                .flex_col()
                .gap(px(2.0));
            for (ix, (label, selected)) in choices.into_iter().enumerate() {
                let harness_mark = match field {
                    EditField::Harness => self
                        .harnesses
                        .ready()
                        .map(|list| self.offered(list))
                        .and_then(|list| list.get(ix).map(|d| d.id)),
                    EditField::Model => Some(draft.harness),
                    _ => None,
                };
                list = list.child(
                    div()
                        .id(("profile-edit-choice", ix))
                        .px(px(8.0))
                        .py(px(6.0))
                        .rounded(px(popover::MENU_ITEM_RADIUS))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .cursor_pointer()
                        .when(selected, |el| {
                            el.bg(crate::theme::card_selected_bg())
                                .shadow(crate::theme::card_selected_shadows())
                        })
                        .when(!selected, |el| el.hover(|s| s.bg(crate::theme::ink(0.05))))
                        .on_click(cx.listener(move |this, _, _, cx| this.choose(field, ix, cx)))
                        .when_some(harness_mark, |el, harness| {
                            let (path, tint) = harness_brand_icon(harness);
                            el.child(
                                crate::icons::icon(path)
                                    .size(px(13.0))
                                    .flex_none()
                                    .text_color(tint.unwrap_or(theme.text_muted)),
                            )
                        })
                        .child(
                            div()
                                .flex_1()
                                .truncate()
                                .text_size(crate::typography::ui_rems(12.5))
                                .text_color(theme.text)
                                .child(label),
                        ),
                );
            }
            return Some(
                div()
                    .h(px(height))
                    .flex()
                    .flex_col()
                    .child(header)
                    .child(list)
                    .into_any_element(),
            );
        }

        let label = |text: &'static str| {
            div()
                .mb(px(3.0))
                .text_size(crate::typography::ui_rems(11.0))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(theme.text_muted)
                .child(text)
        };
        let field_box = || {
            div()
                .h(px(30.0))
                .px(px(9.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(crate::theme::hairline(0.12))
                .bg(crate::theme::ink(0.03))
                .flex()
                .items_center()
                .gap(px(7.0))
        };
        let chooser = |id: &'static str,
                       field: EditField,
                       value: String,
                       mark: Option<(&'static str, Option<gpui::Hsla>)>,
                       cx: &mut Context<Self>| {
            field_box()
                .id(id)
                .cursor_pointer()
                .hover(|s| s.bg(crate::theme::ink(0.06)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(edit) = this.profile_edit.as_mut() {
                        edit.choosing = Some(field);
                    }
                    cx.notify();
                }))
                .when_some(mark, |el, (path, tint)| {
                    el.child(
                        crate::icons::icon(path)
                            .size(px(13.0))
                            .flex_none()
                            .text_color(tint.unwrap_or(theme.text_muted)),
                    )
                })
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(crate::typography::ui_rems(12.5))
                        .text_color(theme.text)
                        .child(SharedString::from(value)),
                )
                .child(
                    crate::icons::icon(crate::icons::ALT_ARROW_DOWN)
                        .size(px(11.0))
                        .flex_none()
                        .text_color(theme.text_muted),
                )
        };
        let model_value = draft
            .model
            .as_deref()
            .map(|model| self.profile_model_label(draft.harness, model))
            .unwrap_or_else(|| "Keep current".to_string());
        let mode_value = draft
            .permission_mode
            .map(|mode| mode_label(mode).to_string())
            .unwrap_or_else(|| "Keep current".to_string());
        let effort_value = draft
            .reasoning
            .map(|level| reasoning_label(level).to_string())
            .unwrap_or_else(|| "Keep current".to_string());
        let harness_chooser = chooser(
            "profile-edit-harness",
            EditField::Harness,
            harness_display_name(draft.harness).to_string(),
            Some((brand, tint)),
            cx,
        );
        let model_chooser = chooser(
            "profile-edit-model",
            EditField::Model,
            model_value,
            Some((brand, tint)),
            cx,
        );
        let mode_chooser = chooser("profile-edit-mode", EditField::Mode, mode_value, None, cx);
        let effort_chooser = chooser(
            "profile-edit-effort",
            EditField::Effort,
            effort_value,
            None,
            cx,
        );
        let name = edit.name.clone();
        let notes = edit.notes.clone();
        let error = edit.error.clone();
        let saving = edit.saving;
        let two = |a: AnyElement, b: AnyElement| {
            div()
                .flex()
                .gap(px(8.0))
                .child(div().flex_1().min_w_0().child(a))
                .child(div().flex_1().min_w_0().child(b))
        };
        let body = div()
            .id("profile-edit-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(12.0))
            .py(px(8.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .child(label("Name"))
                    .child(field_box().child(div().flex_1().min_w_0().child(name))),
            )
            .child(two(
                div()
                    .child(label("Agent"))
                    .child(harness_chooser)
                    .into_any_element(),
                div()
                    .child(label("Model"))
                    .child(model_chooser)
                    .into_any_element(),
            ))
            .child(two(
                div()
                    .child(label("Mode"))
                    .child(mode_chooser)
                    .into_any_element(),
                div()
                    .child(label("Effort"))
                    .child(effort_chooser)
                    .into_any_element(),
            ))
            .child(
                div().child(label("When to use")).child(
                    div()
                        .h(px(40.0))
                        .px(px(9.0))
                        .py(px(6.0))
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(crate::theme::hairline(0.12))
                        .bg(crate::theme::ink(0.03))
                        .overflow_hidden()
                        .child(notes),
                ),
            )
            .when_some(error, |el, error| {
                el.child(
                    div()
                        .text_size(crate::typography::ui_rems(11.5))
                        .text_color(theme.danger)
                        .child(error),
                )
            });
        let footer =
            div()
                .flex_none()
                .px(px(12.0))
                .py(px(8.0))
                .border_t_1()
                .border_color(crate::theme::hairline(0.08))
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(
                    div()
                        .id("profile-edit-use-current")
                        .cursor_pointer()
                        .text_size(crate::typography::ui_rems(11.5))
                        .text_color(theme.text_muted)
                        .hover(|s| s.text_color(theme.text))
                        .on_click(cx.listener(|this, _, _, cx| {
                            let current = this.current_draft(cx);
                            this.edit_profile_draft(cx, |draft| {
                                draft.harness = current.harness;
                                draft.model = current.model;
                                draft.reasoning = current.reasoning;
                                draft.permission_mode = current.permission_mode;
                                draft.model_options = current.model_options;
                            });
                        }))
                        .child("Use current selection"),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .id("profile-edit-more")
                        .cursor_pointer()
                        .text_size(crate::typography::ui_rems(11.5))
                        .text_color(theme.text_muted)
                        .hover(|s| s.text_color(theme.text))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_profile_in_settings(window, cx)
                        }))
                        .child("More…"),
                )
                .child(
                    popover::btn_primary(&theme, if saving { "Saving…" } else { "Save" })
                        .id("profile-edit-save")
                        .on_click(cx.listener(|this, _, _, cx| this.save_profile_edit(cx))),
                );
        Some(
            div()
                .h(px(height))
                .flex()
                .flex_col()
                .child(header)
                .child(body)
                .child(footer)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(name: &str, notes: &str) -> AgentProfile {
        AgentProfile {
            id: name.to_lowercase(),
            name: name.into(),
            harness: HarnessId::Codex,
            model: None,
            reasoning: None,
            permission_mode: None,
            model_options: Default::default(),
            notes: notes.into(),
            instructions: String::new(),
            by_agent: false,
        }
    }

    #[test]
    fn search_matches_name_and_notes() {
        let profiles = [profile("Reviewer", "diff review"), profile("UI", "layout")];
        let names = |q: &str| {
            filter_profiles(&profiles, q)
                .into_iter()
                .map(|p| p.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(names(""), ["Reviewer", "UI"]);
        assert_eq!(names("LAYOUT"), ["UI"]);
        assert_eq!(names("rev"), ["Reviewer"]);
    }

    #[test]
    fn a_pick_only_overrides_what_the_profile_sets() {
        let mut config = ChatConfig {
            harness: HarnessId::ClaudeCode,
            model: Some("opus".into()),
            reasoning: Some(ReasoningLevel::High),
            model_options: serde_json::json!({ "context": "1m" })
                .as_object()
                .cloned()
                .unwrap(),
            sandbox: SandboxLevel::WorkspaceWrite,
            permission_mode: PermissionMode::Full,
            instructions: String::new(),
        };
        let mut same = profile("Same", "");
        same.harness = HarnessId::ClaudeCode;
        same.permission_mode = Some(PermissionMode::Ask);
        ProfilePick::of(&same).write(&mut config);
        assert_eq!(config.model.as_deref(), Some("opus"));
        assert_eq!(config.reasoning, Some(ReasoningLevel::High));
        assert_eq!(config.permission_mode, PermissionMode::Ask);
        assert_eq!(config.model_options["context"], "1m");

        let mut codex = profile("Codex", "");
        codex.model = Some("gpt-5.6".into());
        ProfilePick::of(&codex).write(&mut config);
        assert_eq!(config.harness, HarnessId::Codex);
        assert_eq!(config.model.as_deref(), Some("gpt-5.6"));
        assert!(config.model_options.is_empty());
        assert_eq!(config.reasoning, Some(ReasoningLevel::High));
    }
}
