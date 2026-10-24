use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::time::Instant;

use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, Hsla, KeyDownEvent, ScrollHandle,
    SharedString, Subscription, Task, Window, div, prelude::*, px,
};
use zeron_proto::claude_config::{
    self as cc, ClaudeConfigSnapshot, Kind, LocalCommandOutput, Scope, Setting,
};
use zeron_proto::usage::{UsageSummary, format_tokens};
use zeron_proto::{HarnessId, PermissionMode, ReasoningLevel};
use zeron_rpc::methods;

use super::background::{CardMotion, card_frame, card_motion};
use super::{Composer, ComposerEvent, ComposerInput, ComposerInputEvent};
use crate::icons::{self, icon};
use crate::popover;
use crate::theme::Theme;

pub(crate) const SUMMARY_PREFIX: &str = "Claude Code settings · ";
// Clears the titlebar, the Chat | Work toggle and the New task row above the panel.
pub(crate) const HERO_PANEL_TOP: f32 = Theme::TITLEBAR_HEIGHT + 96.0;
const HERO_BELOW_PILL: f32 = 64.0;
const ABOVE_PILL: f32 = 52.0;
const MIN_HEIGHT: f32 = 180.0;
const STATS_DAYS: u32 = 7 * 36;
const READ_ONLY_NOTE: &str =
    "Claude Code does not set this headless; change it in the terminal's /config.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tab {
    Status,
    Config,
    Usage,
    Stats,
}

impl Tab {
    const ALL: [Tab; 4] = [Tab::Status, Tab::Config, Tab::Usage, Tab::Stats];

