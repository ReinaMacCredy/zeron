//! Model catalog + effort mapping for Codex, ported from zeron's
//! `packages/harness/src/codex.ts`.
//!
//! The live catalog comes from the app server's paginated `model/list`
//! (experimentalApi). This snapshot is the failure/offline fallback, kept in
//! newest-first order so a picker remains useful when discovery cannot run.

use zeron_proto::{
    Model, ModelOption, ModelOptionChoice, PermissionMode, ReasoningLevel, SandboxLevel,
};

/// The unified reasoning ladder Codex accepts (`minimal` is offered but clamped
/// on the wire — see [`to_effort`]).
pub(crate) const REASONING_LEVELS: &[ReasoningLevel] = &[
    ReasoningLevel::Minimal,
    ReasoningLevel::Low,
    ReasoningLevel::Medium,
    ReasoningLevel::High,
    ReasoningLevel::XHigh,
    ReasoningLevel::Max,
    ReasoningLevel::Ultra,
];

/// Codex's API rejects `minimal` when default tools (web_search, image_gen)
/// are enabled, and doesn't know Claude's ultracode/ultrathink modes. It DOES
/// accept `max` and `ultra` natively (gpt-5.6+), so those pass straight
/// through — only the levels Codex can't take are clamped to the nearest
/// effort (port of codex.ts `toEffort`).
pub(crate) fn to_effort(reasoning: Option<ReasoningLevel>) -> Option<&'static str> {
    Some(match reasoning? {
        ReasoningLevel::Minimal | ReasoningLevel::Low => "low",
        ReasoningLevel::Medium => "medium",
        ReasoningLevel::High => "high",
        ReasoningLevel::XHigh | ReasoningLevel::Ultracode | ReasoningLevel::Ultrathink => "xhigh",
        ReasoningLevel::Max => "max",
        ReasoningLevel::Ultra => "ultra",
    })
}

/// `thread/start` / `turn/start`'s `approvalPolicy` for a permission mode.
/// `untrusted` asks for everything but codex's own trusted read-only
/// commands; `on-request` is codex's Auto (the sandbox fences, escalations
/// ask); `never` is yolo.
pub(crate) fn approval_policy(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Ask => "untrusted",
        PermissionMode::Auto => "on-request",
        PermissionMode::Full => "never",
    }
}

/// `thread/start` / `turn/start`'s `approvalsReviewer`: Auto hands escalations
/// to codex's auto-review subagent (the CLI's `--approve-for-me`); the others
/// pin `user` so a config.toml reviewer can't answer an Ask run's prompts.
pub(crate) fn approvals_reviewer(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Auto => "auto_review",
        PermissionMode::Ask | PermissionMode::Full => "user",
    }
}

/// `thread/start`'s `sandbox` param (kebab-case wire words).
pub(crate) fn sandbox_mode(sandbox: SandboxLevel) -> &'static str {
    match sandbox {
        SandboxLevel::ReadOnly => "read-only",
        SandboxLevel::WorkspaceWrite => "workspace-write",
        SandboxLevel::DangerFullAccess => "danger-full-access",
    }
}

/// `turn/start`'s `sandboxPolicy.type` (camelCase variant of the same policy).
pub(crate) fn sandbox_policy_type(sandbox: SandboxLevel) -> &'static str {
    match sandbox {
        SandboxLevel::ReadOnly => "readOnly",
        SandboxLevel::WorkspaceWrite => "workspaceWrite",
        SandboxLevel::DangerFullAccess => "dangerFullAccess",
    }
}

/// `turn/start`'s full `sandboxPolicy` object. Workspace-write keeps network
/// access: zeron agents fetch deps and hit APIs unattended, and with the
/// approval policy pinned to "never" a network-less sandbox would fail those
/// commands with no escalation path.
pub(crate) fn sandbox_policy_value(sandbox: SandboxLevel) -> serde_json::Value {
    let mut policy = serde_json::Map::new();
    policy.insert("type".into(), sandbox_policy_type(sandbox).into());
    if matches!(sandbox, SandboxLevel::WorkspaceWrite) {
        policy.insert("networkAccess".into(), true.into());
    }
    serde_json::Value::Object(policy)
}

const ULTRA_LADDER: &[ReasoningLevel] = &[
    ReasoningLevel::Low,
    ReasoningLevel::Medium,
    ReasoningLevel::High,
    ReasoningLevel::XHigh,
    ReasoningLevel::Max,
    ReasoningLevel::Ultra,
];

const MAX_LADDER: &[ReasoningLevel] = &[
    ReasoningLevel::Low,
    ReasoningLevel::Medium,
    ReasoningLevel::High,
    ReasoningLevel::XHigh,
    ReasoningLevel::Max,
];

const XHIGH_LADDER: &[ReasoningLevel] = &[
    ReasoningLevel::Low,
    ReasoningLevel::Medium,
    ReasoningLevel::High,
    ReasoningLevel::XHigh,
];

/// The service-tier select the app server reports per model (`serviceTiers` /
/// `additionalSpeedTiers` in `model/list`); "default" means Standard and is
/// omitted from the wire params entirely.
fn service_tier() -> ModelOption {
    ModelOption {
        id: "serviceTier".into(),
        label: "Service Tier".into(),
        choices: vec![
            ModelOptionChoice {
                id: "default".into(),
                label: "Standard".into(),
            },
            ModelOptionChoice {
                id: "fast".into(),
                label: "Fast".into(),
            },
        ],
        default_choice: "default".into(),
    }
}

