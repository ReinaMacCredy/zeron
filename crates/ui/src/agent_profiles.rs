use gpui::{
    App, Div, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    Stateful, Styled as _, div, px,
};
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
    summary_line(
        harness_name,
        model_label.or(profile.model.as_deref()),
        profile.permission_mode,
        profile.reasoning,
    )
}

pub fn draft_summary(
    draft: &AgentProfileDraft,
    harness_name: &str,
    model_label: Option<&str>,
) -> String {
    let line = summary_line(
        harness_name,
        model_label.or(draft.model.as_deref()),
        draft.permission_mode,
        draft.reasoning,
    );
    let name = draft.name.trim();
    if name.is_empty() {
        line
    } else {
        format!("{name} · {line}")
    }
}

fn summary_line(
    harness_name: &str,
    model: Option<&str>,
    mode: Option<zeron_proto::PermissionMode>,
    reasoning: Option<zeron_proto::ReasoningLevel>,
) -> String {
    let mut parts = vec![harness_name.to_string()];
    if let Some(model) = model {
        parts.push(model.to_string());
    }
    if let Some(mode) = mode {
        parts.push(mode_label(mode).to_string());
    }
    if let Some(level) = reasoning {
        parts.push(crate::pickers::reasoning_label(level).to_string());
    }
    parts.join(" · ")
}

pub fn char_count(text: &str, limit: usize) -> SharedString {
    format!("{} / {limit}", text.chars().count()).into()
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
        disabled_tools: profile.disabled_tools.clone(),
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
            by_agent: false,
            disabled_tools: Vec::new(),
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
        assert_eq!(
            draft_summary(&draft_of(&profile), "Codex", None),
            "Reviewer · Codex"
        );
        assert_eq!(char_count("ừ ok", 2000).as_ref(), "4 / 2000");
    }
}
