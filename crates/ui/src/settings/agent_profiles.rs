use std::collections::HashMap;

use gpui::{
    AnyElement, Context, Entity, SharedString, Subscription, Task, Window, div, prelude::*, px,
};
use zeron_engine::registry::HarnessDescriptor;
use zeron_proto::{
    AgentProfile, AgentProfileDraft, HarnessId, Model, PermissionMode, ReasoningLevel,
};
use zeron_rpc::methods;

use crate::agent_profiles::{
    ProfileWrite, blank_draft, brand_badge, draft_of, mode_label, summary,
};
use crate::composer::{ComposerInput, ComposerInputEvent};
use crate::pickers::{harness_brand_icon, offered_harnesses, reasoning_label};
use crate::popover::{self, Loadable};
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;

const MODES: [PermissionMode; 3] = [
    PermissionMode::Ask,
    PermissionMode::Auto,
    PermissionMode::Full,
];

struct Editor {
    profile_id: Option<String>,
    draft: AgentProfileDraft,
    name: Entity<ComposerInput>,
    notes: Entity<ComposerInput>,
    instructions: Entity<ComposerInput>,
    error: Option<SharedString>,
    saving: bool,
    confirm_delete: bool,
    _events: [Subscription; 3],
}

pub struct AgentProfilesPage {
    state: Entity<AppState>,
    scroll: widgets::PageScroll,
    profiles: Loadable<Vec<AgentProfile>>,
    harnesses: Vec<HarnessDescriptor>,
    models: HashMap<HarnessId, Vec<Model>>,
    editor: Option<Editor>,
    harness_select: widgets::SelectState,
    model_select: widgets::SelectState,
    mode_select: widgets::SelectState,
    effort_select: widgets::SelectState,
    error: Option<SharedString>,
    task: Option<Task<()>>,
    catalog_task: Option<Task<()>>,
    model_tasks: HashMap<HarnessId, Task<()>>,
    system_prompt: Loadable<String>,
    system_prompt_input: Entity<ComposerInput>,
    system_prompt_task: Option<Task<()>>,
    _state: Subscription,
    _changed: Subscription,
    _system_prompt_events: Subscription,
}

