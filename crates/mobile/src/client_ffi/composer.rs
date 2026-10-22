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

impl From<zeron_proto::model_switch::ModelSwitchPrompt> for ModelSwitchPrompt {
    fn from(p: zeron_proto::model_switch::ModelSwitchPrompt) -> Self {
        Self {
            title: p.title,
            lead: p.lead,
            body: p.body,
            confirm: p.confirm,
            confirm_detail: p.confirm_detail,
            cancel: p.cancel,
            cancel_detail: p.cancel_detail,
            note: p.note,
            note_hint: p.note_hint,
        }
    }
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
        self.client
            .model_switch_prompt(&chat_id, &to_model, &from_label, &to_label)
            .map(Into::into)
    }

    pub fn provider_switch_prompt(
        &self,
        chat_id: String,
        from_provider: String,
        to_provider: String,
        from_model: String,
        to_model: String,
    ) -> Option<ModelSwitchPrompt> {
        self.client
            .provider_switch_prompt(
                &chat_id,
                &from_provider,
                &to_provider,
                &from_model,
                &to_model,
            )
            .map(Into::into)
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
pub struct CompactSectionView {
    pub heading: String,
    pub note: Option<String>,
    pub rows: Vec<StatsRow>,
}

#[uniffi::export]
impl SessionHandle {
    pub fn compact_section(&self, now_ms: i64) -> Option<CompactSectionView> {
        let section = self.inner.compact_section(now_ms)?;
        Some(CompactSectionView {
            heading: section.heading.into(),
            note: section.note.map(Into::into),
            rows: section
                .rows
                .into_iter()
                .map(|(label, value)| StatsRow {
                    label: label.into(),
                    value,
                })
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

/// The phone draws only the empty-draft prediction; the desktop's inline
/// remainder needs a Tab key.
#[uniffi::export]
pub fn prediction_placeholder(prediction: Option<String>, text: String) -> Option<String> {
    zeron_proto::prediction::prediction_display(prediction.as_deref(), &text)
        .0
        .map(Into::into)
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HandoffTarget {
    pub harness: String,
    pub name: String,
    pub model: String,
    pub model_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HandoffLabels {
    pub title: String,
    pub this_thread: String,
    pub new_thread: String,
    pub project: String,
}

#[uniffi::export]
pub fn handoff_labels() -> HandoffLabels {
    use zeron_proto::handoff::*;
    HandoffLabels {
        title: HAND_OFF_THREAD.into(),
        this_thread: CONTINUE_IN_THIS_THREAD.into(),
        new_thread: CONTINUE_IN_A_NEW_THREAD.into(),
        project: HAND_OFF_TO_PROJECT.into(),
    }
}

#[uniffi::export]
pub fn handoff_failed_message(err: String) -> String {
    zeron_proto::handoff::handoff_failed_message(&err)
}

#[uniffi::export]
impl CoreClient {
    pub async fn handoff_targets(&self, device_id: String) -> Vec<HandoffTarget> {
        let client = self.client.clone();
        on_runtime(async move { Ok(client.handoff_targets(&device_id).await) })
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|t| HandoffTarget {
                harness: t.harness,
                name: t.name,
                model: t.model,
                model_label: t.model_label,
            })
            .collect()
    }

    pub async fn continue_in_new_thread(
        &self,
        source_chat_id: String,
        harness: Option<String>,
        model: Option<String>,
        target_space_id: Option<String>,
    ) -> CoreResult<String> {
        let client = self.client.clone();
        on_runtime(async move {
            let target = harness.as_deref().zip(model.as_deref());
            client
                .continue_in_new_thread(&source_chat_id, target, target_space_id.as_deref())
                .await
        })
        .await
    }
}

#[uniffi::export]
impl SessionHandle {
    pub fn handoff_blocked(&self) -> bool {
        self.inner.handoff_blocked()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SidechatCommand {
    pub prompt: Option<String>,
}

#[uniffi::export]
pub fn sidechat_command(text: String) -> Option<SidechatCommand> {
    zeron_proto::app_commands::sidechat_prompt(text.trim()).map(|prompt| SidechatCommand { prompt })
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SidechatRow {
    pub chat_id: String,
    pub title: String,
    pub time_left: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SidechatSheet {
    pub title: String,
    pub context: String,
    pub section: Option<String>,
    pub rows: Vec<SidechatRow>,
}

#[uniffi::export]
impl CoreClient {
    pub fn sidechat_sheet(&self, source_chat_id: String, source_title: String) -> SidechatSheet {
        let rows: Vec<SidechatRow> = self
            .client
            .sidechats_of(&source_chat_id)
            .into_iter()
            .map(|row| SidechatRow {
                chat_id: row.chat_id,
                title: row.title,
                time_left: row.time_left,
            })
            .collect();
        SidechatSheet {
            title: zeron_proto::sidechat::NEW_SIDECHAT.into(),
            context: zeron_proto::sidechat::sidechat_context_line(&source_title),
            section: (!rows.is_empty())
                .then(|| zeron_proto::sidechat::sidechat_tab_label(rows.len())),
            rows,
        }
    }

    pub async fn open_sidechat(
        &self,
        source_chat_id: String,
        prompt: Option<String>,
    ) -> CoreResult<String> {
        let client = self.client.clone();
        on_runtime(async move { client.open_sidechat(&source_chat_id, prompt).await }).await
    }

    pub async fn create_sidechat(&self, source_chat_id: String) -> CoreResult<String> {
        let client = self.client.clone();
        on_runtime(async move { client.create_sidechat(&source_chat_id).await }).await
    }
}

#[uniffi::export]
pub fn new_sidechat_label() -> String {
    zeron_proto::sidechat::NEW_SIDECHAT.into()
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct AgentRunRow {
    pub task_id: String,
    pub title: String,
    pub line: String,
    pub working: bool,
    pub can_stop: bool,
    pub delegated: bool,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ShellRow {
    pub task_id: String,
    pub label: String,
    pub line: String,
    pub can_stop: bool,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct BackgroundRosterView {
    pub title: String,
    pub summary: Option<String>,
    pub agents_section: String,
    pub agents: Vec<AgentRunRow>,
    pub shells_section: String,
    pub shells: Vec<ShellRow>,
}

#[uniffi::export]
impl SessionHandle {
    pub fn background_roster(&self, now_ms: i64) -> BackgroundRosterView {
        use zeron_proto::background::{
            roster_summary, shell_line, shells_section, stoppable, subagents_section,
        };
        let roster = self.inner.background_roster();
        let stoppable = stoppable(roster.harness);
        BackgroundRosterView {
            title: "Agents".into(),
            summary: roster_summary(&roster.agents, roster.shells.len()),
            agents_section: subagents_section(&roster.agents),
            agents: roster
                .agents
                .iter()
                .map(|run| AgentRunRow {
                    task_id: run.part_id.clone(),
                    title: run.title.clone(),
                    line: run.line(roster.harness, now_ms),
                    working: run.state == zeron_proto::background::RunState::Working,
                    can_stop: run.can_stop(stoppable),
                    delegated: run.delegated,
                })
                .collect(),
            shells_section: shells_section(roster.shells.len()),
            shells: roster
                .shells
                .iter()
                .map(|task| ShellRow {
                    task_id: task.id.clone(),
                    label: task.label.clone(),
                    line: shell_line(task, now_ms),
                    can_stop: stoppable,
                })
                .collect(),
        }
    }

    pub fn shell_output(&self, task_id: String) -> Vec<String> {
        self.inner.shell_output(&task_id)
    }

    pub fn watch_shell_output(&self, task_id: Option<String>) {
        self.inner.watch_shell_output(task_id);
    }

    pub async fn stop_background_run(&self, task_id: String, delegated: bool) -> CoreResult<()> {
        let handle = self.inner.clone();
        on_runtime(async move { handle.stop_background_run(&task_id, delegated).await }).await
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ProviderInstanceRow {
    pub id: String,
    pub harness: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct InstanceModels {
    pub models: Vec<ModelInfo>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ProfileRow {
    pub id: String,
    pub name: String,
    pub line: String,
    pub active: bool,
    pub drift: Option<String>,
    pub savable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ProfileMenuLabels {
    pub title: String,
    pub none: String,
    pub none_note: String,
    pub edited: String,
    pub revert: String,
    pub save: String,
    pub retry: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MissionComposerLabels {
    pub start: String,
    pub review_plan_first: String,
    pub plan_doc: String,
    pub steps_only: String,
    pub placeholder: String,
}

#[uniffi::export]
pub fn mission_composer_labels(from_chat: bool) -> MissionComposerLabels {
    use zeron_proto::mission as m;
    MissionComposerLabels {
        start: m::START_MISSION.into(),
        review_plan_first: m::REVIEW_PLAN_FIRST.into(),
        plan_doc: m::PLAN_DOC.into(),
        steps_only: m::STEPS_ONLY.into(),
        placeholder: m::mission_placeholder(from_chat).into(),
    }
}

#[uniffi::export]
pub fn mission_staged_error(images: u32, other_file: Option<String>) -> Option<String> {
    zeron_proto::mission::mission_staged_error(
        images as usize,
        zeron_doc::TASK_IMAGES_MAX,
        other_file.as_deref(),
    )
}

#[uniffi::export]
pub fn mission_text(text: String, pasted: Vec<String>) -> String {
    zeron_proto::mission::mission_text(&text, &pasted)
}

#[uniffi::export]
pub fn profile_menu_labels() -> ProfileMenuLabels {
    use zeron_proto::picker as p;
    ProfileMenuLabels {
        title: p::PROFILES.into(),
        none: p::NO_PROFILE.into(),
        none_note: p::NO_PROFILE_NOTE.into(),
        edited: p::EDITED.into(),
        revert: p::REVERT.into(),
        save: p::SAVE.into(),
        retry: p::RETRY.into(),
    }
}

#[uniffi::export]
impl CoreClient {
    pub async fn list_provider_instances(&self, device_id: String) -> Vec<ProviderInstanceRow> {
        let client = self.client.clone();
        on_runtime(async move { client.list_provider_instances(&device_id).await })
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|instance| ProviderInstanceRow {
                id: instance.id,
                harness: zeron_client::model_menu::harness_wire(instance.driver),
                label: instance.label,
            })
            .collect()
    }

    pub async fn list_instance_models(
        &self,
        device_id: String,
        harness: String,
        instance_id: String,
        force: bool,
    ) -> InstanceModels {
        use zeron_client::model_menu::InstanceCatalog;
        let client = self.client.clone();
        let catalog = on_runtime(async move {
            Ok(client
                .list_instance_models(&device_id, &harness, &instance_id, force)
                .await)
        })
        .await
        .unwrap_or_else(|err| InstanceCatalog::Failed(err.to_string()));
        match catalog {
            InstanceCatalog::Ready(models) => InstanceModels {
                models: models.into_iter().map(Into::into).collect(),
                error: None,
            },
            InstanceCatalog::Failed(error) => InstanceModels {
                models: Vec::new(),
                error: Some(error),
            },
        }
    }

    pub async fn profile_rows(&self, chat_id: String) -> Vec<ProfileRow> {
        let client = self.client.clone();
        on_runtime(async move { client.profile_rows(&chat_id).await })
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|row| ProfileRow {
                id: row.id,
                name: row.name,
                line: row.line,
                active: row.active,
                drift: row.drift,
                savable: row.savable,
            })
            .collect()
    }

    pub async fn profile_switch_prompt(
        &self,
        chat_id: String,
        profile_id: String,
    ) -> CoreResult<Option<ModelSwitchPrompt>> {
        let client = self.client.clone();
        let prompt =
            on_runtime(async move { client.profile_switch_prompt(&chat_id, &profile_id).await })
                .await?;
        Ok(prompt.map(Into::into))
    }

    pub async fn apply_agent_profile(
        &self,
        chat_id: String,
        profile_id: String,
        note: Option<String>,
        hint: Option<String>,
    ) -> CoreResult<()> {
        let client = self.client.clone();
        on_runtime(async move {
            client
                .apply_agent_profile(&chat_id, &profile_id, note.zip(hint))
                .await
        })
        .await
    }

    pub fn detach_agent_profile(&self, chat_id: String) -> CoreResult<()> {
        Ok(self.client.detach_agent_profile(&chat_id)?)
    }

    pub async fn save_chat_into_profile(
        &self,
        chat_id: String,
        profile_id: String,
    ) -> CoreResult<()> {
        let client = self.client.clone();
        on_runtime(async move { client.save_chat_into_profile(&chat_id, &profile_id).await }).await
    }
}
