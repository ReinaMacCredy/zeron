use std::collections::{HashMap, HashSet};

use gpui::{
    AnyElement, Context, Entity, Focusable as _, SharedString, Subscription, Task, Window, div,
    prelude::*, px,
};
use zeron_engine::registry::HarnessDescriptor;
use zeron_proto::tools::{
    HarnessToolInventory, TOOL_GROUPS, ToolGroup, ToolSettingsState, rules_on, set_rules,
    set_zeron_group, tool_group, unlisted_rules, zeron_rule, zeron_unlocked,
};
use zeron_proto::{
    AgentProfile, AgentProfileDraft, HarnessId, Model, PermissionMode, ReasoningLevel,
};
use zeron_rpc::methods;

use crate::agent_profiles::{
    ProfileWrite, blank_draft, brand_badge, draft_summary, expand_toggle, folded_row, in_pairs,
    mode_label, options_for_model, summary, token_count,
};
use crate::composer::{ComposerInput, ComposerInputEvent, INPUT_LINE_HEIGHT, scrolling_field};
use crate::device_panel::setup::activate;
use crate::pickers::{harness_brand_icon, offered_harnesses, reasoning_label};
use crate::popover::{self, Loadable};
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;

struct Editor {
    profile_id: Option<String>,
    draft: AgentProfileDraft,
    name: Entity<ComposerInput>,
    notes: Entity<ComposerInput>,
    instructions: Entity<ComposerInput>,
    expanded: Option<Fold>,
    tools_open: HashSet<ToolGroup>,
    error: Option<SharedString>,
    saving: bool,
    confirm_delete: bool,
    _events: [Subscription; 3],
}

#[derive(Clone, Copy, PartialEq)]
enum LongField {
    Instructions,
    Notes,
}

#[derive(Clone, Copy, PartialEq)]
enum Fold {
    Text(LongField),
    Tools,
}

const FOLDS: [Fold; 3] = [
    Fold::Text(LongField::Instructions),
    Fold::Text(LongField::Notes),
    Fold::Tools,
];

pub struct AgentProfilesPage {
    state: Entity<AppState>,
    scroll: widgets::PageScroll,
    profiles: Loadable<Vec<AgentProfile>>,
    harnesses: Vec<HarnessDescriptor>,
    models: HashMap<HarnessId, Vec<Model>>,
    editor: Option<Editor>,
    boot_expand: Option<Fold>,
    harness_select: widgets::SelectState,
    model_select: widgets::SelectState,
    mode_select: widgets::SelectState,
    effort_select: widgets::SelectState,
    option_selects: HashMap<String, widgets::SelectState>,
    tool_ceiling: Loadable<Vec<String>>,
    claude_tools: Loadable<HarnessToolInventory>,
    tool_ceiling_task: Option<Task<()>>,
    claude_tools_task: Option<Task<()>>,
    error: Option<SharedString>,
    task: Option<Task<()>>,
    list_task: Option<Task<()>>,
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
            boot_expand: match std::env::var("ZERON_OPEN_PROFILE_FIELD").as_deref() {
                Ok("instructions") => Some(Fold::Text(LongField::Instructions)),
                Ok("notes") => Some(Fold::Text(LongField::Notes)),
                Ok("tools") => Some(Fold::Tools),
                _ => None,
            },
            harness_select: Default::default(),
            model_select: Default::default(),
            mode_select: Default::default(),
            effort_select: Default::default(),
            option_selects: HashMap::new(),
            tool_ceiling: Loadable::Idle,
            claude_tools: Loadable::Idle,
            tool_ceiling_task: None,
            claude_tools_task: None,
            error: None,
            task: None,
            list_task: None,
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

