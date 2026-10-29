use gpui::{
    App, Div, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    Stateful, Styled as _, div, px,
};
use serde_json::{Map, Value, json};
use zeron_proto::{AgentProfile, AgentProfileDraft, AgentProfilesSnapshot, HarnessId, Model};
use zeron_rpc::methods;

use crate::state::EngineHandle;

pub fn brand_badge(harness: HarnessId, size: f32, fallback: Hsla) -> impl IntoElement {
    let (path, tint) = crate::pickers::harness_brand_icon(harness);
    div()
        .flex_none()
        .size(px(size))
        .rounded(px((size * 0.28).round()))
        .bg(crate::theme::ink(0.06))
        .flex()
        .items_center()
        .justify_center()
        .child(
            crate::icons::icon(path)
                .size(px((size * 0.58).round()))
                .text_color(tint.unwrap_or(fallback)),
        )
}

pub fn summary(profile: &AgentProfile, harness_name: &str, model_label: Option<&str>) -> String {
    zeron_proto::profile_line(
        harness_name,
        model_label.or(profile.model.as_deref()),
        profile.access,
        profile.reasoning,
        profile.disabled_tools.len(),
    )
}

pub fn draft_summary(
    draft: &AgentProfileDraft,
    harness_name: &str,
    model_label: Option<&str>,
) -> String {
    let line = zeron_proto::profile_line(
        harness_name,
        model_label.or(draft.model.as_deref()),
        draft.access,
        draft.reasoning,
        draft.disabled_tools.len(),
    );
    let name = draft.name.trim();
    if name.is_empty() {
        line
    } else {
        format!("{name} · {line}")
    }
}

pub fn options_for_model(
    models: &[Model],
    model: Option<&str>,
    options: Map<String, Value>,
) -> Map<String, Value> {
    model
        .and_then(|id| models.iter().find(|m| m.id == id))
        .map(|model| crate::pickers::offered_options(model, options))
        .unwrap_or_default()
}

pub fn in_pairs<T>(items: Vec<T>) -> Vec<Vec<T>> {
    let mut pairs: Vec<Vec<T>> = Vec::new();
    for item in items {
        match pairs.last_mut() {
            Some(pair) if pair.len() < 2 => pair.push(item),
            _ => pairs.push(vec![item]),
        }
    }
    pairs
}

pub fn token_count(text: &str, limit_chars: usize) -> SharedString {
    let tokens = |chars| crate::session_stats::tokens(zeron_engine::estimated_tokens(chars));
    format!(
        "~{} / {} tokens",
        tokens(text.chars().count()),
        tokens(limit_chars)
    )
    .into()
}

