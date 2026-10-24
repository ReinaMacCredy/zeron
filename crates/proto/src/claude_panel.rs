use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use serde_json::Value;

use crate::claude_config::{
    self as cc, ClaudeConfigSnapshot, Kind, LocalCommandOutput, Scope, Setting,
};
use crate::usage::UsageSummary;
use crate::{HarnessId, PermissionMode, ReasoningLevel};

pub const SUMMARY_PREFIX: &str = "Claude Code settings · ";
pub const STATS_DAYS: u32 = 7 * 36;
pub const READ_ONLY_NOTE: &str =
    "Claude Code does not set this headless; change it in the terminal's /config.";
pub const UNWRITABLE_NOTE: &str = "Quotes and line breaks cannot be written.";
pub const STATUS_FOOTNOTE: &str =
    "From Claude Code's system/init for this folder; model and permission mode are this chat's.";
pub const STATS_FOOTNOTE: &str = "Claude Code returns no stats headless; Zeron counts them from its Usage data on this device. Longest session is not recorded there.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Status,
    Config,
    Usage,
    Stats,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Status, Tab::Config, Tab::Usage, Tab::Stats];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Status => "Status",
            Tab::Config => "Config",
            Tab::Usage => "Usage",
            Tab::Stats => "Stats",
        }
    }

    pub fn step(self, forward: bool) -> Tab {
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
pub enum Filter {
    All,
    Zeron,
    Terminal,
}

impl Filter {
    pub const ALL: [Filter; 3] = [Filter::All, Filter::Zeron, Filter::Terminal];

    pub fn label(self) -> &'static str {
        match self {
            Filter::All => "All",
            Filter::Zeron => "Affects Zeron chats",
            Filter::Terminal => "Terminal only",
        }
    }

    pub fn matches(self, setting: &Setting) -> bool {
        match self {
            Filter::All => true,
            Filter::Zeron => setting.scope == Scope::Zeron,
            Filter::Terminal => setting.scope == Scope::Terminal,
        }
    }

    pub fn count(self) -> usize {
        cc::SETTINGS.iter().filter(|s| self.matches(s)).count()
    }
}

pub enum Load<T> {
    Loading,
    Ready(T),
    Failed(String),
}

