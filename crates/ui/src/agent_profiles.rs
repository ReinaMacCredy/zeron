use gpui::{
    App, Div, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    Stateful, Styled as _, div, px,
};
use serde_json::{Map, Value, json};
use zeron_proto::tools::{HarnessToolInventory, TOOL_GROUPS, ToolGroupInfo, ZERON_TOOL_PREFIX};
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
    summary_line(
        harness_name,
        model_label.or(profile.model.as_deref()),
        profile.permission_mode,
        profile.reasoning,
        profile.disabled_tools.len(),
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
        draft.disabled_tools.len(),
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
    tools_off: usize,
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
    if tools_off > 0 {
        parts.push(format!("{tools_off} tools off"));
    }
    parts.join(" · ")
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

pub fn zeron_rule(tool: &str) -> String {
    format!("{ZERON_TOOL_PREFIX}{tool}")
}

pub fn set_rules(disabled: &mut Vec<String>, rules: &[String], on: bool) {
    disabled.retain(|rule| !rules.contains(rule));
    if !on {
        disabled.extend(rules.iter().cloned());
    }
}

pub fn rules_on(disabled: &[String], rules: &[String]) -> usize {
    rules.iter().filter(|rule| !disabled.contains(rule)).count()
}

pub fn zeron_unlocked(info: &ToolGroupInfo, ceiling: &[String]) -> Vec<String> {
    info.tools()
        .iter()
        .filter(|tool| !ceiling.iter().any(|off| off == tool.name))
        .map(|tool| zeron_rule(tool.name))
        .collect()
}

pub fn set_zeron_group(
    disabled: &mut Vec<String>,
    info: &ToolGroupInfo,
    ceiling: &[String],
    on: bool,
) {
    if on {
        let all: Vec<String> = info.tools().iter().map(|t| zeron_rule(t.name)).collect();
        set_rules(disabled, &all, true);
    } else {
        set_rules(disabled, &zeron_unlocked(info, ceiling), false);
    }
}

pub fn unlisted_rules(disabled: &[String], inventory: &HarnessToolInventory) -> Vec<String> {
    disabled
        .iter()
        .filter(|rule| match rule.strip_prefix(ZERON_TOOL_PREFIX) {
            Some(name) => !TOOL_GROUPS
                .iter()
                .any(|info| info.tools().iter().any(|tool| tool.name == name)),
            None => {
                !inventory.builtin.iter().any(|g| g.tools.contains(rule))
                    && !inventory.mcp_servers.iter().any(|s| &s.rule == *rule)
            }
        })
        .cloned()
        .collect()
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
        assert_eq!(token_count("ừ ok", 2000).as_ref(), "~1 / 500 tokens");
        assert_eq!(
            token_count(&"x".repeat(16_000), 16_000).as_ref(),
            "~4K / 4K tokens"
        );
        profile.disabled_tools = vec!["WebFetch".into(), "mcp__xcode".into()];
        assert_eq!(summary(&profile, "Codex", None), "Codex · 2 tools off");
    }

    fn rules(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn a_zeron_group_switch_writes_only_unlocked_tools() {
        let pages = zeron_proto::tools::tool_group(zeron_proto::tools::ToolGroup::Pages);
        let ceiling = rules(&["html_preview"]);
        let mut disabled = rules(&["WebFetch"]);
        set_zeron_group(&mut disabled, pages, &ceiling, false);
        assert_eq!(
            disabled,
            rules(&[
                "WebFetch",
                "mcp__zeron__html_render",
                "mcp__zeron__show_visual"
            ])
        );
        assert_eq!(rules_on(&disabled, &zeron_unlocked(pages, &ceiling)), 0);

        disabled.push("mcp__zeron__html_preview".into());
        set_zeron_group(&mut disabled, pages, &ceiling, true);
        assert_eq!(disabled, rules(&["WebFetch"]));

        set_rules(&mut disabled, &[zeron_rule("html_render")], false);
        set_rules(&mut disabled, &[zeron_rule("html_render")], false);
        assert_eq!(disabled, rules(&["WebFetch", "mcp__zeron__html_render"]));
    }

    #[test]
    fn built_in_groups_and_servers_write_their_rules() {
        let web = zeron_proto::tools::BuiltinToolGroup {
            id: "web".into(),
            title: "Web".into(),
            summary: String::new(),
            tools: rules(&["WebSearch", "WebFetch"]),
        };
        let xcode = zeron_proto::tools::McpServerEntry {
            name: "xcode".into(),
            rule: zeron_proto::tools::mcp_server_rule("xcode"),
            status: "connected".into(),
            source: "user".into(),
        };
        let mut disabled = Vec::new();
        set_rules(&mut disabled, &web.tools, false);
        set_rules(&mut disabled, std::slice::from_ref(&xcode.rule), false);
        assert_eq!(disabled, rules(&["WebSearch", "WebFetch", "mcp__xcode"]));
        assert_eq!(rules_on(&disabled, &web.tools), 0);

        disabled.push("mcp__gone".into());
        disabled.push("mcp__zeron__retired_tool".into());
        disabled.push("mcp__zeron__terminal".into());
        let inventory = HarnessToolInventory {
            builtin: vec![web.clone()],
            mcp_servers: vec![xcode],
        };
        assert_eq!(
            unlisted_rules(&disabled, &inventory),
            rules(&["mcp__gone", "mcp__zeron__retired_tool"])
        );

        set_rules(&mut disabled, &web.tools, true);
        assert_eq!(
            disabled,
            rules(&[
                "mcp__xcode",
                "mcp__gone",
                "mcp__zeron__retired_tool",
                "mcp__zeron__terminal"
            ])
        );
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