pub fn expand_toggle(id: &'static str, expanded: bool, muted: Hsla) -> Stateful<Div> {
    div()
        .id(id)
        .absolute()
        .top(px(4.0))
        .right(px(4.0))
        .size(px(22.0))
        .rounded(px(6.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .bg(crate::theme::ink(0.05))
        .hover(|s| s.bg(crate::theme::ink(0.1)))
        .child(
            crate::icons::icon(if expanded {
                crate::icons::LUCIDE_MINIMIZE_2
            } else {
                crate::icons::LUCIDE_MAXIMIZE_2
            })
            .size(px(12.0))
            .text_color(muted),
        )
}

pub fn folded_row(id: &'static str, text: impl Into<SharedString>, muted: Hsla) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .h(px(30.0))
        .px(px(9.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(crate::theme::hairline(0.12))
        .flex()
        .items_center()
        .gap(px(8.0))
        .cursor_pointer()
        .hover(|s| s.bg(crate::theme::ink(0.05)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(crate::typography::ui_rems(12.0))
                .text_color(muted)
                .child(text.into()),
        )
        .child(
            crate::icons::icon(crate::icons::ALT_ARROW_DOWN)
                .size(px(11.0))
                .flex_none()
                .text_color(muted),
        )
}

pub fn blank_draft(harness: HarnessId) -> AgentProfileDraft {
    AgentProfileDraft {
        name: String::new(),
        harness,
        model: None,
        reasoning: None,
        access: None,
        model_options: Default::default(),
        notes: String::new(),
        instructions: String::new(),
        disabled_tools: Vec::new(),
    }
}

gpui::actions!(agent_profiles, [OpenAgentProfileEditor]);

#[derive(Default)]
pub struct PendingProfileEdit(pub Option<(Option<String>, AgentProfileDraft)>);

impl gpui::Global for PendingProfileEdit {}

#[derive(Default)]
pub struct AgentProfilesChanged;

impl gpui::Global for AgentProfilesChanged {}

pub fn bump_agent_profiles(cx: &mut App) {
    cx.default_global::<AgentProfilesChanged>();
}

pub enum ProfileWrite {
    Upsert {
        profile_id: Option<String>,
        draft: AgentProfileDraft,
    },
    Delete {
        profile_id: String,
    },
    Reorder {
        profile_ids: Vec<String>,
    },
}

pub async fn list(
    engine: &EngineHandle,
    target: Option<Value>,
) -> Result<Vec<AgentProfile>, String> {
    let mut params = json!({});
    if let Some(target) = target {
        params["targetDeviceId"] = target;
    }
    engine
        .client()
        .call_as::<AgentProfilesSnapshot>(methods::LIST_AGENT_PROFILES, params)
        .await
        .map(|snapshot| snapshot.profiles)
        .map_err(|err| err.to_string())
}

pub async fn write(
    engine: &EngineHandle,
    write: ProfileWrite,
    target: Option<Value>,
) -> Result<Vec<AgentProfile>, String> {
    let (method, mut params) = match write {
        ProfileWrite::Upsert { profile_id, draft } => (
            methods::UPSERT_AGENT_PROFILE,
            json!({ "profileId": profile_id, "profile": draft }),
        ),
        ProfileWrite::Delete { profile_id } => (
            methods::DELETE_AGENT_PROFILE,
            json!({ "profileId": profile_id }),
        ),
        ProfileWrite::Reorder { profile_ids } => (
            methods::REORDER_AGENT_PROFILES,
            json!({ "profileIds": profile_ids }),
        ),
    };
    if let Some(target) = target {
        params["targetDeviceId"] = target;
    }
    engine
        .client()
        .call_as::<AgentProfilesSnapshot>(method, params)
        .await
        .map(|snapshot| snapshot.profiles)
        .map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeron_proto::{AccessMode, ReasoningLevel};

    #[test]
    fn summary_names_only_the_fields_the_profile_sets() {
        let mut profile = AgentProfile {
            id: "r".into(),
            name: "Reviewer".into(),
            harness: HarnessId::Codex,
            model: Some("gpt-5.6".into()),
            reasoning: Some(ReasoningLevel::XHigh),
            access: Some(AccessMode::Supervised),
            model_options: Default::default(),
            notes: String::new(),
            instructions: String::new(),
            by_agent: false,
            disabled_tools: Vec::new(),
        };
        assert_eq!(
            summary(&profile, "Codex", Some("GPT-5.6")),
            "Codex · GPT-5.6 · Supervised · X-High"
        );
        profile.model = None;
        profile.reasoning = None;
        profile.access = None;
        assert_eq!(summary(&profile, "Codex", None), "Codex");
        assert_eq!(profile.draft().name, "Reviewer");
        assert_eq!(
            draft_summary(&profile.draft(), "Codex", None),
            "Reviewer · Codex"
        );
        assert_eq!(token_count("ừ ok", 2000).as_ref(), "~1 / 500 tokens");
        assert_eq!(
            token_count(&"x".repeat(16_000), 16_000).as_ref(),
            "~4K / 4K tokens"
        );
        profile.disabled_tools = vec!["WebFetch".into(), "mcp__xcode".into()];
        assert_eq!(summary(&profile, "Codex", None), "Codex · 2 tools off");
    }

    #[test]
    fn options_follow_the_new_model() {
        let option = |id: &str, choices: &[&str]| zeron_proto::ModelOption {
            id: id.into(),
            label: id.into(),
            choices: choices
                .iter()
                .map(|c| zeron_proto::ModelOptionChoice {
                    id: c.to_string(),
                    label: c.to_string(),
                })
                .collect(),
            default_choice: choices[0].into(),
        };
        let model = |id: &str, options: Vec<zeron_proto::ModelOption>| Model {
            id: id.into(),
            label: id.into(),
            description: None,
            reasoning_levels: Vec::new(),
            options,
        };
        let models = [
            model(
                "opus",
                vec![
                    option("contextWindow", &["200k", "1m"]),
                    option("autoCompact", &["auto", "400k"]),
                ],
            ),
            model("sonnet", vec![option("contextWindow", &["200k"])]),
        ];
        let picked = json!({ "contextWindow": "1m", "autoCompact": "400k" })
            .as_object()
            .cloned()
            .unwrap();
        assert_eq!(
            options_for_model(&models, Some("opus"), picked.clone()),
            picked
        );
        assert!(options_for_model(&models, Some("sonnet"), picked.clone()).is_empty());
        let kept = json!({ "contextWindow": "200k", "autoCompact": "400k" })
            .as_object()
            .cloned()
            .unwrap();
        assert_eq!(
            options_for_model(&models, Some("sonnet"), kept),
            json!({ "contextWindow": "200k" })
                .as_object()
                .cloned()
                .unwrap()
        );
        assert!(options_for_model(&models, None, picked).is_empty());
    }
}