    pub(crate) fn load(&mut self, cx: &mut Context<Self>) {
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
        self.list_task = Some(cx.spawn(async move |this, cx| {
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
                            crate::composer::growing_field(self.system_prompt_input.clone(), 72.0, 320.0, cx)
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
        let expanded = self.boot_expand.take();
        self.editor = Some(Editor {
            profile_id,
            draft,
            name,
            notes,
            instructions,
            expanded,
            tools_open: HashSet::new(),
            error: None,
            saving: false,
            confirm_delete: false,
            _events: [name_events, notes_events, instructions_events],
        });
        if expanded == Some(Fold::Tools) {
            self.load_tools(cx);
        }
        cx.notify();
    }

    fn load_tools(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let Some(harness) = self.editor.as_ref().map(|editor| editor.draft.harness) else {
            return;
        };
        if self.tool_ceiling.ready().is_none() {
            self.tool_ceiling = Loadable::Loading;
        }
        let ceiling_engine = engine.clone();
        self.tool_ceiling_task = Some(cx.spawn(async move |this, cx| {
            let reply = ceiling_engine
                .client()
                .call_as::<ToolSettingsState>(methods::GET_TOOL_SETTINGS, serde_json::json!({}))
                .await;
            this.update(cx, |page, cx| {
                match reply {
                    Ok(state) => page.tool_ceiling = Loadable::Ready(state.disabled),
                    Err(error) if page.tool_ceiling.ready().is_none() => {
                        page.tool_ceiling = Loadable::Error(error.to_string())
                    }
                    Err(_) => {}
                }
                cx.notify();
            })
            .ok();
        }));
        if harness != HarnessId::ClaudeCode
            || matches!(self.claude_tools, Loadable::Loading | Loadable::Ready(_))
        {
            return;
        }
        self.claude_tools = Loadable::Loading;
        self.claude_tools_task = Some(cx.spawn(async move |this, cx| {
            let reply = engine
                .client()
                .call_as::<HarnessToolInventory>(
                    methods::LIST_HARNESS_TOOLS,
                    serde_json::json!({ "harness": HarnessId::ClaudeCode }),
                )
                .await;
            this.update(cx, |page, cx| {
                page.claude_tools = match reply {
                    Ok(inventory) => Loadable::Ready(inventory),
                    Err(error) => Loadable::Error(error.to_string()),
                };
                cx.notify();
            })
            .ok();
        }));
    }

    pub(crate) fn dismiss_on_escape(&mut self, cx: &mut Context<Self>) -> bool {
        match self.editor.as_mut() {
            None => return false,
            Some(editor) if editor.expanded.is_some() => editor.expanded = None,
            Some(_) => self.editor = None,
        }
        cx.notify();
        true
    }

    fn expand(&mut self, field: Option<Fold>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        editor.expanded = field;
        match field {
            Some(Fold::Text(field)) => {
                let input = match field {
                    LongField::Instructions => &editor.instructions,
                    LongField::Notes => &editor.notes,
                };
                let handle = input.read(cx).focus_handle(cx);
                window.focus(&handle, cx);
            }
            Some(Fold::Tools) => self.load_tools(cx),
            None => {}
        }
        cx.notify();
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
        let expanded = editor.expanded;
        let tools_open = editor.tools_open.clone();
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
                draft.model_options = options_for_model(
                    &models,
                    model.as_deref(),
                    std::mem::take(&mut draft.model_options),
                );
                draft.model = model;
            });
        })
        .render(&self.model_select, cx);