    fn label(self) -> &'static str {
        match self {
            Tab::Status => "Status",
            Tab::Config => "Config",
            Tab::Usage => "Usage",
            Tab::Stats => "Stats",
        }
    }

    fn step(self, forward: bool) -> Tab {
        let ix = Tab::ALL.iter().position(|t| *t == self).unwrap_or(0);
        let n = Tab::ALL.len();
        Tab::ALL[if forward {
            (ix + 1) % n
        } else {
            (ix + n - 1) % n
        }]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Filter {
    All,
    Zeron,
    Terminal,
}

enum Load<T> {
    Loading,
    Ready(T),
    Failed(String),
}

impl<T> Load<T> {
    fn from(result: Result<T, String>) -> Self {
        match result {
            Ok(value) => Load::Ready(value),
            Err(error) => Load::Failed(error),
        }
    }
}

pub(super) struct ConfigPanel {
    chat_id: Option<String>,
    note_chat: Option<String>,
    session_id: Option<String>,
    params: serde_json::Value,
    tab: Tab,
    filter: Filter,
    selected: usize,
    expanded: Option<&'static str>,
    option_sel: usize,
    editing: Option<&'static str>,
    search: Entity<ComposerInput>,
    text: Entity<ComposerInput>,
    values: Load<BTreeMap<String, String>>,
    files: Vec<String>,
    notes: HashMap<&'static str, String>,
    refused: HashSet<&'static str>,
    queue: VecDeque<(&'static str, String, String)>,
    writing: Option<&'static str>,
    originals: HashMap<&'static str, String>,
    changes: Vec<(&'static str, String)>,
    status: Load<LocalCommandOutput>,
    stats: Option<Load<UsageSummary>>,
    opened_at: Instant,
    closing: Option<Instant>,
    focus: FocusHandle,
    focus_pending: bool,
    scroll: ScrollHandle,
    tasks: Vec<Task<()>>,
    write_task: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

#[derive(Clone, Debug)]
struct RowView {
    setting: &'static Setting,
    value: String,
    options: Vec<String>,
    writable: bool,
    note: Option<String>,
}

impl RowView {
    fn is_bool(&self) -> bool {
        self.setting.kind == Kind::Bool
    }

    fn is_text(&self) -> bool {
        self.setting.kind == Kind::Text && self.setting.chip.is_none()
    }

    fn matches(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        let query = query.to_lowercase();
        [self.setting.label, self.setting.id, self.value.as_str()]
            .iter()
            .any(|field| field.to_lowercase().contains(&query))
    }
}

fn filter_matches(filter: Filter, setting: &Setting) -> bool {
    match filter {
        Filter::All => true,
        Filter::Zeron => setting.scope == Scope::Zeron,
        Filter::Terminal => setting.scope == Scope::Terminal,
    }
}

fn effort_value(level: Option<ReasoningLevel>) -> String {
    level.map(|l| l.label().to_string()).unwrap_or_default()
}

impl Composer {
    pub(crate) fn config_panel_on_hero(&self, cx: &App) -> bool {
        self.config_panel.is_some() && self.bound_chat_id(cx).is_none()
    }

    fn config_panel_height(&self, window: &Window, cx: &App) -> f32 {
        let viewport = f32::from(window.viewport_size().height);
        let pill = self.surface_bounds.get();
        let room = if self.config_panel_on_hero(cx) {
            let pill_height = pill.map_or(120.0, |b| f32::from(b.size.height));
            viewport - HERO_PANEL_TOP - pill_height - HERO_BELOW_PILL
        } else {
            let pill_top = pill.map_or(viewport - 120.0, |b| f32::from(b.top()));
            pill_top - ABOVE_PILL - Theme::TITLEBAR_HEIGHT
        };
        (viewport * 0.62)
            .clamp(300.0, 620.0)
            .min(room)
            .max(MIN_HEIGHT)
    }

    pub(super) fn config_panel_open(&self) -> bool {
        self.config_panel
            .as_ref()
            .is_some_and(|panel| panel.closing.is_none())
    }

    pub(crate) fn open_config_panel(&mut self, cx: &mut Context<Self>) {
        if self.config_panel_open() {
            if let Some(panel) = self.config_panel.as_mut() {
                panel.focus_pending = true;
            }
            cx.notify();
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.set_chat_failure("Engine not connected".into(), cx);
            return;
        };
        self.close_ask_tray(cx);
        let chat = self.bound_chat_row(cx).cloned();
        let local = self.state.read(cx).local_device_id.clone();
        let params = self.catalog_params(cx);
        let search = cx.new(|cx| {
            ComposerInput::with_context("Search settings…", "PaletteSearch", cx)
                .with_accessibility_role(gpui::Role::SearchInput)
                .with_text_metrics(12.5, 16.0)
        });
        let text = cx.new(|cx| {
            ComposerInput::with_context("e.g. Vietnamese", "PaletteSearch", cx)
                .with_text_metrics(12.0, 16.0)
        });
        let subs = vec![cx.subscribe(&search, |this, _, event, cx| {
            if matches!(event, ComposerInputEvent::Edited)
                && let Some(panel) = this.config_panel.as_mut()
            {
                panel.selected = 0;
                panel.expanded = None;
                cx.notify();
            }
        })];
        let mut panel = ConfigPanel {
            chat_id: chat.as_ref().map(|c| c.id.clone()),
            note_chat: chat
                .as_ref()
                .filter(|c| Some(&c.device_id) == local.as_ref())
                .map(|c| c.id.clone()),
            session_id: chat.as_ref().and_then(|c| c.harness_session_id.clone()),
            params: params.clone(),
            tab: Tab::Config,
            filter: Filter::All,
            selected: 0,
            expanded: None,
            option_sel: 0,
            editing: None,
            search,
            text,
            values: Load::Loading,
            files: Vec::new(),
            notes: HashMap::new(),
            refused: HashSet::new(),
            queue: VecDeque::new(),
            writing: None,
            originals: HashMap::new(),
            changes: Vec::new(),
            status: Load::Loading,
            stats: None,
            opened_at: crate::motion::now(),
            closing: None,
            focus: cx.focus_handle(),
            focus_pending: true,
            scroll: ScrollHandle::new(),
            tasks: Vec::new(),
            write_task: None,
            _subs: subs,
        };
        let read = {
            let engine = engine.clone();
            let params = params.clone();
            cx.spawn(async move |this, cx| {
                let reply = engine
                    .client()
                    .call_as::<ClaudeConfigSnapshot>(methods::READ_CLAUDE_CONFIG, params)
                    .await
                    .map_err(|e| e.to_string());
                this.update(cx, |this, cx| this.config_snapshot_arrived(reply, cx))
                    .ok();
            })
        };
        let mut usage_params = params;
        usage_params["command"] = "/usage".into();
        usage_params["connectMcp"] = true.into();
        let status = cx.spawn(async move |this, cx| {
            let reply = engine
                .client()
                .call_as::<LocalCommandOutput>(methods::RUN_CLAUDE_LOCAL_COMMAND, usage_params)
                .await
                .map_err(|e| e.to_string());
            this.update(cx, |this, cx| {
                if let Some(panel) = this.config_panel.as_mut() {
                    panel.status = Load::from(reply);
                    cx.notify();
                }
            })
            .ok();
        });
        panel.tasks = vec![read, status];
        self.config_panel = Some(panel);
        cx.notify();
    }

    fn config_snapshot_arrived(
        &mut self,
        reply: Result<ClaudeConfigSnapshot, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self.config_panel.as_mut() else {
            return;
        };
        match reply {
            Ok(snapshot) => {
                let mut values = snapshot.values;
                for (id, _, value) in &panel.queue {
                    values.insert(id.to_string(), value.clone());
                }
                panel.files = snapshot.files;
                panel.values = Load::Ready(values);
            }
            Err(error) if !matches!(panel.values, Load::Ready(_)) => {
                panel.values = Load::Failed(error)
            }
            Err(_) => {}
        }
        cx.notify();
    }

    pub(super) fn close_config_panel(&mut self, cx: &mut Context<Self>) {
        let Some(panel) = self.config_panel.as_mut() else {
            return;
        };
        if panel.closing.is_some() {
            return;
        }
        panel.closing = Some(crate::motion::now());
        panel.expanded = None;
        panel.editing = None;
        let summary = summary_text(&panel.changes);
        let note_chat = panel.note_chat.clone();
        if let (Some(text), Some(chat_id)) = (summary, note_chat)
            && let Some(engine) = self.state.read(cx).engine().cloned()
        {
            cx.background_spawn(async move {
                if let Err(error) = engine
                    .client()
                    .call(
                        methods::MUTATE,
                        serde_json::json!({ "op": "addChatNote", "chatId": chat_id, "text": text }),
                    )
                    .await
                {
                    tracing::warn!(%error, "Claude Code settings summary was not written");
                }
            })
            .detach();
        }
        self.focus_pending = true;
        cx.notify();
    }

    fn config_rows(&self, panel: &ConfigPanel, cx: &App) -> Vec<RowView> {
        let values = match &panel.values {
            Load::Ready(values) => Some(values),
            _ => None,
        };
        let chips = self.pickers.read(cx).chip_choices(cx);
        cc::SETTINGS
            .iter()
            .filter_map(|setting| {
                let row = match setting.chip {
                    Some(cc::Chip::Model) => {
                        let label = |id: &str| {
                            chips
                                .models
                                .iter()
                                .find(|(m, _)| m == id)
                                .map(|(_, label)| label.clone())
                                .unwrap_or_else(|| id.to_string())
                        };
                        RowView {
                            setting,
                            value: chips.model.as_deref().map(label).unwrap_or_default(),
                            options: chips.models.iter().map(|(_, l)| l.clone()).collect(),
                            writable: !chips.models.is_empty(),
                            note: None,
                        }
                    }
                    Some(cc::Chip::Effort) => RowView {
                        setting,
                        value: if chips.efforts.is_empty() {
                            "not offered by this model".into()
                        } else {
                            effort_value(chips.effort)
                        },
                        options: chips
                            .efforts
                            .iter()
                            .map(|l| l.label().to_string())
                            .collect(),
                        writable: !chips.efforts.is_empty(),
                        note: None,
                    },
                    Some(cc::Chip::Permission) => RowView {
                        setting,
                        value: chips.permission.label().to_string(),
                        options: PermissionMode::ALL
                            .iter()
                            .map(|m| m.label().to_string())
                            .collect(),
                        writable: true,
                        note: None,
                    },
                    None => {
                        let value = values?.get(setting.id)?.clone();
                        let refused = panel.refused.contains(setting.id);
                        RowView {
                            setting,
                            value,
                            options: match setting.kind {
                                Kind::Enum(options) => {
                                    options.iter().map(|o| o.to_string()).collect()
                                }
                                Kind::Bool => vec!["true".into(), "false".into()],
                                Kind::Text => Vec::new(),
                            },
                            writable: setting.cli.is_some() && !refused,
                            note: panel.notes.get(setting.id).cloned().or_else(|| {
                                (setting.cli.is_none()).then(|| READ_ONLY_NOTE.to_string())
                            }),
                        }
                    }
                };
                Some(row)
            })
            .collect()
    }

    fn visible_config_rows(&self, panel: &ConfigPanel, cx: &App) -> Vec<RowView> {
        let query = panel.search.read(cx).text().trim().to_string();
        self.config_rows(panel, cx)
            .into_iter()
            .filter(|row| filter_matches(panel.filter, row.setting) && row.matches(&query))
            .collect()
    }

    fn activate_config_row(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(panel) = self.config_panel.as_ref() else {
            return;
        };
        let Some(row) = self.visible_config_rows(panel, cx).get(ix).cloned() else {
            return;
        };
        let id = row.setting.id;
        let current = row.options.iter().position(|o| *o == row.value);
        let panel = self.config_panel.as_mut().unwrap();
        panel.selected = ix;
        if !row.writable {
        } else if row.is_bool() {
            let next = if row.value == "true" { "false" } else { "true" };
            self.set_config_value(&row, next.to_string(), cx);
        } else if row.is_text() {
            panel.expanded = None;
            if panel.editing == Some(id) {
                panel.editing = None;
                panel.focus_pending = true;
            } else {
                panel.editing = Some(id);
                let text = panel.text.clone();
                text.update(cx, |input, cx| input.set_text(row.value.clone(), cx));
                window.focus(&text.focus_handle(cx), cx);
            }
        } else if row.options.len() == 2 {
            let next = row.options[(current.unwrap_or(0) + 1) % 2].clone();
            self.set_config_value(&row, next, cx);
        } else {
            panel.editing = None;
            if panel.expanded == Some(id) {
                panel.expanded = None;
            } else {
                panel.expanded = Some(id);
                panel.option_sel = current.unwrap_or(0);
                panel.scroll.scroll_to_item(ix);
            }
        }
        cx.notify();
    }

    fn pick_config_option(&mut self, option: usize, cx: &mut Context<Self>) {
        let Some(panel) = self.config_panel.as_ref() else {
            return;
        };
        let Some(id) = panel.expanded else {
            return;
        };
        let Some(row) = self
            .config_rows(panel, cx)
            .into_iter()
            .find(|row| row.setting.id == id)
        else {
            return;
        };
        if let Some(value) = row.options.get(option).cloned() {
            self.set_config_value(&row, value, cx);
        }
        if let Some(panel) = self.config_panel.as_mut() {
            panel.expanded = None;
            panel.focus_pending = true;
        }
        cx.notify();
    }

    fn save_config_text(&mut self, cx: &mut Context<Self>) {
        let Some(panel) = self.config_panel.as_mut() else {
            return;
        };
        let Some(id) = panel.editing.take() else {
            return;
        };
        panel.focus_pending = true;
        let value = panel.text.read(cx).text().trim().to_string();
        let Some(row) = self
            .config_rows(self.config_panel.as_ref().unwrap(), cx)
            .into_iter()
            .find(|row| row.setting.id == id)
        else {
            return;
        };
        self.set_config_value(&row, value, cx);
        cx.notify();
    }

    fn set_config_value(&mut self, row: &RowView, value: String, cx: &mut Context<Self>) {
        if row.value == value {
            return;
        }
        let id = row.setting.id;
        match row.setting.chip {
            Some(chip) => {
                let chips = self.pickers.read(cx).chip_choices(cx);
                self.pickers.update(cx, |pickers, cx| match chip {
                    cc::Chip::Model => {
                        if let Some((model, _)) = chips.models.iter().find(|(_, l)| *l == value) {
                            pickers.set_chip_model(model.clone(), cx);
                        }
                    }
                    cc::Chip::Effort => {
                        if let Some(level) = chips.efforts.iter().find(|l| l.label() == value) {
                            pickers.set_chip_effort(*level, cx);
                        }
                    }
                    cc::Chip::Permission => {
                        if let Some(mode) = PermissionMode::ALL.iter().find(|m| m.label() == value)
                        {
                            pickers.set_chip_permission(*mode, cx);
                        }
                    }
                });
                if let Some(panel) = self.config_panel.as_mut() {
                    record_change(panel, id, &row.value, &value);
                }
            }
            None => {
                let Some(panel) = self.config_panel.as_mut() else {
                    return;
                };
                if cc::config_command(row.setting, &value).is_none() {
                    panel
                        .notes
                        .insert(id, "Quotes and line breaks cannot be written.".into());
                    cx.notify();
                    return;
                }
                if let Load::Ready(values) = &mut panel.values {
                    values.insert(id.to_string(), value.clone());
                }
                panel.notes.remove(id);
                panel.queue.push_back((id, row.value.clone(), value));
                self.drain_config_writes(cx);
            }
        }
        cx.notify();
    }

    // One /config at a time: each rewrites the whole file, so two at once can drop a change.
    fn drain_config_writes(&mut self, cx: &mut Context<Self>) {
        let Some(panel) = self.config_panel.as_mut() else {
            return;
        };
        if panel.writing.is_some() {
            return;
        }
        let Some((id, before, value)) = panel.queue.front().cloned() else {
            return;
        };
        let Some(setting) = cc::setting(id) else {
            panel.queue.pop_front();
            return;
        };
        let Some(command) = cc::config_command(setting, &value) else {
            panel.queue.pop_front();
            return;
        };
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        panel.writing = Some(id);
        let mut params = panel.params.clone();
        params["command"] = command.into();
        params["connectMcp"] = false.into();
        let read_params = panel.params.clone();
        panel.write_task = Some(cx.spawn(async move |this, cx| {
            let reply = engine
                .client()
                .call_as::<LocalCommandOutput>(methods::RUN_CLAUDE_LOCAL_COMMAND, params)
                .await
                .map_err(|e| e.to_string());
            let saved = matches!(&reply, Ok(out) if cc::config_saved(&out.text));
            let snapshot = if saved {
                Some(
                    engine
                        .client()
                        .call_as::<ClaudeConfigSnapshot>(methods::READ_CLAUDE_CONFIG, read_params)
                        .await
                        .map_err(|e| e.to_string()),
                )
            } else {
                None
            };
            this.update(cx, |this, cx| {
                let Some(panel) = this.config_panel.as_mut() else {
                    return;
                };
                panel.writing = None;
                panel.queue.pop_front();
                let confirmed = snapshot
                    .as_ref()
                    .and_then(|s| s.as_ref().ok())
                    .and_then(|s| s.values.get(id))
                    .is_none_or(|on_disk| *on_disk == value);
                match reply {
                    Ok(_) if saved && confirmed => record_change(panel, id, &before, &value),
                    Ok(out) => {
                        if cc::config_key_unknown(&out.text) {
                            panel.refused.insert(id);
                        }
                        let text = if saved {
                            "Claude Code reported it saved, but the file still holds the old value."
                                .to_string()
                        } else {
                            out.text.trim().to_string()
                        };
                        panel.notes.insert(id, text);
                        if let Load::Ready(values) = &mut panel.values {
                            values.insert(id.to_string(), before.clone());
                        }
                    }
                    Err(error) => {
                        panel.notes.insert(id, error);
                        if let Load::Ready(values) = &mut panel.values {
                            values.insert(id.to_string(), before.clone());
                        }
                    }
                }
                if let Some(snapshot) = snapshot {
                    this.config_snapshot_arrived(snapshot, cx);
                }
                this.drain_config_writes(cx);
                cx.notify();
            })
            .ok();
        }));
    }

    fn select_config_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        let Some(panel) = self.config_panel.as_mut() else {
            return;
        };
        panel.tab = tab;
        panel.expanded = None;
        panel.editing = None;
        panel.focus_pending = true;
        if tab == Tab::Stats && panel.stats.is_none() {
            panel.stats = Some(Load::Loading);
            if let Some(engine) = self.state.read(cx).engine().cloned() {
                let mut params = serde_json::json!({ "days": STATS_DAYS });
                if let Some(target) = panel.params.get("targetDeviceId") {
                    params["targetDeviceId"] = target.clone();
                }
                panel.tasks.push(cx.spawn(async move |this, cx| {
                    let reply = engine
                        .client()
                        .call_as::<UsageSummary>(methods::GET_USAGE_SUMMARY, params)
                        .await
                        .map_err(|e| e.to_string());
                    this.update(cx, |this, cx| {
                        if let Some(panel) = this.config_panel.as_mut() {
                            panel.stats = Some(Load::from(reply));
                            cx.notify();
                        }
                    })
                    .ok();
                }));
            }
        }
        cx.notify();
    }

    fn on_config_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(panel) = self.config_panel.as_ref() else {
            return;
        };
        let in_search = panel.search.focus_handle(cx).is_focused(window);
        let in_text = panel.text.focus_handle(cx).is_focused(window);
        let key = event.keystroke.key.as_str();
        let mods = event.keystroke.modifiers;
        if mods.platform || mods.control || mods.alt {
            return;
        }
        match key {
            "tab" => {
                let tab = panel.tab.step(!mods.shift);
                self.select_config_tab(tab, cx);
            }
            "escape" => {
                let panel = self.config_panel.as_mut().unwrap();
                if panel.expanded.is_some() {
                    panel.expanded = None;
                } else if panel.editing.is_some() {
                    panel.editing = None;
                    panel.focus_pending = true;
                } else if !panel.search.read(cx).text().is_empty() {
                    panel.search.update(cx, |input, cx| input.set_text("", cx));
                    panel.selected = 0;
                } else {
                    self.close_config_panel(cx);
                }
                cx.notify();
            }
            "enter" if in_text => self.save_config_text(cx),
            _ if in_text => return,
            _ if panel.tab != Tab::Config => return,
            "up" | "down" => {
                let down = key == "down";
                let rows = self.visible_config_rows(panel, cx).len();
                let options = panel.expanded.and_then(|id| {
                    self.config_rows(panel, cx)
                        .into_iter()
                        .find(|r| r.setting.id == id)
                        .map(|r| r.options.len())
                });
                let panel = self.config_panel.as_mut().unwrap();
                if let Some(options) = options {
                    panel.option_sel = if down {
                        (panel.option_sel + 1).min(options.saturating_sub(1))
                    } else {
                        panel.option_sel.saturating_sub(1)
                    };
                } else if rows > 0 {
                    panel.selected = if down {
                        (panel.selected + 1).min(rows - 1)
                    } else {
                        panel.selected.saturating_sub(1)
                    };
                    panel.scroll.scroll_to_item(panel.selected);
                }
                cx.notify();
            }
            "enter" | "space" if !(key == "space" && in_search) => {
                if panel.expanded.is_some() {
                    let option = panel.option_sel;
                    self.pick_config_option(option, cx);
                } else {
                    let ix = panel.selected;
                    self.activate_config_row(ix, window, cx);
                }
            }
            "/" if !in_search => {
                let handle = panel.search.focus_handle(cx);
                window.focus(&handle, cx);
            }
            _ if !in_search => {
                let Some(typed) = event
                    .keystroke
                    .key_char
                    .clone()
                    .filter(|c| c.chars().count() == 1 && !c.trim().is_empty())
                else {
                    return;
                };
                let search = panel.search.clone();
                search.update(cx, |input, cx| {
                    let text = format!("{}{typed}", input.text());
                    input.set_text(text, cx)
                });
                window.focus(&search.focus_handle(cx), cx);
                let panel = self.config_panel.as_mut().unwrap();
                panel.selected = 0;
                panel.expanded = None;
                cx.notify();
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    pub(super) fn render_config_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let bound = self.bound_chat_id(cx);
        let claude = self.pickers.read(cx).resolved(cx).harness == Some(HarnessId::ClaudeCode);
        if self.config_panel_open()
            && (!claude || self.config_panel.as_ref().unwrap().chat_id != bound)
        {
            self.close_config_panel(cx);
        }
        let panel = self.config_panel.as_ref()?;
        let now = crate::motion::now();
        let reduced = crate::motion::reduced_motion(cx);
        let motion = card_motion(panel.opened_at, panel.closing, reduced, now);
        if motion.gone {
            self.config_panel = None;
            return None;
        }
        if motion.animating {
            window.request_animation_frame();
        }
        if motion.live && panel.focus_pending {
            let focus = panel.focus.clone();
            self.config_panel.as_mut().unwrap().focus_pending = false;
            window.focus(&focus, cx);
        }
        let theme = Theme::of(cx).clone();
        let height = self.config_panel_height(window, cx);
        Some(self.config_card(motion, height, &theme, window, cx))
    }

    fn config_card(
        &self,
        motion: CardMotion,
        height: f32,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let panel = self.config_panel.as_ref().unwrap();
        let tab = panel.tab;
        let tabs = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(2.0))
            .px(px(12.0))
            .pt(px(8.0))
            .border_b_1()
            .border_color(crate::theme::hairline(0.06))
            .child(
                div()
                    .mr(px(8.0))
                    .text_size(crate::typography::ui_rems(11.5))
                    .text_color(theme.text_faint)
                    .child("Claude Code settings"),
            )
            .children(Tab::ALL.iter().map(|t| {
                let t = *t;
                let on = t == tab;
                div()
                    .id(("claude-config-tab", t as usize))
                    .debug_selector(move || format!("claude-config-tab-{}", t.label()))
                    .role(gpui::Role::Tab)
                    .aria_label(t.label())
                    .aria_selected(on)
                    .relative()
                    .px(px(12.0))
                    .pt(px(6.0))
                    .pb(px(9.0))
                    .cursor_pointer()
                    .text_size(crate::typography::ui_rems(12.5))
                    .text_color(if on { theme.text } else { theme.text_muted })
                    .hover(|s| s.text_color(theme.text))
                    .child(t.label())
                    .when(on, |el| {
                        el.child(
                            div()
                                .absolute()
                                .left(px(10.0))
                                .right(px(10.0))
                                .bottom(px(-1.0))
                                .h(px(2.0))
                                .rounded(px(2.0))
                                .bg(theme.accent),
                        )
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.select_config_tab(t, cx)))
            }))
            .child(div().flex_1())
            .child(
                div()
                    .id("claude-config-close")
                    .debug_selector(|| "claude-config-close".into())
                    .role(gpui::Role::Button)
                    .aria_label("Close (Esc)")
                    .size(px(26.0))
                    .mb(px(4.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.glass_hover()))
                    .child(
                        icon(icons::CLOSE)
                            .size(px(12.0))
                            .text_color(theme.text_muted),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.close_config_panel(cx))),
            );
        let body = match tab {
            Tab::Config => self.config_pane(theme, window, cx),
            Tab::Status => self.status_pane(theme, cx),
            Tab::Usage => self.usage_pane(theme, cx),
            Tab::Stats => self.stats_pane(theme, cx),
        };
        let card = div()
            .id("claude-config")
            .role(gpui::Role::Dialog)
            .aria_label("Claude Code settings")
            .when(motion.live, |el| {
                el.debug_selector(|| "claude-config".into())
                    .track_focus(&panel.focus)
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        this.on_config_key(event, window, cx)
                    }))
            })
            .occlude()
            .w_full()
            .h(px(height))
            .rounded(px(14.0))
            .border_1()
            .border_color(theme.border_strong)
            .bg(popover::surface_bg(theme))
            .when(!theme.is_frost(), |el| el.shadow_lg())
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(tabs)
            .child(div().flex_1().min_h_0().flex().flex_col().child(body))
            .child(self.config_footer(theme));
        card_frame(card, motion).into_any_element()
    }

    fn config_footer(&self, theme: &Theme) -> impl IntoElement {
        let panel = self.config_panel.as_ref().unwrap();
        let hint = |keys: &[&str], label: &'static str| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(4.0))
                .children(keys.iter().map(|k| kbd(theme, k)))
                .child(label)
        };
        let saved = match (panel.changes.len(), panel.writing) {
            (_, Some(_)) => "Saving…".to_string(),
            (0, None) if panel.files.is_empty() => "Claude Code defaults".to_string(),
            (0, None) => format!("Read from {}", short_files(&panel.files)),
            (1, None) => "1 change, saved".to_string(),
            (n, None) => format!("{n} changes, saved"),
        };
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.0))
            .px(px(14.0))
            .py(px(8.0))
            .border_t_1()
            .border_color(crate::theme::hairline(0.06))
            .text_size(crate::typography::ui_rems(11.5))
            .text_color(theme.text_faint)
            .child(hint(&["↑↓"], "select"))
            .child(hint(&["Enter", "Space"], "change"))
            .child(hint(&["/"], "search"))
            .child(hint(&["Tab"], "tab"))
            .child(hint(&["Esc"], "close"))
            .child(
                div()
                    .ml_auto()
                    .min_w_0()
                    .truncate()
                    .text_color(theme.text_muted)
                    .child(saved),
            )
    }

    fn config_pane(&self, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let panel = self.config_panel.as_ref().unwrap();
        let all = self.config_rows(panel, cx);
        let query = panel.search.read(cx).text().trim().to_string();
        let rows: Vec<RowView> = all
            .iter()
            .filter(|row| filter_matches(panel.filter, row.setting) && row.matches(&query))
            .cloned()
            .collect();
        let selected = panel.selected.min(rows.len().saturating_sub(1));
        let search_focused = panel.search.focus_handle(cx).is_focused(window);
        let search = div()
            .mx(px(12.0))
            .mt(px(10.0))
            .mb(px(6.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .px(px(10.0))
            .h(px(32.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(if search_focused {
                theme.accent
            } else {
                theme.border
            })
            .child(
                icon(icons::LUCIDE_SEARCH)
                    .size(px(13.0))
                    .flex_none()
                    .text_color(theme.text_muted),
            )
            .child(div().flex_1().min_w_0().child(panel.search.clone()))
            .child(kbd(theme, "/"));
        let count = |filter: Filter| {
            cc::SETTINGS
                .iter()
                .filter(|s| filter_matches(filter, s))
                .count()
        };
        let filters = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .px(px(12.0))
            .pb(px(6.0))
            .children(
                [
                    (Filter::All, "All"),
                    (Filter::Zeron, "Affects Zeron chats"),
                    (Filter::Terminal, "Terminal only"),
                ]
                .into_iter()
                .map(|(filter, label)| {
                    let on = panel.filter == filter;
                    div()
                        .id(("claude-config-filter", filter as usize))
                        .debug_selector(move || format!("claude-config-filter-{label}"))
                        .role(gpui::Role::Button)
                        .aria_selected(on)
                        .px(px(9.0))
                        .h(px(20.0))
                        .flex()
                        .items_center()
                        .rounded_full()
                        .border_1()
                        .cursor_pointer()
                        .text_size(crate::typography::ui_rems(11.5))
                        .border_color(if on { theme.accent } else { theme.border })
                        .text_color(if on { theme.accent } else { theme.text_muted })
                        .when(on, |el| el.bg(theme.accent.opacity(0.14)))
                        .child(format!("{label} {}", count(filter)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(panel) = this.config_panel.as_mut() {
                                panel.filter = filter;
                                panel.selected = 0;
                                panel.expanded = None;
                                cx.notify();
                            }
                        }))
                }),
            )
            .child(
                div()
                    .ml_auto()
                    .text_size(crate::typography::ui_rems(11.5))
                    .text_color(theme.text_faint)
                    .child(format!("{} shown", rows.len())),
            );
        let list: AnyElement = match &panel.values {
            Load::Loading => empty_note(theme, "Reading Claude Code's settings…"),
            Load::Failed(error) => empty_note(theme, &format!("Could not read settings: {error}")),
            Load::Ready(_) if rows.is_empty() => {
                empty_note(theme, &format!("No settings match “{query}”"))
            }
            Load::Ready(_) => {
                div()
                    .id("claude-config-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&panel.scroll)
                    .px(px(6.0))
                    .pb(px(8.0))
                    .children(rows.iter().enumerate().map(|(ix, row)| {
                        self.config_row(ix, row, ix == selected, &query, theme, cx)
                    }))
                    .into_any_element()
            }
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(search)
            .child(filters)
            .child(list)
            .into_any_element()
    }

    fn config_row(
        &self,
        ix: usize,
        row: &RowView,
        selected: bool,
        query: &str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let panel = self.config_panel.as_ref().unwrap();
        let id = row.setting.id;
        let changed = panel.changes.iter().any(|(c, _)| *c == id);
        let dim = !row.writable;
        let mono = theme.font_mono.clone();
        let value: AnyElement = if row.is_bool() {
            let on = row.value == "true";
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(crate::settings::widgets::toggle_switch(
                    theme,
                    on,
                    format!("claude-config-{id}"),
                ))
                .child(
                    div()
                        .text_color(if on { theme.text } else { theme.text_muted })
                        .child(row.value.clone()),
                )
                .into_any_element()
        } else if row.is_text() && row.value.is_empty() {
            div()
                .text_color(theme.text_faint)
                .child("default")
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(highlight(&row.value, query, theme))
                .when(row.options.len() > 2 && row.writable, |el| {
                    el.child(div().text_color(theme.text_faint).child("›"))
                })
                .into_any_element()
        };
        let tag = match (row.setting.chip, row.setting.scope) {
            (Some(_), _) => Some(("= chip", true)),
            (None, Scope::Terminal) => Some(("Terminal", false)),
            _ => None,
        };
        let line = div()
            .id(("claude-config-row", ix))
            .debug_selector(move || format!("claude-config-row-{id}"))
            .role(gpui::Role::ListItem)
            .aria_label(SharedString::from(row.setting.label))
            .aria_selected(selected)
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .px(px(8.0))
            .py(px(5.0))
            .rounded(px(7.0))
            .text_size(crate::typography::ui_rems(12.5))
            .when(selected, |el| el.bg(theme.accent.opacity(0.16)))
            .when(!selected, |el| el.hover(|s| s.bg(theme.glass_hover())))
            .child(
                div()
                    .w(px(10.0))
                    .flex_none()
                    .text_color(theme.accent)
                    .when(!selected, |el| el.opacity(0.0))
                    .child("❯"),
            )
            .child(
                div()
                    .w(gpui::relative(0.46))
                    .flex_none()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .items_baseline()
                    .gap(px(6.0))
                    .overflow_hidden()
                    .when(dim, |el| el.opacity(0.5))
                    .child(
                        div()
                            .flex_none()
                            .child(highlight(row.setting.label, query, theme)),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .font_family(mono.clone())
                            .text_size(crate::typography::ui_rems(10.5))
                            .text_color(theme.text_faint)
                            .child(highlight(id, query, theme)),
                    ),
            )
            .child(
                div()
                    .id(("claude-config-value", ix))
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .font_family(mono)
                    .text_size(crate::typography::ui_rems(12.0))
                    .when(dim, |el| el.opacity(0.5))
                    .child(value)
                    .when(changed, |el| {
                        el.child(div().size(px(6.0)).rounded_full().bg(theme.accent))
                    })
                    .when(row.writable, |el| el.cursor_pointer())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.activate_config_row(ix, window, cx);
                    })),
            )
            .when_some(tag, |el, (label, chip)| {
                el.child(
                    div()
                        .flex_none()
                        .px(px(6.0))
                        .rounded(px(4.0))
                        .text_size(crate::typography::ui_rems(10.0))
                        .bg(if chip {
                            theme.accent.opacity(0.14)
                        } else {
                            crate::theme::ink(0.06)
                        })
                        .text_color(if chip { theme.accent } else { theme.text_muted })
                        .child(label),
                )
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                let already = this
                    .config_panel
                    .as_ref()
                    .is_some_and(|panel| panel.selected == ix);
                if already {
                    this.activate_config_row(ix, window, cx);
                } else if let Some(panel) = this.config_panel.as_mut() {
                    panel.selected = ix;
                    panel.expanded = None;
                    panel.focus_pending = true;
                    cx.notify();
                }
            }));
        let note = row
            .note
            .clone()
            .filter(|_| selected || panel.notes.contains_key(id))
            .map(|note| {
                div()
                    .pl(px(28.0))
                    .pb(px(4.0))
                    .text_size(crate::typography::ui_rems(11.5))
                    .text_color(theme.text_faint)
                    .child(note)
            });
        let options = (panel.expanded == Some(id)).then(|| {
            div()
                .ml(px(28.0))
                .mr(px(8.0))
                .mt(px(2.0))
                .mb(px(6.0))
                .p(px(4.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(theme.border)
                .bg(crate::theme::ink(0.03))
                .children(row.options.iter().enumerate().map(|(oix, option)| {
                    let current = *option == row.value;
                    let hot = oix == panel.option_sel;
                    div()
                        .id(("claude-config-option", oix))
                        .debug_selector(move || format!("claude-config-option-{oix}"))
                        .role(gpui::Role::MenuItem)
                        .aria_selected(current)
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .px(px(8.0))
                        .py(px(4.0))
                        .rounded(px(6.0))
                        .cursor_pointer()
                        .font_family(theme.font_mono.clone())
                        .text_size(crate::typography::ui_rems(12.0))
                        .when(hot, |el| el.bg(theme.accent.opacity(0.16)))
                        .when(!hot, |el| el.hover(|s| s.bg(theme.glass_hover())))
                        .child(
                            div()
                                .size(px(7.0))
                                .rounded_full()
                                .border_1()
                                .border_color(if current {
                                    theme.accent
                                } else {
                                    theme.text_muted
                                })
                                .when(current, |el| el.bg(theme.accent)),
                        )
                        .child(SharedString::from(option.clone()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.pick_config_option(oix, cx);
                        }))
                }))
        });
        let editor = (panel.editing == Some(id)).then(|| {
            div()
                .ml(px(28.0))
                .mr(px(8.0))
                .mt(px(2.0))
                .mb(px(6.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(
                    div()
                        .flex_1()
                        .px(px(8.0))
                        .h(px(26.0))
                        .flex()
                        .items_center()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(theme.accent)
                        .child(panel.text.clone()),
                )
                .child(
                    div()
                        .id("claude-config-save-text")
                        .debug_selector(|| "claude-config-save-text".into())
                        .role(gpui::Role::Button)
                        .px(px(10.0))
                        .h(px(26.0))
                        .flex()
                        .items_center()
                        .rounded(px(7.0))
                        .border_1()
                        .border_color(theme.border)
                        .cursor_pointer()
                        .text_size(crate::typography::ui_rems(12.0))
                        .hover(|s| s.bg(theme.glass_hover()))
                        .child("Save")
                        .on_click(cx.listener(|this, _, _, cx| this.save_config_text(cx))),
                )
        });
        div()
            .flex()
            .flex_col()
            .child(line)
            .children(note)
            .children(options)
            .children(editor)
            .into_any_element()
    }

    fn status_pane(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let panel = self.config_panel.as_ref().unwrap();
        let chips = self.pickers.read(cx).chip_choices(cx);
        let model = chips
            .model
            .as_deref()
            .map(|id| {
                let label = chips
                    .models
                    .iter()
                    .find(|(m, _)| m == id)
                    .map(|(_, l)| l.as_str())
                    .unwrap_or(id);
                format!("{id} · {label}")
            })
            .unwrap_or_else(|| "default".into());
        let session = panel
            .session_id
            .as_deref()
            .map(short_id)
            .unwrap_or_else(|| "No session yet".into());
        let mut rows: Vec<(&str, String, bool)> = Vec::new();
        let init = match &panel.status {
            Load::Ready(out) => out.init.clone(),
            _ => None,
        };
        let field = |key: &str| {
            init.as_ref()
                .and_then(|i| i.get(key))
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        };
        let waiting = match &panel.status {
            Load::Loading => "Asking Claude Code…".to_string(),
            Load::Failed(error) => error.clone(),
            Load::Ready(_) => "Not reported".to_string(),
        };
        let or_wait = |value: Option<String>| value.unwrap_or_else(|| waiting.clone());
        rows.push(("Version", or_wait(field("claude_code_version")), false));
        rows.push(("Session", session, false));
        rows.push(("Folder", or_wait(field("cwd").map(|c| tilde(&c))), false));
        rows.push(("Model", model, true));
        rows.push((
            "Permission mode",
            chips.permission.label().to_string(),
            true,
        ));
        rows.push(("Output style", or_wait(field("output_style")), false));
        rows.push((
            "MCP servers",
            or_wait(init.as_ref().map(|i| mcp_summary(i.get("mcp_servers")))),
            false,
        ));
        rows.push((
            "Memory",
            or_wait(init.as_ref().map(|i| memory_summary(i.get("memory_paths")))),
            false,
        ));
        rows.push((
            "Plugins",
            or_wait(init.as_ref().map(|i| plugins_summary(i.get("plugins")))),
            false,
        ));
        div()
            .id("claude-config-status")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p(px(6.0))
            .children(rows.into_iter().enumerate().map(|(ix, (k, v, chip))| {
                kv_row(theme, ix > 0, k, v, chip.then_some("= chip"))
            }))
            .child(
                div()
                    .mx(px(14.0))
                    .mt(px(10.0))
                    .text_size(crate::typography::ui_rems(11.5))
                    .text_color(theme.text_faint)
                    .child(
                        "From Claude Code's system/init for this folder; model and permission mode are this chat's.",
                    ),
            )
            .into_any_element()
    }

    fn usage_pane(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let panel = self.config_panel.as_ref().unwrap();
        let open = div().mx(px(14.0)).mt(px(10.0)).flex().child(
            div()
                .id("claude-config-open-usage")
                .debug_selector(|| "claude-config-open-usage".into())
                .role(gpui::Role::Button)
                .px(px(10.0))
                .h(px(26.0))
                .flex()
                .items_center()
                .rounded(px(7.0))
                .border_1()
                .border_color(theme.border)
                .cursor_pointer()
                .text_size(crate::typography::ui_rems(12.5))
                .hover(|s| s.bg(theme.glass_hover()))
                .child("Open Usage page")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.close_config_panel(cx);
                    cx.emit(ComposerEvent::OpenUsage);
                })),
        );
        let body: AnyElement = match &panel.status {
            Load::Loading => empty_note(theme, "Asking Claude Code for /usage…"),
            Load::Failed(error) => empty_note(theme, error),
            Load::Ready(out) => {
                let report = parse_usage(&out.text);
                if report.meters.is_empty() && report.sections.is_empty() {
                    empty_note(theme, out.text.trim())
                } else {
                    div()
                        .flex()
                        .flex_col()
                        .children(report.meters.into_iter().map(|m| meter(theme, m)))
                        .children(report.sections.into_iter().enumerate().map(|(ix, s)| {
                            let detail = s
                                .lines
                                .iter()
                                .take(3)
                                .cloned()
                                .collect::<Vec<_>>()
                                .join("\n");
                            kv_row(theme, ix > 0, &s.title, detail, None)
                        }))
                        .children(report.note.map(|note| {
                            div()
                                .mx(px(14.0))
                                .mt(px(8.0))
                                .text_size(crate::typography::ui_rems(11.5))
                                .text_color(theme.text_faint)
                                .child(note)
                        }))
                        .into_any_element()
                }
            }
        };
        div()
            .id("claude-config-usage")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p(px(6.0))
            .child(body)
            .child(open)
            .into_any_element()
    }

    fn stats_pane(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let panel = self.config_panel.as_ref().unwrap();
        let body: AnyElement = match &panel.stats {
            None | Some(Load::Loading) => empty_note(theme, "Counting Claude Code sessions…"),
            Some(Load::Failed(error)) => empty_note(theme, error),
            Some(Load::Ready(summary)) => match stats_of(summary) {
                None => empty_note(
                    theme,
                    "No Claude Code activity on this device in the last 36 weeks.",
                ),
                Some(stats) => {
                    let favorite = stats
                        .favorite_model
                        .as_deref()
                        .map(|id| {
                            crate::pickers::model_display_label(HarnessId::ClaudeCode, id, cx)
                        })
                        .unwrap_or_else(|| "No data".into());
                    let max = stats.days.iter().copied().max().unwrap_or(0).max(1);
                    let grid = div()
                        .mx(px(14.0))
                        .my(px(8.0))
                        .flex()
                        .flex_row()
                        .gap(px(3.0))
                        .children(stats.days.chunks(7).map(|week| {
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(3.0))
                                .children(week.iter().map(|tokens| {
                                    div()
                                        .size(px(11.0))
                                        .rounded(px(2.0))
                                        .bg(heat(theme, *tokens, max))
                                }))
                        }));
                    let tiles = [
                        (favorite, "Favorite model"),
                        (stats.sessions.to_string(), "Sessions"),
                        (stats.active_days.to_string(), "Active days"),
                        (plural_days(stats.streak), "Current streak"),
                        ("No data".to_string(), "Longest session"),
                        (format_tokens(stats.tokens), "Tokens"),
                    ];
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .mx(px(14.0))
                                .mt(px(8.0))
                                .text_size(crate::typography::ui_rems(11.5))
                                .text_color(theme.text_faint)
                                .child("Activity · last 36 weeks"),
                        )
                        .child(grid)
                        .child(
                            div()
                                .mx(px(14.0))
                                .my(px(10.0))
                                .flex()
                                .flex_col()
                                .gap(px(8.0))
                                .children(tiles.chunks(3).map(|row| {
                                    div()
                                        .flex()
                                        .flex_row()
                                        .gap(px(8.0))
                                        .children(row.iter().map(|(value, label)| {
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .px(px(10.0))
                                                .py(px(8.0))
                                                .rounded(px(8.0))
                                                .border_1()
                                                .border_color(theme.border)
                                                .child(
                                                    div()
                                                        .truncate()
                                                        .text_size(crate::typography::ui_rems(13.5))
                                                        .text_color(theme.text)
                                                        .child(value.clone()),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(crate::typography::ui_rems(11.5))
                                                        .text_color(theme.text_faint)
                                                        .child(*label),
                                                )
                                        }))
                                })),
                        )
                        .into_any_element()
                }
            },
        };
        div()
            .id("claude-config-stats")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p(px(6.0))
            .child(body)
            .child(
                div()
                    .mx(px(14.0))
                    .text_size(crate::typography::ui_rems(11.5))
                    .text_color(theme.warning)
                    .child(
                        "Claude Code returns no stats headless; Zeron counts them from its Usage data on this device. Longest session is not recorded there.",
                    ),
            )
            .into_any_element()
    }
}

