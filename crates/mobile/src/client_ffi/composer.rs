use serde_json::{Map, Value};
use zeron_client::model_picks;

use super::{
    ChatConfig, CoreClient, CoreResult, ModelInfo, PermissionMode, SessionHandle, on_runtime,
};

#[derive(Debug, Clone, uniffi::Record)]
pub struct PermissionModeInfo {
    pub mode: PermissionMode,
    pub label: String,
    pub short_label: String,
    pub description: String,
}

#[uniffi::export]
pub fn permission_modes() -> Vec<PermissionModeInfo> {
    zeron_proto::PermissionMode::ALL
        .into_iter()
        .map(|mode| PermissionModeInfo {
            mode: mode.into(),
            label: mode.label().into(),
            short_label: mode.short_label().into(),
            description: mode.description().into(),
        })
        .collect()
}

#[uniffi::export]
pub fn pastes_as_file(pasted: String, draft: String) -> bool {
    zeron_proto::attachment_mentions::pastes_as_file(&pasted, &draft)
}

#[uniffi::export]
pub fn next_pasted_text_name(staged: Vec<String>) -> String {
    zeron_proto::attachment_mentions::next_pasted_text_name(staged.iter().map(String::as_str))
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct PastedTextPreview {
    pub title: String,
    pub subtitle: String,
}

#[uniffi::export]
pub fn pasted_text_preview(reference: String, text: String) -> PastedTextPreview {
    use zeron_proto::attachment_mentions::{
        pasted_text_index_from_name, pasted_text_lines_label, pasted_text_pill_label,
    };
    PastedTextPreview {
        title: pasted_text_index_from_name(&reference).map_or_else(
            || "Pasted text".to_owned(),
            |index| pasted_text_pill_label(index, None),
        ),
        subtitle: pasted_text_lines_label(text.lines().count()),
    }
}

#[uniffi::export]
pub fn follow_up_notice(queueing: bool, steer: bool, can_steer: bool) -> Option<String> {
    zeron_proto::steer_queues_notice(queueing, steer, can_steer).map(str::to_owned)
}

#[uniffi::export]
pub fn with_quotes(text: String, quotes: Vec<String>) -> String {
    let quotes: Vec<_> = quotes
        .into_iter()
        .map(zeron_proto::quotes::Quote::new)
        .collect();
    zeron_proto::quotes::with_quotes(&text, &quotes)
}

#[uniffi::export]
pub fn quote_chip_label(count: u32) -> String {
    zeron_proto::quotes::chip_label(count as usize)
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContextCard {
    pub long_context: bool,
    pub short_label: String,
    pub window_label: String,
    pub threshold_label: String,
    pub stops: Vec<String>,
    pub position: u32,
    pub warning: Option<String>,
}

fn options_of(model: &ModelInfo) -> Vec<zeron_proto::ModelOption> {
    model
        .options
        .iter()
        .map(|o| zeron_proto::ModelOption {
            id: o.id.clone(),
            label: o.label.clone(),
            choices: o
                .choices
                .iter()
                .map(|c| zeron_proto::ModelOptionChoice {
                    id: c.id.clone(),
                    label: c.label.clone(),
                })
                .collect(),
            default_choice: o.default_choice.clone(),
        })
        .collect()
}

fn picks_of(config: &ChatConfig) -> Map<String, Value> {
    config
        .model_options
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect()
}

fn repick(
    model: &ModelInfo,
    mut config: ChatConfig,
    change: impl FnOnce(&[zeron_proto::ModelOption], &mut Map<String, Value>),
) -> ChatConfig {
    let mut picks = picks_of(&config);
    change(&options_of(model), &mut picks);
    config.model_options = picks
        .into_iter()
        .map(|(k, v)| {
            let value = v.as_str().map_or_else(|| v.to_string(), str::to_owned);
            (k, value)
        })
        .collect();
    config
}

#[uniffi::export]
pub fn context_card(
    model: ModelInfo,
    config: ChatConfig,
    tokens: Option<u64>,
) -> Option<ContextCard> {
    let card = model_picks::context_card(&options_of(&model), &picks_of(&config), tokens)?;
    Some(ContextCard {
        long_context: card.long_context,
        short_label: card.short_label,
        window_label: card.window_label,
        threshold_label: card.threshold_label,
        stops: card.stops,
        position: card.position as u32,
        warning: card.warning,
    })
}

#[uniffi::export]
pub fn pick_context(model: ModelInfo, config: ChatConfig, position: u32) -> ChatConfig {
    repick(&model, config, |options, picks| {
        model_picks::pick_context(options, picks, position as usize)
    })
}

#[uniffi::export]
pub fn toggle_long_context(model: ModelInfo, config: ChatConfig) -> ChatConfig {
    repick(&model, config, model_picks::toggle_long_context)
}

#[uniffi::export]
pub fn speed_toggle(model: ModelInfo, config: ChatConfig) -> Option<bool> {
    model_picks::speed_toggle(&options_of(&model), &picks_of(&config))
}

#[uniffi::export]
pub fn toggle_speed(model: ModelInfo, config: ChatConfig) -> ChatConfig {
    repick(&model, config, model_picks::toggle_speed)
}

#[uniffi::export]
pub fn fast_mode_on(model: ModelInfo, config: ChatConfig) -> bool {
    zeron_proto::model_options::fast_mode_on(&options_of(&model), &picks_of(&config))
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ThreadMention {
    pub chat_id: String,
    pub title: String,
    pub link: String,
    pub status: Option<String>,
    pub project: String,
}

#[uniffi::export]
impl CoreClient {
    pub fn thread_mentions(
        &self,
        device_id: String,
        space_id: Option<String>,
        exclude: Option<String>,
        query: String,
    ) -> Vec<ThreadMention> {
        self.client
            .thread_mentions(&device_id, space_id.as_deref(), exclude.as_deref(), &query)
            .into_iter()
            .map(|t| ThreadMention {
                chat_id: t.chat_id,
                title: t.title,
                link: t.link,
                status: t.status,
                project: t.project,
            })
            .collect()
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ModelSwitchPrompt {
    pub title: String,
    pub lead: String,
    pub body: String,
    pub confirm: String,
    pub confirm_detail: String,
    pub cancel: String,
    pub cancel_detail: String,
    pub note: String,
    pub note_hint: String,
}

#[uniffi::export]
impl CoreClient {
    pub fn model_switch_prompt(
        &self,
        chat_id: String,
        to_model: String,
        from_label: String,
        to_label: String,
    ) -> Option<ModelSwitchPrompt> {
        let p = self
            .client
            .model_switch_prompt(&chat_id, &to_model, &from_label, &to_label)?;
        Some(ModelSwitchPrompt {
            title: p.title,
            lead: p.lead,
            body: p.body,
            confirm: p.confirm,
            confirm_detail: p.confirm_detail,
            cancel: p.cancel,
            cancel_detail: p.cancel_detail,
            note: p.note,
            note_hint: p.note_hint,
        })
    }

    pub async fn set_session_config_noted(
        &self,
        chat_id: String,
        config: ChatConfig,
        note: String,
        hint: String,
    ) -> CoreResult<()> {
        let config = config.over(self.client.session_config(&chat_id))?;
        let client = self.client.clone();
        on_runtime(async move {
            client
                .set_session_config_noted(&chat_id, &config, &note, &hint)
                .await
        })
        .await
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct StatsRow {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SessionStatsView {
    pub cost: Option<String>,
    pub priced: bool,
    pub cost_note: Option<String>,
    pub tokens: String,
    pub split: Vec<StatsRow>,
    pub rows: Vec<StatsRow>,
}

#[uniffi::export]
impl SessionHandle {
    pub fn stats_view(&self, now_ms: i64) -> Option<SessionStatsView> {
        use zeron_proto::stats_view::{Cache, cost_headline, cost_note, stats_rows, token_split};
        use zeron_proto::usage::format_tokens;
        let stats = self.inner.session_stats()?;
        let cache = Cache::of(&stats, self.inner.snapshot().working, now_ms);
        let row = |label: &str, value: String| StatsRow {
            label: label.to_owned(),
            value,
        };
        Some(SessionStatsView {
            cost: cost_headline(&stats),
            priced: stats.cost_usd().is_some(),
            cost_note: cost_note(&stats),
            tokens: format!("{} tokens", format_tokens(stats.total_tokens())),
            split: token_split(&stats)
                .into_iter()
                .map(|(label, n)| row(label, format_tokens(n)))
                .collect(),
            rows: stats_rows(&stats, cache)
                .into_iter()
                .map(|(label, value)| row(label, value))
                .collect(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CompactCommand {
    pub focus: Option<String>,
}

#[uniffi::export]
pub fn compact_command(text: String) -> Option<CompactCommand> {
    zeron_proto::app_commands::compact_focus(text.trim()).map(|focus| CompactCommand { focus })
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ResumeOffer {
    pub title: String,
    pub body: String,
}

#[uniffi::export]
impl SessionHandle {
    pub async fn compact(&self, focus: Option<String>) -> CoreResult<Option<String>> {
        let handle = self.inner.clone();
        on_runtime(async move { handle.compact(focus).await }).await
    }

    pub fn keep_full_history(&self) {
        self.inner.keep_full_history();
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct InvocationEntry {
    pub name: String,
    pub title: String,
    pub detail: String,
    pub skill: bool,
    pub insert: String,
    pub link: Option<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct InvocationCatalog {
    pub entries: Vec<InvocationEntry>,
    pub skill: bool,
    pub supported: bool,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct InvocationMatches {
    pub entries: Vec<InvocationEntry>,
    pub note: Option<String>,
}

fn invocation_entry(
    candidate: zeron_proto::command_catalog::InvocationCandidate,
    references: bool,
) -> InvocationEntry {
    use zeron_proto::command_catalog::{
        invocation_detail, invocation_insertion, skill_display_name, zeron_owned_command,
    };
    let skill = candidate.invocation.prefix() == '$';
    let insert = format!("{}{}", candidate.invocation.prefix(), candidate.name);
    let link = (!zeron_owned_command(&candidate.name))
        .then(|| invocation_insertion(&candidate.invocation, references))
        .filter(|link| *link != insert);
    InvocationEntry {
        title: if skill {
            skill_display_name(&candidate.name)
        } else {
            insert.clone()
        },
        detail: invocation_detail(&candidate.description, candidate.input_hint.as_deref()),
        name: candidate.name,
        skill,
        insert,
        link,
    }
}

#[uniffi::export]
impl CoreClient {
    pub async fn invocation_catalog(
        &self,
        device_id: String,
        harness: String,
        chat_id: Option<String>,
        space_id: Option<String>,
        skill: bool,
    ) -> InvocationCatalog {
        let client = self.client.clone();
        let found = on_runtime(async move {
            Ok(client
                .invocation_catalog(
                    &device_id,
                    &harness,
                    chat_id.as_deref(),
                    space_id.as_deref(),
                    skill,
                )
                .await)
        })
        .await;
        match found {
            Ok(found) => InvocationCatalog {
                entries: found
                    .entries
                    .into_iter()
                    .map(|c| invocation_entry(c, found.references))
                    .collect(),
                skill,
                supported: found.supported,
                warning: found.warning,
            },
            Err(_) => InvocationCatalog {
                entries: Vec::new(),
                skill,
                supported: true,
                warning: Some(
                    zeron_proto::command_catalog::catalog_error_message(
                        zeron_proto::command_catalog::CatalogFailure::Failed,
                        skill,
                    )
                    .into(),
                ),
            },
        }
    }
}

/// The phone keeps `$` skills out of the `/` menu, the desktop default.
#[uniffi::export]
pub fn filter_invocations(catalog: InvocationCatalog, query: String) -> InvocationMatches {
    let names: Vec<&str> = catalog.entries.iter().map(|e| e.name.as_str()).collect();
    let entries: Vec<InvocationEntry> = zeron_proto::view::filter_indices(&query, &names)
        .into_iter()
        .map(|ix| catalog.entries[ix].clone())
        .collect();
    let note = catalog.warning.clone().or_else(|| {
        entries.is_empty().then(|| {
            zeron_proto::command_catalog::catalog_empty_message(
                catalog.skill,
                catalog.supported,
                catalog.entries.is_empty(),
                true,
            )
            .into()
        })
    });
    InvocationMatches { entries, note }
}
