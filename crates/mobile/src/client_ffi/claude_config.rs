use std::sync::{Arc, Mutex};

use zeron_proto::claude_config::{self as cc, LocalCommandOutput};
use zeron_proto::claude_panel::{
    self as panel, Change, ChipChoices, ConfigState, Load, STATS_FOOTNOTE, STATUS_FOOTNOTE,
};
use zeron_proto::usage::{UsageSummary, format_tokens};

use super::{CoreClient, CoreResult, PermissionMode, on_runtime, zc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ConfigTab {
    Status,
    Config,
    Usage,
    Stats,
}

impl From<panel::Tab> for ConfigTab {
    fn from(tab: panel::Tab) -> Self {
        match tab {
            panel::Tab::Status => Self::Status,
            panel::Tab::Config => Self::Config,
            panel::Tab::Usage => Self::Usage,
            panel::Tab::Stats => Self::Stats,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ConfigFilter {
    All,
    Zeron,
    Terminal,
}

impl From<ConfigFilter> for panel::Filter {
    fn from(filter: ConfigFilter) -> Self {
        match filter {
            ConfigFilter::All => Self::All,
            ConfigFilter::Zeron => Self::Zeron,
            ConfigFilter::Terminal => Self::Terminal,
        }
    }
}

impl From<panel::Filter> for ConfigFilter {
    fn from(filter: panel::Filter) -> Self {
        match filter {
            panel::Filter::All => Self::All,
            panel::Filter::Zeron => Self::Zeron,
            panel::Filter::Terminal => Self::Terminal,
        }
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ConfigTabInfo {
    pub tab: ConfigTab,
    pub label: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ConfigFilterChip {
    pub filter: ConfigFilter,
    pub label: String,
    pub count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ConfigRowKind {
    Toggle,
    Cycle,
    Options,
    Text,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ConfigRowView {
    pub id: String,
    pub label: String,
    pub key: String,
    pub value: String,
    pub kind: ConfigRowKind,
    pub options: Vec<String>,
    pub next_value: Option<String>,
    pub writable: bool,
    pub note: Option<String>,
    pub note_pinned: bool,
    pub tag: Option<String>,
    pub chip: bool,
    pub changed: bool,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ConfigList {
    pub message: Option<String>,
    pub rows: Vec<ConfigRowView>,
    pub shown: String,
    pub footer: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ChipWrite {
    pub model: Option<String>,
    pub model_label: Option<String>,
    pub reasoning: Option<String>,
    pub permission: Option<PermissionMode>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ConfigStatusRow {
    pub key: String,
    pub value: String,
    pub chip: bool,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ConfigStatus {
    pub rows: Vec<ConfigStatusRow>,
    pub footnote: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct UsageMeterView {
    pub label: String,
    pub percent: u32,
    pub resets: Option<String>,
    pub warning: bool,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct UsageSectionView {
    pub title: String,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct UsageReportView {
    pub message: Option<String>,
    pub meters: Vec<UsageMeterView>,
    pub sections: Vec<UsageSectionView>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct StatTile {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ConfigStats {
    pub message: Option<String>,
    pub caption: String,
    pub heat: Vec<u8>,
    pub tiles: Vec<StatTile>,
    pub footnote: String,
}

pub(crate) fn usage_view(report: panel::UsageReport, sections_lines: usize) -> UsageReportView {
    UsageReportView {
        message: None,
        meters: report
            .meters
            .iter()
            .map(|m| UsageMeterView {
                label: m.label.clone(),
                percent: m.percent,
                resets: m.resets.as_ref().map(|r| format!("Resets {r}")),
                warning: m.warning(),
            })
            .collect(),
        sections: report
            .sections
            .into_iter()
            .map(|s| UsageSectionView {
                title: s.title,
                lines: s.lines.into_iter().take(sections_lines).collect(),
            })
            .collect(),
        note: report.note,
    }
}

struct Inner {
    config: ConfigState,
    chips: ChipChoices,
    status: Load<LocalCommandOutput>,
    stats: Option<Load<UsageSummary>>,
}

#[derive(uniffi::Object)]
pub struct ClaudeConfigPanel {
    client: zc::Client,
    chat_id: String,
    session_id: Option<String>,
    inner: Mutex<Inner>,
}

#[uniffi::export]
impl CoreClient {
    pub fn claude_config_panel(&self, chat_id: String) -> Arc<ClaudeConfigPanel> {
        let session_id = self
            .client
            .chat(&chat_id)
            .and_then(|chat| chat.harness_session_id);
        Arc::new(ClaudeConfigPanel {
            client: self.client.clone(),
            chat_id,
            session_id,
            inner: Mutex::new(Inner {
                config: ConfigState::default(),
                chips: ChipChoices::default(),
                status: Load::Loading,
                stats: None,
            }),
        })
    }
}

fn card_command(
    text: &str,
    skills: &[String],
) -> Option<zeron_proto::app_commands::ClaudeCardCommand> {
    zeron_proto::app_commands::claude_card_command(text, |name| skills.iter().any(|s| s == name))
}

#[uniffi::export]
pub fn is_claude_card_command(text: String, skills: Vec<String>) -> bool {
    card_command(&text, &skills).is_some()
}

#[uniffi::export]
impl CoreClient {
    pub async fn run_claude_card(
        &self,
        chat_id: String,
        text: String,
        skills: Vec<String>,
    ) -> CoreResult<Option<ChipWrite>> {
        let Some(command) = card_command(&text, &skills) else {
            return Ok(None);
        };
        let client = self.client.clone();
        let switch =
            on_runtime(async move { client.run_claude_card(&chat_id, command).await }).await?;
        Ok(switch.map(|(model, label)| ChipWrite {
            model: Some(model),
            model_label: Some(label),
            reasoning: None,
            permission: None,
        }))
    }
}

#[uniffi::export]
pub fn opens_config_panel(text: String) -> bool {
    zeron_proto::app_commands::opens_config_panel(text.trim())
}

impl ClaudeConfigPanel {
    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    async fn drain(&self) {
        loop {
            let Some(command) = self.inner().config.next_write() else {
                return;
            };
            let client = self.client.clone();
            let chat = self.chat_id.clone();
            let (reply, snapshot) = zc::runtime::run(async move {
                let reply = client
                    .run_claude_local_command(&chat, &command, false, false)
                    .await
                    .map_err(|e| e.to_string());
                let snapshot = if ConfigState::write_saved(&reply) {
                    Some(
                        client
                            .read_claude_config(&chat)
                            .await
                            .map_err(|e| e.to_string()),
                    )
                } else {
                    None
                };
                Ok::<_, zc::ClientError>((reply, snapshot))
            })
            .await
            .unwrap_or_else(|e| (Err(e.to_string()), None));
            self.inner().config.write_done(reply, snapshot);
        }
    }
}

#[uniffi::export]
impl ClaudeConfigPanel {
    pub fn tabs(&self) -> Vec<ConfigTabInfo> {
        panel::Tab::ALL
            .into_iter()
            .map(|tab| ConfigTabInfo {
                tab: tab.into(),
                label: tab.label().into(),
            })
            .collect()
    }

    pub fn filters(&self) -> Vec<ConfigFilterChip> {
        panel::Filter::ALL
            .into_iter()
            .map(|filter| ConfigFilterChip {
                filter: filter.into(),
                label: filter.label().into(),
                count: filter.count() as u32,
            })
            .collect()
    }

    pub async fn load(&self) {
        let client = self.client.clone();
        let chat = self.chat_id.clone();
        let (snapshot, chips, status) = zc::runtime::run(async move {
            Ok::<_, zc::ClientError>(client.open_claude_panel(&chat).await)
        })
        .await
        .expect("infallible");
        let mut inner = self.inner();
        inner
            .config
            .snapshot_arrived(snapshot.map_err(|e| e.to_string()));
        inner.chips = chips;
        inner.status = Load::from(status.map_err(|e| e.to_string()));
    }

    pub async fn refresh_chips(&self) {
        let client = self.client.clone();
        let chat = self.chat_id.clone();
        if let Ok(chips) =
            zc::runtime::run(
                async move { Ok::<_, zc::ClientError>(client.chip_choices(&chat).await) },
            )
            .await
        {
            self.inner().chips = chips;
        }
    }

    pub async fn load_stats(&self) {
        if self.inner().stats.is_some() {
            return;
        }
        self.inner().stats = Some(Load::Loading);
        let client = self.client.clone();
        let chat = self.chat_id.clone();
        let reply = zc::runtime::run(async move { client.claude_stats_summary(&chat).await })
            .await
            .map_err(|e| e.to_string());
        self.inner().stats = Some(Load::from(reply));
    }

    pub fn list(&self, filter: ConfigFilter, query: String) -> ConfigList {
        let inner = self.inner();
        let query = query.trim().to_string();
        let rows = inner
            .config
            .visible_rows(&inner.chips, filter.into(), &query);
        let message = match &inner.config.values {
            Load::Loading => Some("Reading Claude Code's settings…".to_string()),
            Load::Failed(error) => Some(format!("Could not read settings: {error}")),
            Load::Ready(_) if rows.is_empty() => Some(format!("No settings match “{query}”")),
            Load::Ready(_) => None,
        };
        ConfigList {
            message,
            shown: format!("{} shown", rows.len()),
            footer: inner.config.footer(),
            rows: rows
                .into_iter()
                .map(|row| {
                    let kind = if row.is_bool() {
                        ConfigRowKind::Toggle
                    } else if row.is_text() {
                        ConfigRowKind::Text
                    } else if row.options.len() == 2 {
                        ConfigRowKind::Cycle
                    } else {
                        ConfigRowKind::Options
                    };
                    let next_value = (row.options.len() == 2).then(|| {
                        let current = row.options.iter().position(|o| *o == row.value);
                        row.options[(current.unwrap_or(0) + 1) % 2].clone()
                    });
                    ConfigRowView {
                        id: row.setting.id.into(),
                        label: row.setting.label.into(),
                        key: row.setting.id.into(),
                        changed: inner.config.changed(row.setting.id),
                        note_pinned: inner.config.notes.contains_key(row.setting.id),
                        chip: row.setting.chip.is_some(),
                        tag: row.tag().map(Into::into),
                        kind,
                        next_value,
                        writable: row.writable,
                        note: row.note,
                        options: row.options,
                        value: row.value,
                    }
                })
                .collect(),
        }
    }

    pub async fn change(&self, id: String, value: String) -> Option<ChipWrite> {
        let chip = {
            let mut inner = self.inner();
            let row = inner
                .config
                .rows(&inner.chips)
                .into_iter()
                .find(|row| row.setting.id == id)?;
            match inner.config.set_value(&row, value) {
                Change::Chip(chip, value) => {
                    inner
                        .config
                        .record_change(row.setting.id, &row.value, &value);
                    let chips = &inner.chips;
                    Some(match chip {
                        cc::Chip::Model => {
                            let (model, label) =
                                chips.models.iter().find(|(_, l)| *l == value)?.clone();
                            ChipWrite {
                                model: Some(model),
                                model_label: Some(label),
                                reasoning: None,
                                permission: None,
                            }
                        }
                        cc::Chip::Effort => ChipWrite {
                            model: None,
                            model_label: None,
                            reasoning: chips
                                .efforts
                                .iter()
                                .find(|l| l.label() == value)
                                .and_then(|l| serde_json::to_value(l).ok())
                                .and_then(|v| v.as_str().map(str::to_owned)),
                            permission: None,
                        },
                        cc::Chip::Permission => ChipWrite {
                            model: None,
                            model_label: None,
                            reasoning: None,
                            permission: zeron_proto::PermissionMode::ALL
                                .into_iter()
                                .find(|m| m.label() == value)
                                .map(Into::into),
                        },
                    })
                }
                Change::Queued => None,
                Change::Unchanged | Change::Refused => return None,
            }
        };
        if chip.is_none() {
            self.drain().await;
        }
        chip
    }

    pub fn status(&self) -> ConfigStatus {
        let inner = self.inner();
        ConfigStatus {
            rows: panel::status_rows(
                &inner.status,
                &inner.chips,
                self.session_id.as_deref(),
                None,
            )
            .into_iter()
            .map(|row| ConfigStatusRow {
                key: row.key.into(),
                value: row.value,
                chip: row.chip,
            })
            .collect(),
            footnote: STATUS_FOOTNOTE.into(),
        }
    }

    pub fn usage(&self) -> UsageReportView {
        let inner = self.inner();
        let empty = |message: String| UsageReportView {
            message: Some(message),
            meters: Vec::new(),
            sections: Vec::new(),
            note: None,
        };
        match &inner.status {
            Load::Loading => empty("Asking Claude Code for /usage…".into()),
            Load::Failed(error) => empty(error.clone()),
            Load::Ready(out) => {
                let report = panel::parse_usage(&out.text);
                if report.is_empty() {
                    empty(out.text.trim().to_string())
                } else {
                    usage_view(report, 3)
                }
            }
        }
    }

    pub fn stats(&self) -> ConfigStats {
        let inner = self.inner();
        let message = |text: String| ConfigStats {
            message: Some(text),
            caption: String::new(),
            heat: Vec::new(),
            tiles: Vec::new(),
            footnote: STATS_FOOTNOTE.into(),
        };
        match &inner.stats {
            None | Some(Load::Loading) => message("Counting Claude Code sessions…".into()),
            Some(Load::Failed(error)) => message(error.clone()),
            Some(Load::Ready(summary)) => match panel::stats_of(summary) {
                None => {
                    message("No Claude Code activity on this device in the last 36 weeks.".into())
                }
                Some(stats) => {
                    let max = stats.days.iter().copied().max().unwrap_or(0);
                    let favorite = stats
                        .favorite_model
                        .as_deref()
                        .map(|id| {
                            inner
                                .chips
                                .models
                                .iter()
                                .find(|(m, _)| m == id)
                                .map_or_else(|| id.to_string(), |(_, label)| label.clone())
                        })
                        .unwrap_or_else(|| "No data".into());
                    let tile = |value: String, label: &str| StatTile {
                        value,
                        label: label.into(),
                    };
                    ConfigStats {
                        message: None,
                        caption: "Activity · last 36 weeks".into(),
                        heat: stats
                            .days
                            .iter()
                            .map(|t| panel::heat_level(*t, max))
                            .collect(),
                        tiles: vec![
                            tile(favorite, "Favorite model"),
                            tile(stats.sessions.to_string(), "Sessions"),
                            tile(stats.active_days.to_string(), "Active days"),
                            tile(panel::plural_days(stats.streak), "Current streak"),
                            tile("No data".into(), "Longest session"),
                            tile(format_tokens(stats.tokens), "Tokens"),
                        ],
                        footnote: STATS_FOOTNOTE.into(),
                    }
                }
            },
        }
    }

    pub async fn close(&self) -> CoreResult<Option<String>> {
        let Some(summary) = self.inner().config.summary() else {
            return Ok(None);
        };
        let client = self.client.clone();
        let chat = self.chat_id.clone();
        let text = summary.clone();
        on_runtime(async move { client.add_chat_note(&chat, &text, None).await }).await?;
        Ok(Some(summary))
    }
}