        let mode_ix = draft
            .permission_mode
            .and_then(|mode| PermissionMode::ALL.iter().position(|m| *m == mode))
            .map_or(0, |ix| ix + 1);
        let mode_select = widgets::select("profile-mode", "Mode", &theme, |page: &mut Self| {
            &mut page.mode_select
        })
        .options(
            std::iter::once(widgets::SelectOption::new("Keep current")).chain(
                PermissionMode::ALL
                    .iter()
                    .map(|mode| widgets::SelectOption::new(mode_label(*mode)).detail(mode.label())),
            ),
            mode_ix,
        )
        .width(196.0)
        .on_select(|page, ix, _, cx| {
            page.edit_draft(cx, |draft| {
                draft.permission_mode = ix.checked_sub(1).map(|ix| PermissionMode::ALL[ix]);
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

        let model_choices = models_for(&self.models, &draft)
            .map(|model| model.options.clone())
            .unwrap_or_default();
        let mut option_fields = Vec::new();
        for option in model_choices {
            let key = option.id.clone();
            self.option_selects.entry(key.clone()).or_default();
            let current = draft.model_options.get(&key).and_then(|v| v.as_str());
            let selected = option
                .choices
                .iter()
                .position(|choice| Some(choice.id.as_str()) == current)
                .map_or(0, |ix| ix + 1);
            let reach_key = key.clone();
            let choices = option.choices.clone();
            let select = widgets::select(
                format!("profile-option-{key}"),
                option.label.clone(),
                &theme,
                move |page: &mut Self| page.option_selects.entry(reach_key.clone()).or_default(),
            )
            .options(
                std::iter::once(widgets::SelectOption::new("Keep current")).chain(
                    option
                        .choices
                        .iter()
                        .map(|choice| widgets::SelectOption::new(choice.label.clone())),
                ),
                selected,
            )
            .width(196.0)
            .on_select(move |page, ix, _, cx| {
                let choice = ix.checked_sub(1).and_then(|ix| choices.get(ix));
                let key = key.clone();
                page.edit_draft(cx, |draft| match choice {
                    Some(choice) => {
                        draft
                            .model_options
                            .insert(key, serde_json::Value::String(choice.id.clone()));
                    }
                    None => {
                        draft.model_options.remove(&key);
                    }
                });
            })
            .render(&self.option_selects[&option.id], cx);
            option_fields.push((option.label, select));
        }

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

        let hint = |text: &'static str| {
            div()
                .mt(px(4.0))
                .text_size(px(11.0))
                .text_color(theme.text_muted)
                .child(text)
        };
        let long_field = |which: LongField,
                          height: gpui::DefiniteLength,
                          cx: &mut Context<Self>| {
            let (label, input, limit, toggle_id, text) = match which {
                LongField::Instructions => (
                    "Instructions",
                    instructions.clone(),
                    zeron_engine::agent_profiles::MAX_INSTRUCTIONS_CHARS,
                    "profile-instructions-expand",
                    "Added to the agent's system prompt on every run of a chat using this profile.",
                ),
                LongField::Notes => (
                    "When to use",
                    notes.clone(),
                    zeron_engine::agent_profiles::MAX_AGENT_PROFILE_NOTES_CHARS,
                    "profile-notes-expand",
                    "Agents read this to pick a profile for delegate_task.",
                ),
            };
            let open = expanded == Some(Fold::Text(which));
            let title = field_label(&theme, label).when(open, |el| {
                el.flex()
                    .child(div().flex_1())
                    .child(token_count(input.read(cx).text(), limit))
            });
            div()
                .flex()
                .flex_col()
                .child(title)
                .child(
                    popover::dialog_field(
                        div()
                            .h(height)
                            .child(scrolling_field(input))
                            .into_any_element(),
                    )
                    .relative()
                    .pr(px(34.0))
                    .child(
                        expand_toggle(toggle_id, open, theme.text_muted).on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.expand((!open).then_some(Fold::Text(which)), window, cx)
                            },
                        )),
                    ),
                )
                .child(hint(text))
        };
        let card = popover::dialog_card(&theme)
            .id("agent-profile-card")
            .w(px(460.0))
            .role(gpui::Role::Dialog)
            .aria_label(title)
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.dismiss_on_escape(cx);
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
            );
        let tools_line = if draft.disabled_tools.is_empty() {
            "All tools on".to_string()
        } else {
            format!("{} off", draft.disabled_tools.len())
        };
        let card = match expanded {
            Some(open) => {
                let mut folded = draft.clone();
                folded.name = name.read(cx).text().to_string();
                let line = draft_summary(
                    &folded,
                    &self.harness_name(draft.harness),
                    self.model_label(draft.harness, draft.model.as_deref())
                        .as_deref(),
                );
                let tall = px((f32::from(viewport.height) - 340.0).clamp(160.0, 440.0));
                let mut card = card.child(
                    folded_row("profile-folded", line, theme.text_muted)
                        .mt(px(12.0))
                        .on_click(cx.listener(|this, _, window, cx| this.expand(None, window, cx))),
                );
                for other in FOLDS.into_iter().filter(|fold| *fold != open) {
                    let (id, label) = match other {
                        Fold::Text(LongField::Instructions) => {
                            ("profile-folded-instructions", "Instructions".to_string())
                        }
                        Fold::Text(LongField::Notes) => {
                            ("profile-folded-notes", "When to use".to_string())
                        }
                        Fold::Tools => ("profile-folded-tools", format!("Tools · {tools_line}")),
                    };
                    card = card.child(
                        folded_row(id, label, theme.text_muted)
                            .mt(px(6.0))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.expand(Some(other), window, cx)
                            })),
                    );
                }
                match open {
                    Fold::Text(which) => card.child(long_field(which, tall.into(), cx)),
                    Fold::Tools => card
                        .child(
                            div().relative().child(field_label(&theme, "Tools")).child(
                                expand_toggle("profile-tools-collapse", true, theme.text_muted)
                                    .top(px(8.0))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.expand(None, window, cx)
                                    })),
                            ),
                        )
                        .child(self.render_tools(&theme, &draft, &tools_open, tall, cx)),
                }
            }
            None => card
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
                .children(in_pairs(option_fields).into_iter().map(|pair| {
                    let mut row = div().flex().gap(px(12.0));
                    for (label, select) in pair {
                        row = row.child(field(&label, select.into_any_element()));
                    }
                    row
                }))
                .child(field(
                    "Tools",
                    folded_row("profile-tools-folded", tools_line, theme.text_muted)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.expand(Some(Fold::Tools), window, cx)
                        }))
                        .into_any_element(),
                ))
                .child(long_field(
                    LongField::Instructions,
                    crate::typography::ui_rems(INPUT_LINE_HEIGHT * 5.0).into(),
                    cx,
                ))
                .child(long_field(
                    LongField::Notes,
                    crate::typography::ui_rems(INPUT_LINE_HEIGHT * 3.0).into(),
                    cx,
                )),
        };
        let card = card
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

    fn render_tools(
        &self,
        theme: &Theme,
        draft: &AgentProfileDraft,
        open_groups: &HashSet<ToolGroup>,
        height: gpui::Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ceiling = self.tool_ceiling.ready().cloned().unwrap_or_default();
        let settled = !matches!(self.tool_ceiling, Loadable::Idle | Loadable::Loading);
        let disabled = &draft.disabled_tools;
        let mut list = div()
            .id("profile-tools")
            .h(height)
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .child(tools_heading(theme, "Zeron tools"));
        if let Loadable::Error(error) = &self.tool_ceiling {
            list = list.child(tools_note(
                theme,
                format!("Settings > Tools unavailable: {error}"),
            ));
        }
        for info in TOOL_GROUPS {
            let group = info.group;
            let unlocked = zeron_unlocked(info, &ceiling);
            let locked = unlocked.is_empty();
            let live = rules_on(disabled, &unlocked);
            let on = !locked && live > 0;
            let open = open_groups.contains(&group);
            let badge = if locked {
                Some(widgets::badge(theme, "off in Settings"))
            } else if live < unlocked.len() {
                Some(widgets::badge_active(
                    theme,
                    format!("{live} of {}", unlocked.len()),
                ))
            } else {
                None
            };
            let group_ceiling = ceiling.clone();
            let title = div()
                .id(SharedString::from(format!("profile-tools-open-{group:?}")))
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(6.0))
                .cursor_pointer()
                .on_click(cx.listener(move |page, _, _, cx| {
                    if let Some(editor) = page.editor.as_mut()
                        && !editor.tools_open.remove(&group)
                    {
                        editor.tools_open.insert(group);
                    }
                    cx.notify();
                }))
                .child(
                    crate::icons::icon(if open {
                        crate::icons::ALT_ARROW_DOWN
                    } else {
                        crate::icons::ALT_ARROW_RIGHT
                    })
                    .size(px(12.0))
                    .flex_none()
                    .text_color(theme.text_muted),
                )
                .child(tools_title(theme, info.title, false))
                .children(badge);
            list = list.child(tools_row(theme, false).child(title).child(activate(
                super::tools::switch(
                    theme,
                    format!("profile-tools-group-{group:?}"),
                    info.title,
                    on,
                ),
                settled && !locked,
                cx,
                move |page: &mut Self, cx| {
                    let ceiling = group_ceiling.clone();
                    page.edit_draft(cx, |draft| {
                        set_zeron_group(&mut draft.disabled_tools, tool_group(group), &ceiling, !on)
                    })
                },
            )));
            if !open {
                continue;
            }
            for tool in info.tools() {
                let name = tool.name;
                let rule = zeron_rule(name);
                let locked = ceiling.iter().any(|off| off == name);
                let on = !locked && !disabled.contains(&rule);
                let text = div()
                    .flex_1()
                    .min_w_0()
                    .child(tools_title(theme, name, true))
                    .when(locked, |el| {
                        el.child(tools_detail(theme, "off in Settings"))
                    });
                list = list.child(tools_row(theme, true).child(text).child(activate(
                    super::tools::switch(theme, format!("profile-tools-tool-{name}"), name, on),
                    settled && !locked,
                    cx,
                    move |page: &mut Self, cx| {
                        let rule = rule.clone();
                        page.edit_draft(cx, |draft| {
                            set_rules(&mut draft.disabled_tools, &[rule], !on)
                        })
                    },
                )));
            }
        }
        let claude = draft.harness == HarnessId::ClaudeCode;
        if claude {
            list = list.child(tools_heading(theme, "Claude built-in"));
            match &self.claude_tools {
                Loadable::Idle | Loadable::Loading => {
                    list = list.child(tools_note(theme, "Asking Claude Code for its tools…"))
                }
                Loadable::Error(error) => {
                    list = list.child(tools_note(
                        theme,
                        format!("Could not list Claude's tools: {error}"),
                    ))
                }
                Loadable::Ready(inventory) => {
                    for builtin in &inventory.builtin {
                        let on = rules_on(disabled, &builtin.tools) == builtin.tools.len();
                        let rules = builtin.tools.clone();
                        let text = div()
                            .flex_1()
                            .min_w_0()
                            .child(tools_title(theme, builtin.title.clone(), false))
                            .when(!builtin.summary.is_empty(), |el| {
                                el.child(tools_detail(theme, builtin.summary.clone()))
                            });
                        list = list.child(tools_row(theme, false).child(text).child(activate(
                            super::tools::switch(
                                theme,
                                format!("profile-tools-builtin-{}", builtin.id),
                                builtin.title.clone(),
                                on,
                            ),
                            true,
                            cx,
                            move |page: &mut Self, cx| {
                                let rules = rules.clone();
                                page.edit_draft(cx, |draft| {
                                    set_rules(&mut draft.disabled_tools, &rules, !on)
                                })
                            },
                        )));
                    }
                    list = list.child(tools_heading(theme, "MCP servers"));
                    if inventory.mcp_servers.is_empty() {
                        list =
                            list.child(tools_note(theme, "No MCP servers in your Claude config."));
                    }
                    for server in &inventory.mcp_servers {
                        let on = !disabled.contains(&server.rule);
                        let rule = server.rule.clone();
                        let text = div()
                            .flex_1()
                            .min_w_0()
                            .child(tools_title(theme, server.name.clone(), false))
                            .when(!server.status.is_empty(), |el| {
                                el.child(tools_detail(theme, server.status.clone()))
                            });
                        list = list.child(tools_row(theme, false).child(text).child(activate(
                            super::tools::switch(
                                theme,
                                format!("profile-tools-server-{}", server.rule),
                                server.name.clone(),
                                on,
                            ),
                            true,
                            cx,
                            move |page: &mut Self, cx| {
                                let rule = rule.clone();
                                page.edit_draft(cx, |draft| {
                                    set_rules(&mut draft.disabled_tools, &[rule], !on)
                                })
                            },
                        )));
                    }
                }
            }
        }
        let inventory = match &self.claude_tools {
            Loadable::Ready(inventory) if claude => Some(inventory.clone()),
            Loadable::Idle | Loadable::Loading if claude => None,
            _ => Some(HarnessToolInventory::default()),
        };
        let unlisted = inventory
            .map(|inventory| unlisted_rules(disabled, &inventory))
            .unwrap_or_default();
        if !unlisted.is_empty() {
            list = list.child(tools_heading(theme, "Not found"));
        }
        for rule in unlisted {
            let text = div()
                .flex_1()
                .min_w_0()
                .child(tools_title(theme, rule.clone(), true))
                .child(tools_detail(theme, "not found · turn on to remove"));
            let id = format!("profile-tools-unlisted-{rule}");
            list = list.child(tools_row(theme, false).child(text).child(activate(
                super::tools::switch(theme, id, rule.clone(), false),
                true,
                cx,
                move |page: &mut Self, cx| {
                    let rule = rule.clone();
                    page.edit_draft(cx, |draft| {
                        set_rules(&mut draft.disabled_tools, &[rule], true)
                    })
                },
            )));
        }
        list.into_any_element()
    }
}

