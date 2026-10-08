use gpui::{App, Hsla, IntoElement, ParentElement as _, Styled as _, div, px};
use serde_json::{Value, json};
use zeron_proto::{AgentProfile, AgentProfileDraft, AgentProfilesSnapshot, HarnessId};
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
    let mut parts = vec![harness_name.to_string()];
    if let Some(model) = model_label.or(profile.model.as_deref()) {
        parts.push(model.to_string());
    }
    if let Some(mode) = profile.permission_mode {
        parts.push(mode_label(mode).to_string());
    }
    if let Some(level) = profile.reasoning {
        parts.push(crate::pickers::reasoning_label(level).to_string());
    }
    parts.join(" · ")
}

pub fn mode_label(mode: zeron_proto::PermissionMode) -> &'static str {
    match mode {
        zeron_proto::PermissionMode::Ask => "Ask",
        zeron_proto::PermissionMode::Auto => "Auto",
        zeron_proto::PermissionMode::Full => "Full",
    }
}

pub fn draft_of(profile: &AgentProfile) -> AgentProfileDraft {
    AgentProfileDraft {
        name: profile.name.clone(),
        harness: profile.harness,
        model: profile.model.clone(),
        reasoning: profile.reasoning,
        permission_mode: profile.permission_mode,
        model_options: profile.model_options.clone(),
        notes: profile.notes.clone(),
        instructions: profile.instructions.clone(),
    }
}

pub fn blank_draft(harness: HarnessId) -> AgentProfileDraft {
    AgentProfileDraft {
        name: String::new(),
        harness,
        model: None,
        reasoning: None,
        permission_mode: None,
        model_options: Default::default(),
        notes: String::new(),
        instructions: String::new(),
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
) -> Result<Vec<AgentProfile>, String> {
    let (method, params) = match write {
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
    use zeron_proto::{PermissionMode, ReasoningLevel};

    #[test]
    fn summary_names_only_the_fields_the_profile_sets() {
        let mut profile = AgentProfile {
            id: "r".into(),
            name: "Reviewer".into(),
            harness: HarnessId::Codex,
            model: Some("gpt-5.6".into()),
            reasoning: Some(ReasoningLevel::XHigh),
            permission_mode: Some(PermissionMode::Ask),
            model_options: Default::default(),
            notes: String::new(),
            instructions: String::new(),
        };
        assert_eq!(
            summary(&profile, "Codex", Some("GPT-5.6")),
            "Codex · GPT-5.6 · Ask · X-High"
        );
        profile.model = None;
        profile.reasoning = None;
        profile.permission_mode = None;
        assert_eq!(summary(&profile, "Codex", None), "Codex");
        assert_eq!(draft_of(&profile).name, "Reviewer");
    }
}