impl AgentProfilesPage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let observer = cx.observe(&state, |page: &mut Self, _, cx| {
            if matches!(page.profiles, Loadable::Idle) {
                page.load(cx);
            }
            cx.notify();
        });
        let changed = cx.observe_global::<crate::agent_profiles::AgentProfilesChanged>(
            |page: &mut Self, cx| page.load(cx),
        );
        let system_prompt_input =
            cx.new(|cx| ComposerInput::new("Always reply in Vietnamese.", cx));
        let system_prompt_events = cx.subscribe(
            &system_prompt_input,
            |_: &mut Self, _, _: &ComposerInputEvent, cx| cx.notify(),
        );
        let mut page = Self {
            state,
            scroll: widgets::PageScroll::default(),
            profiles: Loadable::Idle,
            harnesses: Vec::new(),
            models: HashMap::new(),
            editor: None,
            harness_select: Default::default(),
            model_select: Default::default(),
            mode_select: Default::default(),
            effort_select: Default::default(),
            error: None,
            task: None,
            catalog_task: None,
            model_tasks: HashMap::new(),
            system_prompt: Loadable::Idle,
            system_prompt_input,
            system_prompt_task: None,
            _state: observer,
            _changed: changed,
            _system_prompt_events: system_prompt_events,
        };
        page.load(cx);
        page
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        if matches!(self.profiles, Loadable::Idle) {
            self.profiles = Loadable::Loading;
        }
        if matches!(self.system_prompt, Loadable::Idle) {
            self.system_prompt = Loadable::Loading;
            self.call_system_prompt(None, cx);
        }
        let list_engine = engine.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = crate::agent_profiles::list(&list_engine, None).await;
            this.update(cx, |page, cx| {
                match result {
                    Ok(profiles) => page.profiles = Loadable::Ready(profiles),
                    Err(error) if page.profiles.ready().is_none() => {
                        page.profiles = Loadable::Error(error)
                    }
                    Err(error) => page.error = Some(error.into()),
                }
                cx.notify();
            })
            .ok();
        }));
        if self.harnesses.is_empty() && self.catalog_task.is_none() {
            self.catalog_task = Some(cx.spawn(async move |this, cx| {
                let result = engine
                    .client()
                    .call_as::<Vec<HarnessDescriptor>>(
                        methods::LIST_HARNESSES,
                        serde_json::json!({}),
                    )
                    .await;
                this.update(cx, |page, cx| {
                    page.catalog_task = None;
                    if let Ok(list) = result {
                        page.harnesses = offered_harnesses(&list);
                        for harness in page.harnesses.iter().map(|h| h.id).collect::<Vec<_>>() {
                            page.ensure_models(harness, cx);
                        }
                    }
                    cx.notify();
                })
                .ok();
            }));
        }
    }

    fn ensure_models(&mut self, harness: HarnessId, cx: &mut Context<Self>) {
        if self.models.contains_key(&harness) || self.model_tasks.contains_key(&harness) {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let task = cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(
                    methods::LIST_MODELS,
                    serde_json::json!({ "harness": harness, "force": false }),
                )
                .await;
            this.update(cx, |page, cx| {
                page.model_tasks.remove(&harness);
                if let Some(models) = result.ok().and_then(|value| {
                    serde_json::from_value::<Vec<Model>>(value["models"].clone()).ok()
                }) {
                    page.models.insert(harness, models);
                }
                cx.notify();
            })
            .ok();
        });
        self.model_tasks.insert(harness, task);
    }

    fn call_system_prompt(&mut self, save: Option<String>, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let (method, params) = match &save {
            Some(prompt) => (
                methods::SET_SYSTEM_PROMPT,
                serde_json::json!({ "prompt": prompt }),
            ),
            None => (methods::GET_SYSTEM_PROMPT, serde_json::json!({})),
        };
        let loading = save.is_none();
        self.system_prompt_task = Some(cx.spawn(async move |this, cx| {
            let result = engine.client().call(method, params).await;
            this.update(cx, |page, cx| {
                match result {
                    Ok(value) => {
                        let prompt = value["prompt"].as_str().unwrap_or_default().to_string();
                        page.system_prompt_input
                            .update(cx, |input, cx| input.set_text(prompt.clone(), cx));
                        page.system_prompt = Loadable::Ready(prompt);
                    }
                    Err(error) if loading => {
                        page.system_prompt = Loadable::Error(error.to_string())
                    }
                    Err(error) => page.error = Some(format!("Could not save: {error}").into()),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn render_system_prompt(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let saved = self.system_prompt.ready().cloned();
        let current = self.system_prompt_input.read(cx).text().trim().to_string();
        let dirty = saved.as_ref().is_some_and(|saved| *saved != current);
        div()
            .flex()
            .flex_col()
            .child(widgets::section_label(theme, "System prompt").mt(px(28.0)))
            .child(
                widgets::section_card(theme).mt(px(8.0)).child(
                    widgets::card_row(theme, true)
                        .flex_col()
                        .items_stretch()
                        .gap(px(8.0))
                        .child(
                            div()
                                .text_size(crate::typography::ui_rems(12.0))
                                .text_color(theme.text_muted)
                                .child("Added to every agent's system prompt on this device, before a profile's own instructions."),
                        )
                        .child(popover::dialog_field(
                            div()
                                .h(px(72.0))
                                .overflow_hidden()
                                .child(self.system_prompt_input.clone())
                                .into_any_element(),
                        ))
                        .child(
                            div().flex().justify_end().child(
                                widgets::text_action(
                                    theme,
                                    widgets::ActionTone::Filled,
                                    "Save",
                                )
                                .id("system-prompt-save")
                                .when(!dirty, |el| el.opacity(0.4).cursor_default())
                                .when(dirty, |el| {
                                    el.on_click(cx.listener(|this, _, _, cx| {
                                        let prompt =
                                            this.system_prompt_input.read(cx).text().to_string();
                                        this.call_system_prompt(Some(prompt), cx);
                                    }))
                                }),
                            ),
                        ),
                ),
            )
            .into_any_element()
    }

    pub(crate) fn open_editor(
        &mut self,
        profile_id: Option<String>,
        draft: AgentProfileDraft,
        cx: &mut Context<Self>,
    ) {
        let name = cx.new(|cx| ComposerInput::new("UI work", cx).with_single_line());
        name.update(cx, |input, cx| input.set_text(draft.name.clone(), cx));
        let notes = cx.new(|cx| {
            ComposerInput::new("Use for UI work: components, layout and design tokens.", cx)
        });
        notes.update(cx, |input, cx| input.set_text(draft.notes.clone(), cx));
        let instructions = cx.new(|cx| {
            ComposerInput::new("You review diffs. Never edit files; report findings.", cx)
        });
        instructions.update(cx, |input, cx| {
            input.set_text(draft.instructions.clone(), cx)
        });
        let instructions_events = cx.subscribe(
            &instructions,
            |_: &mut Self, _, _: &ComposerInputEvent, cx| cx.notify(),
        );
        let name_events = cx.subscribe(&name, |this: &mut Self, _, event, cx| {
            if matches!(event, ComposerInputEvent::Submitted) {
                this.save(cx);
            }
        });
        let notes_events = cx.subscribe(&notes, |_: &mut Self, _, _: &ComposerInputEvent, cx| {
            cx.notify()
        });
        self.ensure_models(draft.harness, cx);
        self.editor = Some(Editor {
            profile_id,
            draft,
            name,
            notes,
            instructions,
            error: None,
            saving: false,
            confirm_delete: false,
            _events: [name_events, notes_events, instructions_events],
        });
        cx.notify();
    }

    pub(crate) fn dismiss_on_escape(&mut self, cx: &mut Context<Self>) -> bool {
        if self.editor.take().is_none() {
            return false;
        }
        cx.notify();
        true
    }

    fn edit_draft(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut AgentProfileDraft)) {
        if let Some(editor) = self.editor.as_mut() {
            change(&mut editor.draft);
            editor.confirm_delete = false;
        }
        cx.notify();
    }

    fn pick_harness(&mut self, harness: HarnessId, cx: &mut Context<Self>) {
        self.ensure_models(harness, cx);
        self.edit_draft(cx, |draft| {
            if draft.harness != harness {
                draft.harness = harness;
                draft.model = None;
                draft.model_options.clear();
            }
        });
    }

    fn reasoning_choices(&self, draft: &AgentProfileDraft) -> Vec<ReasoningLevel> {
        let model = draft.model.as_deref().and_then(|id| {
            self.models
                .get(&draft.harness)
                .and_then(|models| models.iter().find(|model| model.id == id))
        });
        match model {
            Some(model) if !model.reasoning_levels.is_empty() => model.reasoning_levels.clone(),
            _ => self
                .harnesses
                .iter()
                .find(|h| h.id == draft.harness)
                .map(|h| h.reasoning_levels.clone())
                .unwrap_or_default(),
        }
    }

    fn harness_name(&self, harness: HarnessId) -> String {
        self.harnesses
            .iter()
            .find(|h| h.id == harness)
            .map(|h| h.name.clone())
            .unwrap_or_else(|| format!("{harness:?}"))
    }

    fn model_label(&self, harness: HarnessId, model: Option<&str>) -> Option<String> {
        let model = model?;
        Some(
            self.models
                .get(&harness)
                .and_then(|models| models.iter().find(|m| m.id == model))
                .map(|m| m.label.clone())
                .unwrap_or_else(|| model.to_string()),
        )
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        if editor.saving {
            return;
        }
        let mut draft = editor.draft.clone();
        draft.name = editor.name.read(cx).text().trim().to_string();
        draft.notes = editor.notes.read(cx).text().trim().to_string();
        draft.instructions = editor.instructions.read(cx).text().trim().to_string();
        if draft.name.is_empty() {
            editor.error = Some("Give the profile a name.".into());
            cx.notify();
            return;
        }
        editor.saving = true;
        let write = ProfileWrite::Upsert {
            profile_id: editor.profile_id.clone(),
            draft,
        };
        self.write(write, true, cx);
    }

    fn remove(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        let Some(profile_id) = editor.profile_id.clone() else {
            return;
        };
        if !editor.confirm_delete {
            editor.confirm_delete = true;
            cx.notify();
            return;
        }
        editor.saving = true;
        self.write(ProfileWrite::Delete { profile_id }, true, cx);
    }

    fn move_profile(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        let Some(profiles) = self.profiles.ready() else {
            return;
        };
        if to >= profiles.len() {
            return;
        }
        let mut next = profiles.clone();
        next.swap(from, to);
        let profile_ids = next.iter().map(|p| p.id.clone()).collect();
        self.profiles = Loadable::Ready(next);
        self.write(ProfileWrite::Reorder { profile_ids }, false, cx);
    }

    fn write(&mut self, write: ProfileWrite, closes_editor: bool, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = crate::agent_profiles::write(&engine, write, None).await;
            this.update(cx, |page, cx| {
                match result {
                    Ok(profiles) => {
                        page.profiles = Loadable::Ready(profiles);
                        page.error = None;
                        if closes_editor {
                            page.editor = None;
                        }
                        crate::agent_profiles::bump_agent_profiles(cx);
                    }
                    Err(error) => match page.editor.as_mut().filter(|_| closes_editor) {
                        Some(editor) => {
                            editor.saving = false;
                            editor.error = Some(error.into());
                        }
                        None => page.error = Some(format!("Could not save: {error}").into()),
                    },
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn render_editor(
        &mut self,
        viewport: gpui::Size<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let theme = Theme::of(cx).for_popup();
        let editor = self.editor.as_ref()?;
        let draft = editor.draft.clone();
        let editing = editor.profile_id.is_some();
        let name = editor.name.clone();
        let notes = editor.notes.clone();
        let instructions = editor.instructions.clone();
        let error = editor.error.clone();
        let confirm_delete = editor.confirm_delete;
        let saving = editor.saving;

        let harnesses = self.harnesses.clone();
        let harness_ix = harnesses
            .iter()
            .position(|h| h.id == draft.harness)
            .unwrap_or(0);
        let harness_select =
            widgets::select("profile-harness", "Agent", &theme, |page: &mut Self| {
                &mut page.harness_select
            })
            .options(
                harnesses.iter().map(|h| {
                    let id = h.id;
                    widgets::SelectOption::new(h.name.clone())
                        .leading(move || brand_mark(id).into_any_element())
                }),
                harness_ix,
            )
            .width(196.0)
            .on_select(move |page, ix, _, cx| {
                if let Some(harness) = harnesses.get(ix) {
                    page.pick_harness(harness.id, cx);
                }
            })
            .render(&self.harness_select, cx);

        let models = self.models.get(&draft.harness).cloned().unwrap_or_default();
        let model_ix = draft
            .model
            .as_deref()
            .and_then(|id| models.iter().position(|m| m.id == id))
            .map_or(0, |ix| ix + 1);
        let harness = draft.harness;
        let mut model_options = vec![
            widgets::SelectOption::new("Keep current")
                .leading(move || brand_mark(harness).into_any_element()),
        ];
        model_options.extend(models.iter().map(|m| {
            widgets::SelectOption::new(m.label.clone())
                .leading(move || brand_mark(harness).into_any_element())
        }));
        let model_select = widgets::select("profile-model", "Model", &theme, |page: &mut Self| {
            &mut page.model_select
        })
        .options(model_options, model_ix)
        .width(196.0)
        .on_select(move |page, ix, _, cx| {
            let model = ix
                .checked_sub(1)
                .and_then(|ix| models.get(ix))
                .map(|m| m.id.clone());
            page.edit_draft(cx, |draft| {
                draft.model = model;
                draft.model_options.clear();
            });
        })
        .render(&self.model_select, cx);

        let mode_ix = draft
            .permission_mode
            .and_then(|mode| MODES.iter().position(|m| *m == mode))
            .map_or(0, |ix| ix + 1);
        let mode_select = widgets::select("profile-mode", "Mode", &theme, |page: &mut Self| {
            &mut page.mode_select
        })
        .options(
            std::iter::once(widgets::SelectOption::new("Keep current")).chain(
                MODES
                    .iter()
                    .map(|mode| widgets::SelectOption::new(mode_label(*mode)).detail(mode.label())),
            ),
            mode_ix,
        )
        .width(196.0)
        .on_select(|page, ix, _, cx| {
            page.edit_draft(cx, |draft| {
                draft.permission_mode = ix.checked_sub(1).map(|ix| MODES[ix]);
            });
        })
        .render(&self.mode_select, cx);

        let levels = self.reasoning_choices(&draft);
        let effort_ix = draft
            .reasoning
            .and_then(|level| levels.iter().position(|l| *l == level))
            .map_or(0, |ix| ix + 1);
        let effort_levels = levels.clone();
        let effort_select =
            widgets::select("profile-effort", "Effort", &theme, |page: &mut Self| {
                &mut page.effort_select
            })
            .options(
                std::iter::once(widgets::SelectOption::new("Keep current")).chain(
                    levels
                        .iter()
                        .map(|l| widgets::SelectOption::new(reasoning_label(*l))),
                ),
                effort_ix,
            )
            .width(196.0)
            .on_select(move |page, ix, _, cx| {
                let level = ix
                    .checked_sub(1)
                    .and_then(|ix| effort_levels.get(ix))
                    .copied();
                page.edit_draft(cx, |draft| draft.reasoning = level);
            })
            .render(&self.effort_select, cx);

        let field = |label: &str, control: AnyElement| {
            div()
                .flex()
                .flex_col()
                .child(field_label(&theme, label))
                .child(control)
        };
        let title = if editing {
            "Edit agent profile"
        } else {
            "New agent profile"
        };
        let mut actions = div()
            .mt(px(16.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0));
        if editing {
            actions = actions.child(
                widgets::text_action(
                    &theme,
                    widgets::ActionTone::Quiet,
                    if confirm_delete {
                        "Confirm remove"
                    } else {
                        "Remove"
                    },
                )
                .id("profile-remove")
                .text_color(theme.danger)
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| this.remove(cx))),
            );
        }
        actions = actions
            .child(div().flex_1())
            .child(
                widgets::text_action(&theme, widgets::ActionTone::Quiet, "Cancel")
                    .id("profile-cancel")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.editor = None;
                        cx.notify();
                    })),
            )
            .child(
                widgets::text_action(
                    &theme,
                    widgets::ActionTone::Solid,
                    if saving { "Saving…" } else { "Save" },
                )
                .id("profile-save")
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
            );

        let card = popover::dialog_card(&theme)
            .id("agent-profile-card")
            .w(px(460.0))
            .role(gpui::Role::Dialog)
            .aria_label(title)
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.editor = None;
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(brand_badge(draft.harness, 24.0, theme.text))
                    .child(popover::dialog_title(&theme, title)),
            )
            .child(field(
                "Name",
                popover::dialog_field(name.into_any_element()).into_any_element(),
            ))
            .child(
                div()
                    .flex()
                    .gap(px(12.0))
                    .child(field("Agent", harness_select.into_any_element()))
                    .child(field("Model", model_select.into_any_element())),
            )
            .child(
                div()
                    .flex()
                    .gap(px(12.0))
                    .child(field("Mode", mode_select.into_any_element()))
                    .child(field("Effort", effort_select.into_any_element())),
            )
            .child(field(
                "Instructions",
                popover::dialog_field(
                    div()
                        .h(px(80.0))
                        .overflow_hidden()
                        .child(instructions)
                        .into_any_element(),
                )
                .into_any_element(),
            ))
            .child(
                div()
                    .mt(px(4.0))
                    .text_size(px(11.0))
                    .text_color(theme.text_muted)
                    .child("Added to the agent's system prompt on every run of a chat using this profile."),
            )
            .child(field(
                "When to use",
                popover::dialog_field(
                    div()
                        .h(px(56.0))
                        .overflow_hidden()
                        .child(notes)
                        .into_any_element(),
                )
                .into_any_element(),
            ))
            .child(
                div()
                    .mt(px(4.0))
                    .text_size(px(11.0))
                    .text_color(theme.text_muted)
                    .child("Agents read this to pick a profile for delegate_task."),
            )
            .when_some(error, |el, error| {
                el.child(
                    div()
                        .mt(px(10.0))
                        .text_size(px(12.0))
                        .text_color(theme.danger)
                        .child(error),
                )
            })
            .child(actions)
            .into_any_element();
        Some(popover::modal("agent-profile-dialog", viewport, card))
    }

    fn on_scroll_hovered(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.scroll.set_list_hovered(*hovered) {
            cx.notify();
        }
    }
}

fn field_label(theme: &Theme, label: &str) -> gpui::Div {
    div()
        .mt(px(12.0))
        .mb(px(5.0))
        .text_size(px(12.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(theme.text_muted)
        .child(SharedString::from(label.to_string()))
}

fn brand_mark(harness: HarnessId) -> impl IntoElement {
    let (path, tint) = harness_brand_icon(harness);
    crate::icons::icon(path)
        .size(px(14.0))
        .text_color(tint.unwrap_or(gpui::white().opacity(0.85)))
}

impl popover::ScrollRailHost for AgentProfilesPage {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        self.scroll.rail_bar()
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.scroll.rail_scroll()
    }
}

impl Render for AgentProfilesPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_settings_surface();
        let dialog = self.render_editor(window.viewport_size(), cx);
        let default_harness = self
            .harnesses
            .first()
            .map(|h| h.id)
            .unwrap_or(HarnessId::ClaudeCode);
        let body: AnyElement = match &self.profiles {
            Loadable::Idle | Loadable::Loading => div()
                .mt(px(16.0))
                .text_color(theme.text_muted)
                .child("Loading…")
                .into_any_element(),
            Loadable::Error(error) => widgets::error_strip(&theme, error.clone())
                .mt(px(16.0))
                .into_any_element(),
            Loadable::Ready(profiles) if profiles.is_empty() => widgets::section_card(&theme)
                .mt(px(8.0))
                .child(
                    widgets::card_row(&theme, true).child(
                        div()
                            .text_size(crate::typography::ui_rems(13.0))
                            .text_color(theme.text_muted)
                            .child("No profiles yet. Add one to start chats from a saved agent and model."),
                    ),
                )
                .into_any_element(),
            Loadable::Ready(profiles) => {
                let count = profiles.len();
                let mut block = widgets::section_card(&theme).mt(px(8.0));
                for (ix, profile) in profiles.iter().enumerate() {
                    let edit_id = profile.id.clone();
                    let edit_draft = draft_of(profile);
                    let line = summary(
                        profile,
                        &self.harness_name(profile.harness),
                        self.model_label(profile.harness, profile.model.as_deref()).as_deref(),
                    );
                    let arrow = |id: &'static str, path: &'static str, enabled: bool, to: usize| {
                        widgets::action_button(&theme, widgets::ActionTone::Quiet)
                            .id((id, ix))
                            .size(px(32.0))
                            .px_0()
                            .py_0()
                            .justify_center()
                            .when(enabled, |el| {
                                el.on_click(cx.listener(move |this, _, _, cx| {
                                    this.move_profile(ix, to, cx)
                                }))
                            })
                            .when(!enabled, |el| el.opacity(0.35).cursor_default())
                            .child(
                                crate::icons::icon(path)
                                    .size(px(14.0))
                                    .text_color(theme.text_muted),
                            )
                    };
                    block = block.child(
                        widgets::card_row(&theme, ix == 0)
                            .gap(px(12.0))
                            .child(brand_badge(profile.harness, 30.0, theme.text))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(160.0))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(8.0))
                                            .child(widgets::row_title(&theme, profile.name.clone()))
                                            .when(profile.by_agent, |el| {
                                                el.child(widgets::badge(&theme, "by agent"))
                                            }),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(5.0))
                                            .text_size(crate::typography::ui_rems(12.0))
                                            .text_color(theme.text_muted)
                                            .child(SharedString::from(line)),
                                    )
                                    .when(!profile.notes.is_empty(), |el| {
                                        el.child(
                                            div()
                                                .mt(px(3.0))
                                                .text_size(crate::typography::ui_rems(12.0))
                                                .italic()
                                                .text_color(theme.text_muted)
                                                .child(SharedString::from(profile.notes.clone())),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .gap(px(2.0))
                                    .child(arrow(
                                        "profile-up",
                                        crate::icons::ALT_ARROW_UP,
                                        ix > 0,
                                        ix.saturating_sub(1),
                                    ))
                                    .child(arrow(
                                        "profile-down",
                                        crate::icons::ALT_ARROW_DOWN,
                                        ix + 1 < count,
                                        ix + 1,
                                    ))
                                    .child(
                                        widgets::text_action(
                                            &theme,
                                            widgets::ActionTone::Filled,
                                            "Edit",
                                        )
                                        .id(("profile-edit", ix))
                                        .ml(px(6.0))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.open_editor(
                                                Some(edit_id.clone()),
                                                edit_draft.clone(),
                                                cx,
                                            )
                                        })),
                                    ),
                            ),
                    );
                }
                block.into_any_element()
            }
        };

        let scrollbar = popover::rail(self, "agent-profiles-page-scrollbar", &theme, cx);
        div()
            .id("agent-profiles-page-host")
            .relative()
            .size_full()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .child(
                crate::edge_fade::edge_faded(
                    16.0,
                    true,
                    true,
                    div()
                        .id("agent-profiles-page")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll.scroll)
                        .child(
                            widgets::page_column()
                                .child(
                                    div()
                                        .flex()
                                        .items_end()
                                        .child(
                                            div()
                                                .flex_1()
                                                .child(widgets::page_header(&theme, "Agent profiles", None)),
                                        )
                                        .child(
                                            widgets::text_action(
                                                &theme,
                                                widgets::ActionTone::Filled,
                                                "New profile",
                                            )
                                            .id("profile-new")
                                            .cursor_pointer()
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.open_editor(None, blank_draft(default_harness), cx)
                                            })),
                                        ),
                                )
                                .child(widgets::page_subtitle(
                                    &theme,
                                    "Saved agent, model, mode and effort. Pick one from the model picker, or let an agent pick one for delegate_task.",
                                ))
                                .when_some(self.error.clone(), |el, message| {
                                    el.child(
                                        widgets::error_strip(&theme, message)
                                            .id("agent-profiles-error")
                                            .cursor_pointer()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.error = None;
                                                cx.notify();
                                            })),
                                    )
                                })
                                .child(body)
                                .child(self.render_system_prompt(&theme, cx)),
                        ),
                )
                .fade_overflow_y(&self.scroll.scroll),
            )
            .children(scrollbar)
            .when_some(dialog, |el, dialog| el.child(dialog))
    }
}
