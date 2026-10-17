use super::*;

use zeron_proto::{AgentProfile, AgentProfileDraft};

use crate::agent_profiles::{
    ProfileWrite, brand_badge, draft_summary, expand_toggle, folded_row, in_pairs, mode_label,
    summary, token_count,
};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ProfilePick {
    profile_id: String,
    harness: HarnessId,
    model: Option<String>,
    reasoning: Option<ReasoningLevel>,
    permission_mode: Option<PermissionMode>,
    model_options: serde_json::Map<String, serde_json::Value>,
    instructions: String,
    disabled_tools: Vec<String>,
}

impl ProfilePick {
    fn of(profile: &AgentProfile) -> Self {
        Self {
            profile_id: profile.id.clone(),
            harness: profile.harness,
            model: profile.model.clone(),
            reasoning: profile.reasoning,
            permission_mode: profile.permission_mode,
            model_options: profile.model_options.clone(),
            instructions: profile.instructions.clone(),
            disabled_tools: profile.disabled_tools.clone(),
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
        config.disabled_tools = self.disabled_tools.clone();
        config.profile_id = Some(self.profile_id.clone());
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Drift {
    Harness,
    Model,
    Effort,
    Mode,
    Options(Vec<String>),
    Tools,
    Instructions,
}

fn option_drifts(profile: &AgentProfile, current: &ResolvedRunConfig) -> Vec<String> {
    profile
        .model_options
        .iter()
        .filter(|(key, value)| current.model_options.get(*key) != Some(value))
        .map(|(key, _)| key.clone())
        .collect()
}

fn tools_drift(profile: &AgentProfile, current: &ResolvedRunConfig) -> bool {
    use std::collections::BTreeSet;
    profile.disabled_tools.iter().collect::<BTreeSet<_>>()
        != current.disabled_tools.iter().collect::<BTreeSet<_>>()
}

pub(super) fn profile_drift(profile: &AgentProfile, current: &ResolvedRunConfig) -> Vec<Drift> {
    if current
        .harness
        .is_some_and(|harness| harness != profile.harness)
    {
        return vec![Drift::Harness];
    }
    let mut drift = Vec::new();
    if profile
        .model
        .as_ref()
        .is_some_and(|model| current.model.as_ref() != Some(model))
    {
        drift.push(Drift::Model);
    }
    if profile
        .reasoning
        .is_some_and(|level| current.reasoning != Some(level))
    {
        drift.push(Drift::Effort);
    }
    if profile
        .permission_mode
        .is_some_and(|mode| current.permission_mode != mode)
    {
        drift.push(Drift::Mode);
    }
    let options = option_drifts(profile, current);
    if !options.is_empty() {
        drift.push(Drift::Options(options));
    }
    if tools_drift(profile, current) {
        drift.push(Drift::Tools);
    }
    if profile.instructions != current.instructions {
        drift.push(Drift::Instructions);
    }
    drift
}

fn absorb_current(profile: &AgentProfile, current: &ResolvedRunConfig) -> AgentProfileDraft {
    let mut draft = crate::agent_profiles::draft_of(profile);
    let harness_moved = current
        .harness
        .is_some_and(|harness| harness != profile.harness);
    if let Some(harness) = current.harness {
        draft.harness = harness;
    }
    if profile.model.is_some() || harness_moved {
        draft.model = current.model.clone();
    }
    if profile.reasoning.is_some() {
        draft.reasoning = current.reasoning;
    }
    if profile.permission_mode.is_some() {
        draft.permission_mode = Some(current.permission_mode);
    }
    draft.model_options = if harness_moved {
        Default::default()
    } else {
        profile
            .model_options
            .keys()
            .filter_map(|key| {
                current
                    .model_options
                    .get(key)
                    .map(|value| (key.clone(), value.clone()))
            })
            .collect()
    };
    draft.disabled_tools = current.disabled_tools.clone();
    draft
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EditField {
    Harness,
    Model,
    Mode,
    Effort,
    ModelOption(usize),
}

pub(super) struct ProfileEdit {
    profile_id: Option<String>,
    draft: AgentProfileDraft,
    name: Entity<ComposerInput>,
    notes: Entity<ComposerInput>,
    pub(super) choosing: Option<EditField>,
    pub(super) expanded: bool,
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

    fn default_rows(&self, cx: &App) -> usize {
        let has_profiles = self.profiles.ready().is_some_and(|p| !p.is_empty());
        usize::from(has_profiles && self.search.read(cx).text().trim().is_empty())
    }

    pub(super) fn profile_rows_len(&self, cx: &App) -> usize {
        self.default_rows(cx) + self.visible_profiles(cx).len() + 1
    }

    pub(super) fn activate_profile_index(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(ix) = ix.checked_sub(self.default_rows(cx)) else {
            self.detach_profile(cx);
            return;
        };
        let profiles = self.visible_profiles(cx);
        match profiles.get(ix) {
            Some(profile) => self.apply_profile(profile, cx),
            None => self.save_current_as_profile(cx),
        }
    }

    pub(super) fn applied_profile(&self, cx: &App) -> Option<(AgentProfile, Vec<Drift>)> {
        let id = self.effective_profile_id(cx)?;
        let profile = self.profiles.ready()?.iter().find(|p| p.id == id)?.clone();
        let mut current = self.resolved(cx);
        // The picker drops an option set to its default; a profile may name it.
        for option in self.selected_model(cx).map_or(&[][..], |m| &m.options) {
            current
                .model_options
                .entry(option.id.clone())
                .or_insert_with(|| serde_json::Value::String(option.default_choice.clone()));
        }
        let drift = profile_drift(&profile, &current);
        Some((profile, drift))
    }

    fn detach_profile(&mut self, cx: &mut Context<Self>) {
        if self.chat_id(cx).is_some() {
            self.update_chat_config(cx, None, |config| {
                config.profile_id = None;
                config.instructions.clear();
            });
        } else {
            self.config.profile_id = None;
            self.config.instructions.clear();
        }
        self.animate_close(cx);
        cx.notify();
    }

    fn save_current_into_profile(&mut self, profile: &AgentProfile, cx: &mut Context<Self>) {
        let Some(engine) = self.engine(cx) else {
            return;
        };
        let target = self.space_target(cx).map(serde_json::Value::String);
        let write = ProfileWrite::Upsert {
            profile_id: Some(profile.id.clone()),
            draft: absorb_current(profile, &self.resolved(cx)),
        };
        self.profiles_task = Some(cx.spawn(async move |this, cx| {
            let result = crate::agent_profiles::write(&engine, write, target).await;
            this.update(cx, |pickers, cx| {
                pickers.profiles = match result {
                    Ok(profiles) => Loadable::Ready(profiles),
                    Err(error) => Loadable::Error(error),
                };
                crate::agent_profiles::bump_agent_profiles(cx);
                cx.notify();
            })
            .ok();
        }));
    }

    fn drift_line(
        &self,
        profile: &AgentProfile,
        drift: &[Drift],
        resolved: &ResolvedRunConfig,
    ) -> String {
        drift
            .iter()
            .map(|change| match change {
                Drift::Harness => format!(
                    "{} → {}",
                    harness_display_name(profile.harness),
                    resolved
                        .harness
                        .map(harness_display_name)
                        .unwrap_or_default()
                ),
                Drift::Model => format!(
                    "{} → {}",
                    profile
                        .model
                        .as_deref()
                        .map(|model| self.profile_model_label(profile.harness, model))
                        .unwrap_or_default(),
                    resolved
                        .model
                        .as_deref()
                        .map(|model| self.profile_model_label(profile.harness, model))
                        .unwrap_or_default()
                ),
                Drift::Effort => format!(
                    "effort {} → {}",
                    profile.reasoning.map(reasoning_label).unwrap_or_default(),
                    resolved.reasoning.map(reasoning_label).unwrap_or_default()
                ),
                Drift::Mode => format!(
                    "{} → {}",
                    profile.permission_mode.map(mode_label).unwrap_or_default(),
                    mode_label(resolved.permission_mode)
                ),
                Drift::Options(keys) => {
                    let options = resolved
                        .harness
                        .zip(resolved.model.as_deref())
                        .and_then(|(harness, model)| self.profile_catalog_model(harness, model))
                        .map(|model| model.options.clone())
                        .unwrap_or_default();
                    let names = keys
                        .iter()
                        .map(|key| {
                            options
                                .iter()
                                .find(|option| option.id == *key)
                                .map_or(key.as_str(), |option| option.label.as_str())
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("{names} changed")
                }
                Drift::Tools => "tools changed".to_string(),
                Drift::Instructions => "instructions changed".to_string(),
            })
            .collect::<Vec<_>>()
            .join(" · ")
    }

    fn profile_catalog_model(&self, harness: HarnessId, model: &str) -> Option<&Model> {
        self.models
            .get(&harness)
            .and_then(|models| models.ready())
            .and_then(|models| models.iter().find(|m| m.id == model))
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
                    instance: None,
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
            self.config.disabled_tools = pick.disabled_tools.clone();
            self.config.profile_id = Some(pick.profile_id.clone());
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
        draft.disabled_tools = resolved.disabled_tools;
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
            expanded: false,
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
        let target = self.space_target(cx).map(serde_json::Value::String);
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
            let result = crate::agent_profiles::write(&engine, write, target).await;
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

    pub(super) fn open_profile_in_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        let applied = self.applied_profile(cx);
        let resolved = self.resolved(cx);
        let offset = self.default_rows(cx);
        if offset == 1 {
            list = list.child(self.render_default_row(applied.is_none(), &theme, cx));
        }
        for (ix, profile) in profiles.iter().enumerate() {
            let row = ix + offset;
            let is_active = row == self.active;
            let drift = applied
                .as_ref()
                .filter(|(applied, _)| applied.id == profile.id)
                .map(|(_, drift)| drift.clone());
            let line = self.profile_summary(profile);
            let drift_line = drift
                .as_ref()
                .filter(|drift| !drift.is_empty())
                .map(|drift| self.drift_line(profile, drift, &resolved));
            let savable = drift
                .as_ref()
                .is_some_and(|drift| drift.iter().any(|change| *change != Drift::Instructions));
            let edited = drift.as_ref().is_some_and(|drift| !drift.is_empty());
            let switches =
                drift.is_none() && switching.is_some_and(|harness| harness != profile.harness);
            let edit_id = profile.id.clone();
            let edit_draft = crate::agent_profiles::draft_of(profile);
            let revert_profile = profile.clone();
            let save_profile = profile.clone();
            let row_hover = SharedString::from(format!("profile-row-{ix}"));
            list = list.child(
                div()
                    .id(("profile-row", ix))
                    .group(row_hover.clone())
                    .relative()
                    .px(px(8.0))
                    .py(px(5.0))
                    .rounded(px(popover::MENU_ITEM_RADIUS))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .cursor_pointer()
                    .when(drift.is_some(), |el| {
                        el.bg(theme.accent_wash.opacity(0.5)).child(
                            div()
                                .absolute()
                                .left(px(-popover::CARD_INSET))
                                .top(px(8.0))
                                .bottom(px(8.0))
                                .w(px(3.0))
                                .rounded_r(px(2.0))
                                .bg(theme.accent),
                        )
                    })
                    .when(is_active, |el| el.bg(crate::theme::ink(0.05)))
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered && this.active != row {
                            this.active = row;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.activate_profile_index(row, cx);
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
                                    .w_full()
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
                                    .when_some(drift.as_ref(), |el, drift| {
                                        let (label, tint) = if drift.is_empty() {
                                            ("Active", theme.accent)
                                        } else {
                                            ("Edited", theme.warning)
                                        };
                                        el.child(
                                            div()
                                                .flex_none()
                                                .px(px(5.0))
                                                .rounded(px(4.0))
                                                .bg(tint.opacity(0.16))
                                                .text_size(crate::typography::ui_rems(10.0))
                                                .font_weight(gpui::FontWeight::MEDIUM)
                                                .text_color(tint)
                                                .child(label),
                                        )
                                    })
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
                                    })
                                    .when(edited, |el| {
                                        el.child(div().flex_1())
                                            .child(
                                                profile_row_button(
                                                    ("profile-revert", ix),
                                                    "Revert",
                                                    &theme,
                                                )
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    cx.stop_propagation();
                                                    this.apply_profile(&revert_profile, cx);
                                                })),
                                            )
                                            .when(savable, |el| {
                                                el.child(
                                                    profile_row_button(
                                                        ("profile-save", ix),
                                                        "Save",
                                                        &theme,
                                                    )
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        cx.stop_propagation();
                                                        this.save_current_into_profile(
                                                            &save_profile,
                                                            cx,
                                                        );
                                                    })),
                                                )
                                            })
                                    }),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(crate::typography::ui_rems(11.0))
                                    .text_color(theme.text_muted)
                                    .child(SharedString::from(line)),
                            )
                            .when_some(drift_line, |el, line| {
                                el.child(
                                    div()
                                        .text_size(crate::typography::ui_rems(11.0))
                                        .text_color(theme.warning)
                                        .child(SharedString::from(line)),
                                )
                            }),
                    )
                    .when(drift.as_ref().is_some_and(|drift| drift.is_empty()), |el| {
                        el.child(
                            crate::icons::icon(crate::icons::CHECK)
                                .flex_none()
                                .size(px(14.0))
                                .text_color(theme.accent),
                        )
                    })
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
        let save_ix = offset + profiles.len();
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