impl<T> Load<T> {
    pub fn from(result: Result<T, String>) -> Self {
        match result {
            Ok(value) => Load::Ready(value),
            Err(error) => Load::Failed(error),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ChipChoices {
    pub models: Vec<(String, String)>,
    pub model: Option<String>,
    pub efforts: Vec<ReasoningLevel>,
    pub effort: Option<ReasoningLevel>,
    pub permission: PermissionMode,
}

impl ChipChoices {
    pub fn model_label(&self) -> String {
        self.model
            .as_deref()
            .map(|id| {
                self.models
                    .iter()
                    .find(|(m, _)| m == id)
                    .map(|(_, label)| label.clone())
                    .unwrap_or_else(|| id.to_string())
            })
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug)]
pub struct RowView {
    pub setting: &'static Setting,
    pub value: String,
    pub options: Vec<String>,
    pub writable: bool,
    pub note: Option<String>,
}

impl RowView {
    pub fn is_bool(&self) -> bool {
        self.setting.kind == Kind::Bool
    }

    pub fn is_text(&self) -> bool {
        self.setting.kind == Kind::Text && self.setting.chip.is_none()
    }

    pub fn matches(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        let query = query.to_lowercase();
        [self.setting.label, self.setting.id, self.value.as_str()]
            .iter()
            .any(|field| field.to_lowercase().contains(&query))
    }

    pub fn tag(&self) -> Option<&'static str> {
        match (self.setting.chip, self.setting.scope) {
            (Some(_), _) => Some("= chip"),
            (None, Scope::Terminal) => Some("Terminal"),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Unchanged,
    Chip(cc::Chip, String),
    Queued,
    Refused,
}

pub struct ConfigState {
    pub values: Load<BTreeMap<String, String>>,
    pub files: Vec<String>,
    pub notes: HashMap<&'static str, String>,
    pub refused: HashSet<&'static str>,
    queue: VecDeque<(&'static str, String, String)>,
    pub writing: Option<&'static str>,
    originals: HashMap<&'static str, String>,
    pub changes: Vec<(&'static str, String)>,
}

impl Default for ConfigState {
    fn default() -> Self {
        Self {
            values: Load::Loading,
            files: Vec::new(),
            notes: HashMap::new(),
            refused: HashSet::new(),
            queue: VecDeque::new(),
            writing: None,
            originals: HashMap::new(),
            changes: Vec::new(),
        }
    }
}

impl ConfigState {
    pub fn snapshot_arrived(&mut self, reply: Result<ClaudeConfigSnapshot, String>) {
        match reply {
            Ok(snapshot) => {
                let mut values = snapshot.values;
                for (id, _, value) in &self.queue {
                    values.insert(id.to_string(), value.clone());
                }
                self.files = snapshot.files;
                self.values = Load::Ready(values);
            }
            Err(error) if !matches!(self.values, Load::Ready(_)) => {
                self.values = Load::Failed(error)
            }
            Err(_) => {}
        }
    }

    pub fn rows(&self, chips: &ChipChoices) -> Vec<RowView> {
        let values = match &self.values {
            Load::Ready(values) => Some(values),
            _ => None,
        };
        cc::SETTINGS
            .iter()
            .filter_map(|setting| {
                Some(match setting.chip {
                    Some(cc::Chip::Model) => RowView {
                        setting,
                        value: chips.model_label(),
                        options: chips.models.iter().map(|(_, l)| l.clone()).collect(),
                        writable: !chips.models.is_empty(),
                        note: None,
                    },
                    Some(cc::Chip::Effort) => RowView {
                        setting,
                        value: if chips.efforts.is_empty() {
                            "not offered by this model".into()
                        } else {
                            chips
                                .effort
                                .map(|l| l.label().to_string())
                                .unwrap_or_default()
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
                        let refused = self.refused.contains(setting.id);
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
                            note: self.notes.get(setting.id).cloned().or_else(|| {
                                setting.cli.is_none().then(|| READ_ONLY_NOTE.to_string())
                            }),
                        }
                    }
                })
            })
            .collect()
    }

    pub fn visible_rows(&self, chips: &ChipChoices, filter: Filter, query: &str) -> Vec<RowView> {
        self.rows(chips)
            .into_iter()
            .filter(|row| filter.matches(row.setting) && row.matches(query))
            .collect()
    }

    pub fn changed(&self, id: &str) -> bool {
        self.changes.iter().any(|(c, _)| *c == id)
    }

    pub fn record_change(&mut self, id: &'static str, before: &str, after: &str) {
        let original = self
            .originals
            .entry(id)
            .or_insert_with(|| before.to_string())
            .clone();
        self.changes.retain(|(c, _)| *c != id);
        if original != after {
            self.changes.push((id, after.to_string()));
        }
    }

    pub fn set_value(&mut self, row: &RowView, value: String) -> Change {
        if row.value == value {
            return Change::Unchanged;
        }
        let id = row.setting.id;
        if let Some(chip) = row.setting.chip {
            return Change::Chip(chip, value);
        }
        if cc::config_command(row.setting, &value).is_none() {
            self.notes.insert(id, UNWRITABLE_NOTE.into());
            return Change::Refused;
        }
        if let Load::Ready(values) = &mut self.values {
            values.insert(id.to_string(), value.clone());
        }
        self.notes.remove(id);
        self.queue.push_back((id, row.value.clone(), value));
        Change::Queued
    }

    // One /config at a time: each rewrites the whole file, so two at once can drop a change.
    pub fn next_write(&mut self) -> Option<String> {
        if self.writing.is_some() {
            return None;
        }
        loop {
            let (id, _, value) = self.queue.front().cloned()?;
            match cc::setting(id).and_then(|s| cc::config_command(s, &value)) {
                Some(command) => {
                    self.writing = Some(id);
                    return Some(command);
                }
                None => {
                    self.queue.pop_front();
                }
            }
        }
    }

    pub fn write_saved(reply: &Result<LocalCommandOutput, String>) -> bool {
        matches!(reply, Ok(out) if cc::config_saved(&out.text))
    }

    pub fn write_done(
        &mut self,
        reply: Result<LocalCommandOutput, String>,
        snapshot: Option<Result<ClaudeConfigSnapshot, String>>,
    ) {
        self.writing = None;
        let Some((id, before, value)) = self.queue.pop_front() else {
            return;
        };
        let saved = Self::write_saved(&reply);
        let confirmed = snapshot
            .as_ref()
            .and_then(|s| s.as_ref().ok())
            .and_then(|s| s.values.get(id))
            .is_none_or(|on_disk| *on_disk == value);
        let failure = match reply {
            Ok(_) if saved && confirmed => {
                self.record_change(id, &before, &value);
                None
            }
            Ok(out) => {
                if cc::config_key_unknown(&out.text) {
                    self.refused.insert(id);
                }
                Some(if saved {
                    "Claude Code reported it saved, but the file still holds the old value."
                        .to_string()
                } else {
                    out.text.trim().to_string()
                })
            }
            Err(error) => Some(error),
        };
        if let Some(note) = failure {
            self.notes.insert(id, note);
            if let Load::Ready(values) = &mut self.values {
                values.insert(id.to_string(), before);
            }
        }
        if let Some(snapshot) = snapshot {
            self.snapshot_arrived(snapshot);
        }
    }

    pub fn summary(&self) -> Option<String> {
        summary_text(&self.changes)
    }

    pub fn footer(&self) -> String {
        match (self.changes.len(), self.writing) {
            (_, Some(_)) => "Saving…".to_string(),
            (0, None) if self.files.is_empty() => "Claude Code defaults".to_string(),
            (0, None) => format!("Read from {}", short_files(&self.files)),
            (1, None) => "1 change, saved".to_string(),
            (n, None) => format!("{n} changes, saved"),
        }
    }
}

pub fn summary_text(changes: &[(&'static str, String)]) -> Option<String> {
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

pub fn summary_parts(text: &str) -> Option<Vec<(String, String)>> {
    let body = text.strip_prefix(SUMMARY_PREFIX)?;
    Some(
        body.split(", ")
            .map(|part| {
                let (label, value) = part.split_once(" → ").unwrap_or((part, ""));
                (label.to_string(), value.to_string())
            })
            .collect(),
    )
}

pub struct StatusRow {
    pub key: &'static str,
    pub value: String,
    pub chip: bool,
}

// Only the device that ran Claude Code knows the home its paths shorten against.
pub fn status_rows(
    status: &Load<LocalCommandOutput>,
    chips: &ChipChoices,
    session_id: Option<&str>,
    home: Option<&str>,
) -> Vec<StatusRow> {
    let model = chips
        .model
        .as_deref()
        .map(|id| format!("{id} · {}", chips.model_label()))
        .unwrap_or_else(|| "default".into());
    let session = session_id
        .map(short_id)
        .unwrap_or_else(|| "No session yet".into());
    let init = match status {
        Load::Ready(out) => out.init.clone(),
        _ => None,
    };
    let derived = init.as_ref().and_then(host_home);
    let home = home.or(derived.as_deref());
    let field = |key: &str| {
        init.as_ref()
            .and_then(|i| i.get(key))
            .and_then(|v| v.as_str())
            .map(str::to_owned)
    };
    let waiting = match status {
        Load::Loading => "Asking Claude Code…".to_string(),
        Load::Failed(error) => error.clone(),
        Load::Ready(_) => "Not reported".to_string(),
    };
    let or_wait = |value: Option<String>| value.unwrap_or_else(|| waiting.clone());
    let row = |key, value, chip| StatusRow { key, value, chip };
    vec![
        row("Version", or_wait(field("claude_code_version")), false),
        row("Session", session, false),
        row(
            "Folder",
            or_wait(field("cwd").map(|c| tilde(&c, home))),
            false,
        ),
        row("Model", model, true),
        row(
            "Permission mode",
            chips.permission.label().to_string(),
            true,
        ),
        row("Output style", or_wait(field("output_style")), false),
        row(
            "MCP servers",
            or_wait(init.as_ref().map(|i| mcp_summary(i.get("mcp_servers")))),
            false,
        ),
        row(
            "Memory",
            or_wait(
                init.as_ref()
                    .map(|i| memory_summary(i.get("memory_paths"), home)),
            ),
            false,
        ),
        row(
            "Plugins",
            or_wait(init.as_ref().map(|i| plugins_summary(i.get("plugins")))),
            false,
        ),
    ]
}

// The user memory file sits at `<home>/.claude/CLAUDE.md` on the host.
fn host_home(init: &Value) -> Option<String> {
    init.get("memory_paths")?
        .get("user")?
        .as_str()?
        .strip_suffix("/.claude/CLAUDE.md")
        .map(str::to_owned)
}

pub fn short_id(id: &str) -> String {
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

pub fn tilde(path: &str, home: Option<&str>) -> String {
    match home.filter(|h| !h.is_empty()) {
        Some(home) if path.starts_with(home) => format!("~{}", &path[home.len()..]),
        _ => path.to_string(),
    }
}

pub fn short_files(files: &[String]) -> String {
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

pub fn mcp_summary(servers: Option<&Value>) -> String {
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

pub fn memory_summary(paths: Option<&Value>, home: Option<&str>) -> String {
    match paths.and_then(|p| p.as_object()) {
        Some(map) if !map.is_empty() => map
            .iter()
            .map(|(kind, path)| {
                format!("{kind}: {}", tilde(path.as_str().unwrap_or_default(), home))
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => "none".into(),
    }
}

pub fn plugins_summary(plugins: Option<&Value>) -> String {
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

#[derive(Clone, Debug, PartialEq)]
pub struct Meter {
    pub label: String,
    pub percent: u32,
    pub resets: Option<String>,
}

impl Meter {
    pub fn warning(&self) -> bool {
        self.percent >= 80
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct UsageSection {
    pub title: String,
    pub lines: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UsageReport {
    pub meters: Vec<Meter>,
    pub sections: Vec<UsageSection>,
    pub note: Option<String>,
}

impl UsageReport {
    pub fn is_empty(&self) -> bool {
        self.meters.is_empty() && self.sections.is_empty()
    }
}

pub fn parse_usage(text: &str) -> UsageReport {
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

#[derive(Clone, Debug, PartialEq)]
pub struct Stats {
    pub days: Vec<u64>,
    pub favorite_model: Option<String>,
    pub sessions: u64,
    pub active_days: u64,
    pub streak: u64,
    pub tokens: u64,
}

pub fn stats_of(summary: &UsageSummary) -> Option<Stats> {
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

pub fn heat_level(tokens: u64, max: u64) -> u8 {
    if tokens == 0 {
        return 0;
    }
    let share = tokens as f32 / max.max(1) as f32;
    if share > 0.6 {
        4
    } else if share > 0.3 {
        3
    } else if share > 0.1 {
        2
    } else {
        1
    }
}

pub fn plural_days(n: u64) -> String {
    if n == 1 {
        "1 day".into()
    } else {
        format!("{n} days")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContextCategory {
    pub name: String,
    pub tokens: String,
    pub percent: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContextReport {
    pub model: Option<String>,
    pub tokens: Option<String>,
    pub categories: Vec<ContextCategory>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelReport {
    pub current: String,
    pub available: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct McpReport {
    pub total: u32,
    pub connected: u32,
    pub not_connected: u32,
    pub disabled: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CommandReply {
    Context(ContextReport),
    Usage(UsageReport),
    Model(ModelReport),
    Mcp(McpReport),
    TerminalOnly(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalEquivalent {
    SessionInfo,
    PermissionChip,
    ClaudeConfig,
    CopyTranscript,
}

pub fn terminal_equivalent(name: &str) -> Option<(&'static str, TerminalEquivalent)> {
    Some(match name {
        "status" => ("Open Session info", TerminalEquivalent::SessionInfo),
        "permissions" => ("Open permission chip", TerminalEquivalent::PermissionChip),
        "hooks" | "theme" => (
            "Open Claude Code settings",
            TerminalEquivalent::ClaudeConfig,
        ),
        "export" => ("Copy transcript", TerminalEquivalent::CopyTranscript),
        _ => return None,
    })
}

pub fn terminal_only_notice(name: &str) -> String {
    format!("/{name} only runs in Claude Code's terminal.")
}

// Claude Code answers a terminal-only command in headless mode with this one line.
fn terminal_only(reply: &str) -> Option<String> {
    let rest = reply.trim().strip_prefix('/')?;
    let (name, tail) = rest.split_once(' ')?;
    (tail.trim_end_matches('.') == "isn't available in this environment" && !name.is_empty())
        .then(|| name.to_string())
}

fn table_cells(line: &str) -> Option<Vec<String>> {
    let inner = line.trim().strip_prefix('|')?.strip_suffix('|')?;
    Some(inner.split('|').map(|c| c.trim().to_string()).collect())
}

pub fn parse_context(text: &str) -> Option<ContextReport> {
    let mut report = ContextReport {
        model: None,
        tokens: None,
        categories: Vec::new(),
    };
    let mut in_categories = false;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("**Model:**") {
            report.model = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("**Tokens:**") {
            report.tokens = Some(rest.trim().to_string());
        } else if line.starts_with('#') {
            in_categories = line.contains("by category");
        } else if in_categories
            && let Some(cells) = table_cells(line)
            && let [name, tokens, percent] = cells.as_slice()
            && let Ok(percent) = percent.trim_end_matches('%').parse::<f32>()
        {
            report.categories.push(ContextCategory {
                name: name.clone(),
                tokens: tokens.clone(),
                percent,
            });
        }
    }
    (report.tokens.is_some() || !report.categories.is_empty()).then_some(report)
}

pub fn parse_model(text: &str) -> Option<ModelReport> {
    let current = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("Current model:"))?
        .trim()
        .trim_matches('`')
        .to_string();
    let available = text
        .split_once("Available:")
        .map(|(_, rest)| {
            rest.split(',')
                .map(|a| a.trim().trim_end_matches('.').to_string())
                .filter(|a| !a.is_empty() && !a.starts_with("or "))
                .collect()
        })
        .unwrap_or_default();
    Some(ModelReport { current, available })
}

pub fn parse_mcp(text: &str) -> Option<McpReport> {
    let (total, rest) = text.trim().split_once(" MCP server(s):")?;
    let mut report = McpReport {
        total: total.trim().parse().ok()?,
        connected: 0,
        not_connected: 0,
        disabled: 0,
    };
    for part in rest.split('.').next().unwrap_or(rest).split(',') {
        let (n, label) = part.trim().split_once(' ')?;
        let n: u32 = n.parse().ok()?;
        match label {
            "connected" => report.connected = n,
            "not connected" => report.not_connected = n,
            "disabled" => report.disabled = n,
            _ => {}
        }
    }
    Some(report)
}

pub fn command_reply(command: &str, reply: &str) -> Option<CommandReply> {
    let (name, rest) = crate::invocation::leading_command(command)?;
    if !rest.is_empty() {
        return None;
    }
    if let Some(name) = terminal_only(reply) {
        return Some(CommandReply::TerminalOnly(name));
    }
    match name {
        "context" => parse_context(reply).map(CommandReply::Context),
        "usage" | "cost" | "stats" => Some(parse_usage(reply))
            .filter(|r| !r.is_empty())
            .map(CommandReply::Usage),
        "model" => parse_model(reply).map(CommandReply::Model),
        "mcp" => parse_mcp(reply).map(CommandReply::Mcp),
        _ => None,
    }
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
    fn command_replies_become_cards_and_terminal_only_commands_a_notice() {
        let context = "## Context Usage\n\n**Model:** claude-haiku-5-5  \n**Tokens:** 50k / 1m (5%)\n\n### Estimated usage by category\n\n| Category | Tokens | Percentage |\n|----------|--------|------------|\n| System prompt | 3.3k | 0.3% |\n| Free space | 917k | 91.7% |\n\n### MCP Tools\n\n| Tool | Server | Tokens |\n|------|--------|--------|\n| mcp__a__b | a | 147 |\n";
        let Some(CommandReply::Context(report)) = command_reply("/context", context) else {
            panic!("context card");
        };
        assert_eq!(report.model.as_deref(), Some("claude-haiku-5-5"));
        assert_eq!(report.tokens.as_deref(), Some("50k / 1m (5%)"));
        assert_eq!(report.categories.len(), 2);
        assert_eq!(report.categories[1].percent, 91.7);
        let mcp = "7 MCP server(s): 4 connected, 3 not connected, 0 disabled. Use `/mcp` in the terminal for details.";
        assert_eq!(
            command_reply("/mcp", mcp),
            Some(CommandReply::Mcp(McpReport {
                total: 7,
                connected: 4,
                not_connected: 3,
                disabled: 0
            }))
        );
        let model = "Current model: `Haiku 5.5`\nUsage: /model <name>. Available: sonnet, opus, haiku, opusplan, default, or a full model ID.";
        let Some(CommandReply::Model(report)) = command_reply("/model", model) else {
            panic!("model card");
        };
        assert_eq!(report.current, "Haiku 5.5");
        assert_eq!(
            report.available,
            ["sonnet", "opus", "haiku", "opusplan", "default"]
        );
        assert_eq!(
            command_reply(
                "/permissions",
                "/permissions isn't available in this environment."
            ),
            Some(CommandReply::TerminalOnly("permissions".into()))
        );
        assert_eq!(command_reply("/context please", context), None);
        assert_eq!(command_reply("/review", "Looks good."), None);
    }

    #[test]
    fn summary_names_each_changed_row_and_stays_quiet_without_changes() {
        assert_eq!(summary_text(&[]), None);
        let text =
            summary_text(&[("theme", "dark".into()), ("autoCompact", "false".into())]).unwrap();
        assert_eq!(
            text,
            "Claude Code settings · Theme → dark, Auto-compact → false"
        );
        assert_eq!(
            summary_parts(&text).unwrap(),
            [
                ("Theme".to_string(), "dark".to_string()),
                ("Auto-compact".to_string(), "false".to_string())
            ]
        );
        assert_eq!(
            summary_text(&[("language", String::new())]).unwrap(),
            "Claude Code settings · Language → default"
        );
    }
}
