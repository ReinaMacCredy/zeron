use zeron_engine::registry::HarnessDescriptor;
pub use zeron_proto::review::*;
use zeron_proto::{HarnessId, Model};
use zeron_rpc::methods;

pub fn brand(
    options: &ReviewerOptions,
    choice: &ReviewerChoice,
    chat: Option<HarnessId>,
) -> (&'static str, Option<gpui::Hsla>) {
    let (harness, model) = choice_agent(options, choice, chat);
    crate::pickers::agent_brand_icon(harness, model.as_deref())
}

pub async fn review_settings(engine: &crate::state::EngineHandle) -> ReviewSettings {
    engine
        .client()
        .call_as::<ReviewSettings>(methods::GET_REVIEW_SETTINGS, serde_json::json!({}))
        .await
        .unwrap_or_default()
}

pub async fn list_models(engine: &crate::state::EngineHandle, harness: HarnessId) -> Vec<Model> {
    engine
        .client()
        .call(
            methods::LIST_MODELS,
            serde_json::json!({ "harness": harness, "force": false }),
        )
        .await
        .ok()
        .and_then(|value| serde_json::from_value::<Vec<Model>>(value["models"].clone()).ok())
        .unwrap_or_default()
}

pub async fn reviewer_options(engine: &crate::state::EngineHandle) -> ReviewerOptions {
    let descriptors = engine
        .client()
        .call_as::<Vec<HarnessDescriptor>>(methods::LIST_HARNESSES, serde_json::json!({}))
        .await
        .map(|list| crate::pickers::offered_harnesses(&list))
        .unwrap_or_default();
    let profiles = crate::agent_profiles::list(engine, None)
        .await
        .unwrap_or_default();
    ReviewerOptions {
        offered: descriptors.iter().map(|descriptor| descriptor.id).collect(),
        levels: descriptors
            .into_iter()
            .map(|descriptor| (descriptor.id, descriptor.reasoning_levels))
            .collect(),
        profiles,
    }
}

pub fn severity_colors(
    severity: Severity,
    theme: &crate::theme::Theme,
) -> (gpui::Hsla, gpui::Hsla) {
    match severity {
        Severity::Bug => (theme.danger, theme.danger.opacity(0.14)),
        Severity::Risk => (theme.danger.opacity(0.6), theme.danger.opacity(0.08)),
        Severity::Nit => (theme.text_muted, crate::theme::ink(0.08)),
    }
}

pub fn finding_card_height(body: &str) -> f32 {
    match split_finding(body).2 {
        "" => crate::comments::card_height("") - crate::comments::CARD_LINE_HEIGHT,
        detail => crate::comments::card_height(detail),
    }
}