fn record_change(panel: &mut ConfigPanel, id: &'static str, before: &str, after: &str) {
    let original = panel
        .originals
        .entry(id)
        .or_insert_with(|| before.to_string())
        .clone();
    panel.changes.retain(|(c, _)| *c != id);
    if original != after {
        panel.changes.push((id, after.to_string()));
    }
}

fn summary_text(changes: &[(&'static str, String)]) -> Option<String> {
    if changes.is_empty() {
        return None;
    }
    let parts = changes
        .iter()
        .map(|(id, value)| {
            let label = cc::setting(id).map_or(*id, |s| s.label);
            let value = if value.is_empty() { "default" } else { value };
            format!("{label} → {value}")
        })
        .collect::<Vec<_>>();
    Some(format!("{SUMMARY_PREFIX}{}", parts.join(", ")))
}

pub(crate) fn summary_row(
    id: SharedString,
    text: &str,
    theme: &Theme,
    on_reopen: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let body = text.strip_prefix(SUMMARY_PREFIX).unwrap_or(text);
    let mut spans: Vec<AnyElement> =
        vec![div().flex_none().child(SUMMARY_PREFIX).into_any_element()];
    let parts: Vec<&str> = body.split(", ").collect();
    for (ix, part) in parts.iter().enumerate() {
        let (label, value) = part.split_once(" → ").unwrap_or((part, ""));
        let comma = if ix + 1 < parts.len() { "," } else { "" };
        spans.push(
            div()
                .flex_none()
                .flex()
                .flex_row()
                .when(ix > 0, |el| el.ml(px(4.0)))
                .child(
                    div()
                        .text_color(theme.text)
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(label.to_string()),
                )
                .child(if value.is_empty() {
                    comma.to_string()
                } else {
                    format!(" → {value}{comma}")
                })
                .into_any_element(),
        );
    }
    let button_id: SharedString = format!("{id}-reopen").into();
    div()
        .id(id)
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(10.0))
        .py(px(6.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(theme.border)
        .bg(crate::theme::ink(0.03))
        .text_size(crate::typography::ui_rems(12.5))
        .text_color(theme.text_muted)
        .child(
            icon(icons::SETTINGS)
                .size(px(14.0))
                .flex_none()
                .text_color(theme.text_muted),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_row()
                .flex_wrap()
                .children(spans),
        )
        .child(
            div()
                .id(button_id)
                .debug_selector(|| "claude-config-reopen".into())
                .role(gpui::Role::Button)
                .aria_label("Reopen Claude Code settings")
                .flex_none()
                .px(px(10.0))
                .h(px(24.0))
                .flex()
                .items_center()
                .rounded(px(7.0))
                .border_1()
                .border_color(theme.border)
                .cursor_pointer()
                .text_color(theme.text)
                .hover(|s| s.bg(theme.glass_hover()))
                .child("Reopen")
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    on_reopen(window, cx)
                }),
        )
        .into_any_element()
}

fn kbd(theme: &Theme, label: &str) -> gpui::Div {
    div()
        .flex_none()
        .px(px(4.0))
        .rounded(px(4.0))
        .border_1()
        .border_color(theme.border)
        .font_family(theme.font_mono.clone())
        .text_size(crate::typography::ui_rems(10.5))
        .text_color(theme.text_muted)
        .child(SharedString::from(label.to_string()))
}

fn empty_note(theme: &Theme, text: &str) -> AnyElement {
    div()
        .p(px(30.0))
        .flex()
        .justify_center()
        .text_size(crate::typography::ui_rems(12.5))
        .text_color(theme.text_faint)
        .child(SharedString::from(text.to_string()))
        .into_any_element()
}

fn highlight(text: &str, query: &str, theme: &Theme) -> AnyElement {
    let lower = text.to_lowercase();
    let start = (!query.is_empty())
        .then(|| lower.find(&query.to_lowercase()))
        .flatten()
        .filter(|start| {
            text.is_char_boundary(*start) && text.is_char_boundary(start + query.len())
        });
    match start {
        Some(start) => {
            let end = start + query.len();
            div()
                .flex()
                .flex_row()
                .child(text[..start].to_string())
                .child(
                    div()
                        .rounded(px(2.0))
                        .bg(theme.warning.opacity(0.3))
                        .child(text[start..end].to_string()),
                )
                .child(text[end..].to_string())
                .into_any_element()
        }
        None => div().child(text.to_string()).into_any_element(),
    }
}

fn kv_row(theme: &Theme, rule: bool, key: &str, value: String, tag: Option<&str>) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .gap(px(12.0))
        .px(px(14.0))
        .py(px(7.0))
        .when(rule, |el| {
            el.border_t_1().border_color(crate::theme::hairline(0.06))
        })
        .text_size(crate::typography::ui_rems(12.5))
        .child(
            div()
                .w(gpui::relative(0.34))
                .flex_none()
                .text_color(theme.text_muted)
                .child(key.to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(6.0))
                .font_family(theme.font_mono.clone())
                .text_size(crate::typography::ui_rems(12.0))
                .text_color(theme.text)
                .child(
                    div().min_w_0().flex().flex_col().children(
                        value
                            .lines()
                            .map(|line| div().truncate().child(line.to_string()))
                            .collect::<Vec<_>>(),
                    ),
                )
                .when_some(tag, |el, tag| {
                    el.child(
                        div()
                            .px(px(6.0))
                            .rounded(px(4.0))
                            .font_family(theme.font_sans.clone())
                            .text_size(crate::typography::ui_rems(10.0))
                            .bg(theme.accent.opacity(0.14))
                            .text_color(theme.accent)
                            .child(tag.to_string()),
                    )
                }),
        )
        .into_any_element()
}

