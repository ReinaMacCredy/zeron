//! Model catalog + effort mapping for Codex, ported from zeron's
//! `packages/harness/src/codex.ts`.
//!
//! The live catalog comes from the app server's paginated `model/list`
//! (experimentalApi). This snapshot is the failure/offline fallback, kept in
//! newest-first order so a picker remains useful when discovery cannot run.

pub(crate) use zeron_proto::model_catalog::codex::static_models;
use zeron_proto::{AccessMode, ReasoningLevel, SandboxLevel};

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

/// The approval policy and sandbox a run's thread and turns get. Full access
/// is codex's `--dangerously-bypass-approvals-and-sandbox`, as every run was
/// before modes; auto-accept edits writes the workspace and, like
/// supervised, asks before anything but codex's known-safe reads (under
/// `"on-request"` a sandboxed `rm -rf` or `git push` would run unasked).
/// A title run never asks and never writes, whatever its chat's mode.
pub(crate) fn permissions(access: AccessMode, title_only: bool) -> (&'static str, SandboxLevel) {
    match access {
        _ if title_only => ("never", SandboxLevel::ReadOnly),
        AccessMode::FullAccess => ("never", SandboxLevel::DangerFullAccess),
        AccessMode::AutoAcceptEdits => ("untrusted", SandboxLevel::WorkspaceWrite),
        AccessMode::Supervised => ("untrusted", SandboxLevel::ReadOnly),
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

/// `turn/start`'s full `sandboxPolicy` object. Workspace-write (auto-accept
/// edits) keeps network access: zeron agents fetch deps and hit APIs, and a
/// network-less sandbox would ask before every one of those commands.
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
    fn each_access_mode_has_its_own_policy_and_sandbox() {
        let wire = |access, title_only| {
            let (policy, sandbox) = permissions(access, title_only);
            (policy, sandbox_mode(sandbox))
        };
        assert_eq!(
            wire(AccessMode::FullAccess, false),
            ("never", "danger-full-access")
        );
        assert_eq!(
            wire(AccessMode::AutoAcceptEdits, false),
            ("untrusted", "workspace-write")
        );
        assert_eq!(
            wire(AccessMode::Supervised, false),
            ("untrusted", "read-only")
        );
        for access in AccessMode::ALL {
            assert_eq!(wire(access, true), ("never", "read-only"));
        }
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
        assert_eq!(models.len(), 8);
        assert_eq!(models[0].id, "gpt-6.1-sol");
        assert!(models[0].reasoning_levels.contains(&ReasoningLevel::Ultra));
        assert_eq!(models[3].id, "gpt-6-luna");
        assert!(!models[3].reasoning_levels.contains(&ReasoningLevel::Ultra));
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
