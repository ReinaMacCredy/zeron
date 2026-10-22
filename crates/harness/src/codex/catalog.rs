//! Model catalog + effort mapping for Codex, ported from zeron's
//! `packages/harness/src/codex.ts`.
//!
//! The live catalog comes from the app server's paginated `model/list`
//! (experimentalApi). This snapshot is the failure/offline fallback, kept in
//! newest-first order so a picker remains useful when discovery cannot run.

pub(crate) use zeron_proto::model_catalog::codex::static_models;
use zeron_proto::{PermissionMode, ReasoningLevel, SandboxLevel};

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