fn meter(theme: &Theme, meter: Meter) -> AnyElement {
    let warn = meter.percent >= 80;
    let color = if warn { theme.warning } else { theme.accent };
    div()
        .mx(px(14.0))
        .mt(px(8.0))
        .mb(px(6.0))
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(
            div()
                .flex()
                .flex_row()
                .text_size(crate::typography::ui_rems(12.5))
                .child(meter.label)
                .child(
                    div()
                        .ml_auto()
                        .text_color(if warn {
                            theme.warning
                        } else {
                            theme.text_muted
                        })
                        .child(format!("{}%", meter.percent)),
                ),
        )
        .child(
            div()
                .h(px(6.0))
                .rounded(px(3.0))
                .bg(crate::theme::ink(0.06))
                .overflow_hidden()
                .child(
                    div()
                        .h_full()
                        .w(gpui::relative(meter.percent.min(100) as f32 / 100.0))
                        .rounded(px(3.0))
                        .bg(color),
                ),
        )
        .when_some(meter.resets, |el, resets| {
            el.child(
                div()
                    .text_size(crate::typography::ui_rems(11.5))
                    .text_color(theme.text_faint)
                    .child(format!("Resets {resets}")),
            )
        })
        .into_any_element()
}

fn heat(theme: &Theme, tokens: u64, max: u64) -> Hsla {
    if tokens == 0 {
        return crate::theme::ink(0.06);
    }
    let share = tokens as f32 / max as f32;
    let alpha = if share > 0.6 {
        1.0
    } else if share > 0.3 {
        0.7
    } else if share > 0.1 {
        0.45
    } else {
        0.25
    };
    theme.accent.opacity(alpha)
}