fn model(
    id: &str,
    label: &str,
    description: &str,
    ladder: &[ReasoningLevel],
    options: Vec<ModelOption>,
) -> Model {
    Model {
        id: id.into(),
        label: label.into(),
        description: (!description.is_empty()).then(|| description.into()),
        reasoning_levels: ladder.to_vec(),
        options,
    }
}

/// The curated fallback catalog: newest family first, with efforts as the
/// app server reports them. Daybreak Blue reports NO service tiers, so it
/// carries no trait — sending `serviceTier: priority` for it would be
/// rejected. The live `model/list` remains authoritative whenever available.
pub(crate) fn static_models() -> Vec<Model> {
    vec![
        model(
            "gpt-6-astra",
            "GPT-6-Astra",
            "Our most capable model for complex, demanding work.",
            ULTRA_LADDER,
            vec![service_tier()],
        ),
        model(
            "gpt-5.6-sol",
            "GPT-5.6-Sol",
            "Frontier reasoning flagship",
            ULTRA_LADDER,
            vec![service_tier()],
        ),
        model(
            "gpt-5.6-terra",
            "GPT-5.6-Terra",
            "Deep multi-step agentic work",
            ULTRA_LADDER,
            vec![service_tier()],
        ),
        model(
            "gpt-5.6-luna",
            "GPT-5.6-Luna",
            "Fast frontier model",
            MAX_LADDER,
            vec![service_tier()],
        ),
        model(
            "gpt-daybreak-blue-latest",
            "Daybreak Blue",
            "Frontier model for defensive cybersecurity work",
            ULTRA_LADDER,
            Vec::new(),
        ),
        model(
            "gpt-5.5",
            "GPT-5.5",
            "Previous generation flagship",
            XHIGH_LADDER,
            vec![service_tier()],
        ),
        model(
            "gpt-5.4",
            "GPT-5.4",
            "Reliable general coding",
            XHIGH_LADDER,
            vec![service_tier()],
        ),
        model(
            "gpt-5.4-mini",
            "GPT-5.4-Mini",
            "Small, fast and capable",
            XHIGH_LADDER,
            vec![service_tier()],
        ),
        model(
            "gpt-5.3-codex-spark",
            "GPT-5.3-Codex-Spark",
            "Ultra-fast lightweight coding",
            XHIGH_LADDER,
            vec![service_tier()],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_mode_maps_to_codex_policy_and_sandbox() {
        assert_eq!(approval_policy(PermissionMode::Ask), "untrusted");
        assert_eq!(approval_policy(PermissionMode::Auto), "on-request");
        assert_eq!(approval_policy(PermissionMode::Full), "never");
        assert_eq!(approvals_reviewer(PermissionMode::Ask), "user");
        assert_eq!(approvals_reviewer(PermissionMode::Auto), "auto_review");
        assert_eq!(approvals_reviewer(PermissionMode::Full), "user");
        assert_eq!(
            sandbox_mode(PermissionMode::Ask.sandbox()),
            "workspace-write"
        );
        assert_eq!(
            sandbox_mode(PermissionMode::Auto.sandbox()),
            "workspace-write"
        );
        assert_eq!(
            sandbox_mode(PermissionMode::Full.sandbox()),
            "danger-full-access"
        );
    }

    #[test]
    fn effort_clamps_like_codex_ts() {
        assert_eq!(to_effort(None), None);
        assert_eq!(to_effort(Some(ReasoningLevel::Minimal)), Some("low"));
        assert_eq!(to_effort(Some(ReasoningLevel::Ultracode)), Some("xhigh"));
        assert_eq!(to_effort(Some(ReasoningLevel::Ultrathink)), Some("xhigh"));
        assert_eq!(to_effort(Some(ReasoningLevel::Max)), Some("max"));
        assert_eq!(to_effort(Some(ReasoningLevel::Ultra)), Some("ultra"));
    }

    #[test]
    fn catalog_is_newest_first_with_service_tiers() {
        let models = static_models();
        assert_eq!(models.len(), 9);
        assert_eq!(models[0].id, "gpt-6-astra");
        assert!(models[0].reasoning_levels.contains(&ReasoningLevel::Ultra));
        assert!(!models[5].reasoning_levels.contains(&ReasoningLevel::Max));
        for m in &models {
            let tier = m.options.iter().find(|o| o.id == "serviceTier");
            // Daybreak Blue reports no service tiers on the wire.
            if m.id == "gpt-daybreak-blue-latest" {
                assert!(tier.is_none(), "{} must not carry serviceTier", m.id);
            } else {
                assert!(tier.is_some(), "{} missing serviceTier", m.id);
            }
        }
    }

    #[test]
    fn daybreak_blue_rides_the_full_ultra_ladder() {
        let models = static_models();
        let daybreak = models
            .iter()
            .find(|m| m.id == "gpt-daybreak-blue-latest")
            .expect("daybreak blue in catalog");
        assert_eq!(daybreak.label, "Daybreak Blue");
        assert!(daybreak.reasoning_levels.contains(&ReasoningLevel::Ultra));
        assert!(daybreak.options.is_empty());
    }
}