    fn render_default_row(
        &mut self,
        selected: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id("profile-default")
            .relative()
            .px(px(8.0))
            .py(px(5.0))
            .rounded(px(popover::MENU_ITEM_RADIUS))
            .flex()
            .items_center()
            .gap(px(10.0))
            .cursor_pointer()
            .when(self.active == 0, |el| el.bg(crate::theme::ink(0.05)))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if *hovered && this.active != 0 {
                    this.active = 0;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(|this, _, _, cx| this.activate_profile_index(0, cx)))
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
                        crate::icons::icon(crate::icons::LUCIDE_CIRCLE_DASHED)
                            .size(px(13.0))
                            .text_color(theme.text_muted),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .child(
                        div()
                            .text_size(crate::typography::ui_rems(12.5))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child("No profile"),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(crate::typography::ui_rems(11.0))
                            .text_color(theme.text_muted)
                            .child("Keeps the model, drops instructions"),
                    ),
            )
            .when(selected, |el| {
                el.child(
                    crate::icons::icon(crate::icons::CHECK)
                        .flex_none()
                        .size(px(14.0))
                        .text_color(theme.accent),
                )
            })
            .into_any_element()
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
            EditField::ModelOption(ix) => {
                let Some(option) = self.edit_options(draft).into_iter().nth(ix) else {
                    return Vec::new();
                };
                let current = draft.model_options.get(&option.id).and_then(|v| v.as_str());
                std::iter::once(("Keep current".into(), current.is_none()))
                    .chain(option.choices.iter().map(|choice| {
                        (
                            SharedString::from(choice.label.clone()),
                            current == Some(choice.id.as_str()),
                        )
                    }))
                    .collect()
            }
        }
    }

    fn edit_options(&self, draft: &AgentProfileDraft) -> Vec<zeron_proto::ModelOption> {
        draft
            .model
            .as_deref()
            .and_then(|id| self.profile_catalog_model(draft.harness, id))
            .map(|model| model.options.clone())
            .unwrap_or_default()
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
                let models = self.edit_models(&draft);
                let model = ix
                    .checked_sub(1)
                    .and_then(|ix| models.get(ix).map(|m| m.id.clone()));
                self.edit_profile_draft(cx, |draft| {
                    draft.model_options = crate::agent_profiles::options_for_model(
                        &models,
                        model.as_deref(),
                        std::mem::take(&mut draft.model_options),
                    );
                    draft.model = model;
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
            EditField::ModelOption(option_ix) => {
                let Some(option) = self.edit_options(&draft).into_iter().nth(option_ix) else {
                    return;
                };
                let choice = ix
                    .checked_sub(1)
                    .and_then(|ix| option.choices.get(ix).map(|c| c.id.clone()));
                self.edit_profile_draft(cx, |draft| match choice {
                    Some(choice) => {
                        draft
                            .model_options
                            .insert(option.id, serde_json::Value::String(choice));
                    }
                    None => {
                        draft.model_options.remove(&option.id);
                    }
                });
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

        let label = |text: &str| {
            div()
                .mb(px(3.0))
                .text_size(crate::typography::ui_rems(11.0))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(theme.text_muted)
                .child(SharedString::from(text.to_string()))
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
        let chooser = |id: gpui::ElementId,
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
            "profile-edit-harness".into(),
            EditField::Harness,
            harness_display_name(draft.harness).to_string(),
            Some((brand, tint)),
            cx,
        );
        let model_chooser = chooser(
            "profile-edit-model".into(),
            EditField::Model,
            model_value,
            Some((brand, tint)),
            cx,
        );
        let mode_chooser = chooser(
            "profile-edit-mode".into(),
            EditField::Mode,
            mode_value,
            None,
            cx,
        );
        let effort_chooser = chooser(
            "profile-edit-effort".into(),
            EditField::Effort,
            effort_value,
            None,
            cx,
        );
        let option_fields = self
            .edit_options(&draft)
            .into_iter()
            .enumerate()
            .map(|(ix, option)| {
                let value = draft
                    .model_options
                    .get(&option.id)
                    .and_then(|v| v.as_str())
                    .and_then(|id| option.choices.iter().find(|c| c.id == id))
                    .map_or_else(|| "Keep current".to_string(), |c| c.label.clone());
                div()
                    .child(label(&option.label))
                    .child(chooser(
                        ("profile-edit-option", ix).into(),
                        EditField::ModelOption(ix),
                        value,
                        None,
                        cx,
                    ))
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let name = edit.name.clone();
        let notes = edit.notes.clone();
        let error = edit.error.clone();
        let saving = edit.saving;
        let expanded = edit.expanded;
        let two = |a: AnyElement, b: AnyElement| {
            div()
                .flex()
                .gap(px(8.0))
                .child(div().flex_1().min_w_0().child(a))
                .child(div().flex_1().min_w_0().child(b))
        };
        let notes_label = if expanded {
            label("When to use")
                .flex()
                .child(div().flex_1())
                .child(token_count(
                    notes.read(cx).text(),
                    zeron_engine::agent_profiles::MAX_AGENT_PROFILE_NOTES_CHARS,
                ))
        } else {
            label("When to use")
        };
        let notes_field = div().flex_1().flex().flex_col().child(notes_label).child(
            div()
                .relative()
                .flex_1()
                .min_h(px(40.0))
                .pl(px(9.0))
                .pr(px(30.0))
                .py(px(6.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(crate::theme::hairline(0.12))
                .bg(crate::theme::ink(0.03))
                .overflow_hidden()
                .child(crate::composer::scrolling_field(notes.clone()))
                .child(
                    expand_toggle("profile-edit-notes-expand", expanded, theme.text_muted)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let Some(edit) = this.profile_edit.as_mut() {
                                edit.expanded = !edit.expanded;
                            }
                            let handle = notes.read(cx).focus_handle(cx);
                            window.focus(&handle, cx);
                            cx.notify();
                        })),
                ),
        );
        let body = div()
            .id("profile-edit-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(12.0))
            .py(px(8.0))
            .flex()
            .flex_col()
            .gap(px(6.0));
        let body = if expanded {
            let mut folded = draft.clone();
            folded.name = name.read(cx).text().to_string();
            body.child(
                folded_row(
                    "profile-edit-folded",
                    draft_summary(
                        &folded,
                        harness_display_name(draft.harness),
                        draft
                            .model
                            .as_deref()
                            .map(|model| self.profile_model_label(draft.harness, model))
                            .as_deref(),
                    ),
                    theme.text_muted,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(edit) = this.profile_edit.as_mut() {
                        edit.expanded = false;
                    }
                    cx.notify();
                })),
            )
        } else {
            body.child(
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
            .children(in_pairs(option_fields).into_iter().map(|pair| {
                let mut pair = pair.into_iter();
                let first = pair.next().unwrap_or_else(|| div().into_any_element());
                let second = pair.next().unwrap_or_else(|| div().into_any_element());
                two(first, second)
            }))
        };
        let body = body.child(notes_field).when_some(error, |el, error| {
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
                                draft.disabled_tools = current.disabled_tools;
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

fn profile_row_button(
    id: (&'static str, usize),
    label: &'static str,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .px(px(7.0))
        .py(px(2.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(crate::theme::hairline(0.12))
        .text_size(crate::typography::ui_rems(11.0))
        .text_color(theme.text)
        .hover(|s| s.bg(crate::theme::ink(0.08)))
        .child(label)
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
            disabled_tools: Vec::new(),
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
            profile_id: None,
            instance_id: None,
            disabled_tools: Vec::new(),
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
        assert_eq!(config.profile_id.as_deref(), Some("codex"));
    }

    #[test]
    fn drift_compares_only_what_the_profile_sets() {
        let mut planner = profile("Planner", "");
        planner.harness = HarnessId::ClaudeCode;
        planner.reasoning = Some(ReasoningLevel::High);
        planner.model_options = serde_json::json!({ "context": "1m" })
            .as_object()
            .cloned()
            .unwrap();
        let mut current = ResolvedRunConfig {
            harness: Some(HarnessId::ClaudeCode),
            instance_id: None,
            model: Some("opus".into()),
            reasoning: Some(ReasoningLevel::High),
            model_options: serde_json::json!({ "context": "1m", "fast": "on" })
                .as_object()
                .cloned()
                .unwrap(),
            permission_mode: PermissionMode::Full,
            instructions: String::new(),
            profile_id: Some("planner".into()),
            disabled_tools: Vec::new(),
        };
        assert!(profile_drift(&planner, &current).is_empty());

        current.reasoning = Some(ReasoningLevel::Max);
        current.model_options.remove("context");
        assert_eq!(
            profile_drift(&planner, &current),
            [Drift::Effort, Drift::Options(vec!["context".into()])]
        );
        let saved = absorb_current(&planner, &current);
        assert_eq!(saved.reasoning, Some(ReasoningLevel::Max));
        assert!(saved.model.is_none());
        assert!(saved.permission_mode.is_none());
        assert!(saved.model_options.is_empty());

        current.harness = Some(HarnessId::Codex);
        assert_eq!(profile_drift(&planner, &current), [Drift::Harness]);
    }

    fn tools(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn a_pick_replaces_the_tool_set_even_when_empty() {
        let mut config = ChatConfig {
            harness: HarnessId::ClaudeCode,
            model: None,
            reasoning: None,
            model_options: Default::default(),
            sandbox: SandboxLevel::WorkspaceWrite,
            permission_mode: PermissionMode::Full,
            instructions: String::new(),
            profile_id: None,
            instance_id: None,
            disabled_tools: tools(&["WebFetch"]),
        };
        let mut lean = profile("Lean", "");
        lean.harness = HarnessId::ClaudeCode;
        lean.disabled_tools = tools(&["mcp__xcode", "mcp__zeron__delegate_task"]);
        ProfilePick::of(&lean).write(&mut config);
        assert_eq!(
            config.disabled_tools,
            tools(&["mcp__xcode", "mcp__zeron__delegate_task"])
        );

        let mut full = profile("Full", "");
        full.harness = HarnessId::ClaudeCode;
        ProfilePick::of(&full).write(&mut config);
        assert!(config.disabled_tools.is_empty());

        lean.harness = HarnessId::Codex;
        ProfilePick::of(&lean).write(&mut config);
        assert_eq!(config.disabled_tools, lean.disabled_tools);
    }

    #[test]
    fn drift_compares_tool_sets_and_save_absorbs_them() {
        let mut lean = profile("Lean", "");
        lean.harness = HarnessId::ClaudeCode;
        lean.disabled_tools = tools(&["WebFetch", "mcp__xcode"]);
        let mut current = ResolvedRunConfig {
            harness: Some(HarnessId::ClaudeCode),
            permission_mode: PermissionMode::Full,
            profile_id: Some("lean".into()),
            disabled_tools: tools(&["mcp__xcode", "WebFetch"]),
            ..Default::default()
        };
        assert!(profile_drift(&lean, &current).is_empty());

        current.disabled_tools.push("NotebookEdit".into());
        assert_eq!(profile_drift(&lean, &current), [Drift::Tools]);
        let saved = absorb_current(&lean, &current);
        assert_eq!(saved.disabled_tools, current.disabled_tools);

        current.disabled_tools.clear();
        assert_eq!(profile_drift(&lean, &current), [Drift::Tools]);
        lean.disabled_tools.clear();
        assert!(profile_drift(&lean, &current).is_empty());
    }
}