fn models_for<'a>(
    models: &'a HashMap<HarnessId, Vec<Model>>,
    draft: &AgentProfileDraft,
) -> Option<&'a Model> {
    let id = draft.model.as_deref()?;
    models.get(&draft.harness)?.iter().find(|m| m.id == id)
}

fn tools_heading(theme: &Theme, label: &'static str) -> gpui::Div {
    div()
        .mt(px(10.0))
        .mb(px(2.0))
        .text_size(px(11.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(theme.text_muted)
        .child(label)
}

fn tools_note(theme: &Theme, text: impl Into<SharedString>) -> gpui::Div {
    div()
        .py(px(6.0))
        .text_size(px(12.0))
        .text_color(theme.text_muted)
        .child(text.into())
}

fn tools_row(theme: &Theme, nested: bool) -> gpui::Div {
    div()
        .min_h(px(36.0))
        .py(px(4.0))
        .when(nested, |el| el.pl(px(18.0)))
        .border_t_1()
        .border_color(widgets::row_divider(theme))
        .flex()
        .items_center()
        .gap(px(10.0))
}

fn tools_title(theme: &Theme, text: impl Into<SharedString>, mono: bool) -> gpui::Div {
    div()
        .truncate()
        .text_size(px(12.5))
        .text_color(theme.text)
        .when(mono, |el| {
            el.font_family(theme.font_mono.clone()).text_size(px(12.0))
        })
        .child(text.into())
}

fn tools_detail(theme: &Theme, text: impl Into<SharedString>) -> gpui::Div {
    div()
        .truncate()
        .text_size(px(11.0))
        .text_color(theme.text_muted)
        .child(text.into())
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
                    let edit_draft = profile.draft();
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
