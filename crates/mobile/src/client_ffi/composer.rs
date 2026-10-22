use serde_json::{Map, Value};
use zeron_client::model_picks;

use super::{ChatConfig, CoreClient, ModelInfo, PermissionMode};

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