fn plural_days(n: u64) -> String {
    if n == 1 {
        "1 day".into()
    } else {
        format!("{n} days")
    }
}

fn short_id(id: &str) -> String {
    if id.chars().count() <= 12 {
        return id.to_string();
    }
    let head: String = id.chars().take(4).collect();
    let tail: String = id
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{head}…{tail}")
}

fn tilde(path: &str) -> String {
    match dirs_home() {
        Some(home) if path.starts_with(&home) => format!("~{}", &path[home.len()..]),
        _ => path.to_string(),
    }
}

fn dirs_home() -> Option<String> {
    std::env::var("HOME").ok().filter(|h| !h.is_empty())
}

fn short_files(files: &[String]) -> String {
    files
        .iter()
        .map(|f| {
            let path = std::path::Path::new(f);
            let parent = path
                .parent()
                .and_then(|p| p.file_name())
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            format!("{parent}/{name}")
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

fn mcp_summary(servers: Option<&serde_json::Value>) -> String {
    let servers = servers
        .and_then(|s| s.as_array())
        .cloned()
        .unwrap_or_default();
    if servers.is_empty() {
        return "none".into();
    }
    let connected = servers
        .iter()
        .filter(|s| s.get("status").and_then(|v| v.as_str()) == Some("connected"))
        .count();
    let failed = servers.len() - connected;
    let names = servers
        .iter()
        .filter_map(|s| s.get("name").and_then(|v| v.as_str()))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{connected} connected · {failed} failed ({names})")
}

fn memory_summary(paths: Option<&serde_json::Value>) -> String {
    match paths.and_then(|p| p.as_object()) {
        Some(map) if !map.is_empty() => map
            .iter()
            .map(|(kind, path)| format!("{kind}: {}", tilde(path.as_str().unwrap_or_default())))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => "none".into(),
    }
}

fn plugins_summary(plugins: Option<&serde_json::Value>) -> String {
    let plugins = plugins
        .and_then(|p| p.as_array())
        .cloned()
        .unwrap_or_default();
    if plugins.is_empty() {
        return "none".into();
    }
    let names = plugins
        .iter()
        .filter_map(|p| p.get("name").and_then(|v| v.as_str()))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{} enabled ({names})", plugins.len())
}

#[derive(Debug, PartialEq)]
struct Meter {
    label: String,
    percent: u32,
    resets: Option<String>,
}

#[derive(Debug, PartialEq)]
struct UsageSection {
    title: String,
    lines: Vec<String>,
}

#[derive(Debug, Default, PartialEq)]
struct UsageReport {
    meters: Vec<Meter>,
    sections: Vec<UsageSection>,
    note: Option<String>,
}

fn parse_usage(text: &str) -> UsageReport {
    let mut report = UsageReport::default();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((label, rest)) = line.split_once(": ")
            && let Some((pct, tail)) = rest.split_once("% used")
            && let Ok(percent) = pct.trim().parse::<u32>()
        {
            let resets = tail
                .trim_start_matches([' ', '·'])
                .strip_prefix("resets ")
                .map(str::to_owned);
            report.meters.push(Meter {
                label: label.to_string(),
                percent,
                resets,
            });
        } else if line.starts_with("Last ") {
            report.sections.push(UsageSection {
                title: line.to_string(),
                lines: Vec::new(),
            });
        } else if let Some(section) = report.sections.last_mut() {
            section
                .lines
                .push(line.split_whitespace().collect::<Vec<_>>().join(" "));
        } else if report.meters.is_empty() && report.note.is_none() {
            report.note = Some(line.to_string());
        }
    }
    report
}

#[derive(Debug, PartialEq)]
struct Stats {
    days: Vec<u64>,
    favorite_model: Option<String>,
    sessions: u64,
    active_days: u64,
    streak: u64,
    tokens: u64,
}

fn stats_of(summary: &UsageSummary) -> Option<Stats> {
    let claude: Vec<_> = summary
        .buckets
        .iter()
        .filter(|b| b.harness == HarnessId::ClaudeCode)
        .collect();
    if claude.is_empty() || summary.hourly {
        return None;
    }
    let mut days = vec![0u64; summary.periods.len()];
    let mut models: BTreeMap<&str, u64> = BTreeMap::new();
    for bucket in &claude {
        if let Some(day) = days.get_mut(bucket.period) {
            *day += bucket.tokens.total().max(1);
        }
        *models.entry(bucket.model.as_str()).or_default() += bucket.tokens.total();
    }
    let streak = days
        .iter()
        .rev()
        .skip_while(|t| **t == 0)
        .take_while(|t| **t > 0)
        .count() as u64;
    let streak = if days.last().is_some_and(|t| *t > 0)
        || days.iter().rev().nth(1).is_some_and(|t| *t > 0)
    {
        streak
    } else {
        0
    };
    Some(Stats {
        favorite_model: models
            .iter()
            .max_by_key(|(_, tokens)| **tokens)
            .map(|(model, _)| model.to_string()),
        sessions: summary
            .sessions
            .iter()
            .filter(|s| s.harness == HarnessId::ClaudeCode)
            .map(|s| s.sessions)
            .sum(),
        active_days: days.iter().filter(|t| **t > 0).count() as u64,
        streak,
        tokens: claude.iter().map(|b| b.tokens.total()).sum(),
        days,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_text_becomes_meters_and_contributor_blocks() {
        let text = "You are currently using your subscription to power your Claude Code usage\n\n\
Current session: 5% used · resets Oct 25 at 2:50am (Asia/Saigon)\n\
Current week (all models): 81% used · resets Oct 29 at 5am (Asia/Saigon)\n\
Current week (Fable): 2% used · resets Oct 29 at 5am (Asia/Saigon)\n\n\
What's contributing to your limits usage?\n\
Approximate · sessions on this machine only · shares overlap\n\n\
Last 24h · 2893 requests · 147 sessions\n\
  85%  Give \"general-purpose\" subagents a cheaper model\n\
  Top MCP servers: zeron 3%\n\n\
Last 7d · 67140 requests · 1484 sessions\n\
  90%  Give \"general-purpose\" subagents a cheaper model\n";
        let report = parse_usage(text);
        assert_eq!(report.meters.len(), 3);
        assert_eq!(
            report.meters[1],
            Meter {
                label: "Current week (all models)".into(),
                percent: 81,
                resets: Some("Oct 29 at 5am (Asia/Saigon)".into()),
            }
        );
        assert_eq!(report.sections.len(), 2);
        assert_eq!(
            report.sections[0].title,
            "Last 24h · 2893 requests · 147 sessions"
        );
        assert_eq!(
            report.sections[0].lines,
            [
                "85% Give \"general-purpose\" subagents a cheaper model",
                "Top MCP servers: zeron 3%"
            ]
        );
        assert!(report.note.unwrap().starts_with("You are currently using"));
    }

    #[test]
    fn summary_names_each_changed_row_and_stays_quiet_without_changes() {
        assert_eq!(summary_text(&[]), None);
        assert_eq!(
            summary_text(&[("theme", "dark".into()), ("autoCompact", "false".into())]).unwrap(),
            "Claude Code settings · Theme → dark, Auto-compact → false"
        );
        assert_eq!(
            summary_text(&[("language", String::new())]).unwrap(),
            "Claude Code settings · Language → default"
        );
    }
}
