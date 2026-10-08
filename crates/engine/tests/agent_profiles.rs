use std::sync::Arc;

use serde_json::json;
use zeron_engine::{EngineCore, HarnessRegistry};
use zeron_proto::AgentProfilesSnapshot;
use zeron_rpc::methods;

fn ids(snapshot: &AgentProfilesSnapshot) -> Vec<&str> {
    snapshot.profiles.iter().map(|p| p.id.as_str()).collect()
}

#[tokio::test]
async fn agent_profiles_crud_over_rpc_survives_a_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let assemble = || {
        EngineCore::assemble(
            &data,
            Arc::new(HarnessRegistry::new()),
            zeron_proto::HarnessId::Mock,
            None,
        )
        .expect("engine assembles")
    };

    let core = assemble();
    let client = zeron_rpc::memory_client(core.rpc_service());
    let empty: AgentProfilesSnapshot = client
        .call_as(methods::LIST_AGENT_PROFILES, json!({}))
        .await
        .unwrap();
    assert!(empty.profiles.is_empty());

    for (name, harness) in [("Reviewer", "codex"), ("UI work", "claude-code")] {
        client
            .call(
                methods::UPSERT_AGENT_PROFILE,
                json!({"profile": {
                    "name": name,
                    "harness": harness,
                    "model": "gpt-5.6",
                    "reasoning": "xhigh",
                    "permissionMode": "ask",
                    "notes": "Review a diff."
                }}),
            )
            .await
            .unwrap();
    }
    let reordered: AgentProfilesSnapshot = client
        .call_as(
            methods::REORDER_AGENT_PROFILES,
            json!({"profileIds": ["ui-work", "reviewer"]}),
        )
        .await
        .unwrap();
    assert_eq!(ids(&reordered), ["ui-work", "reviewer"]);

    let edited: AgentProfilesSnapshot = client
        .call_as(
            methods::UPSERT_AGENT_PROFILE,
            json!({"profileId": "reviewer", "profile": {"name": "Reviewer", "harness": "codex"}}),
        )
        .await
        .unwrap();
    assert_eq!(edited.profiles[1].model, None);
    assert!(
        client
            .call(methods::DELETE_AGENT_PROFILE, json!({"profileId": "nope"}))
            .await
            .is_err()
    );
    drop(client);
    drop(core);

    let core = assemble();
    let client = zeron_rpc::memory_client(core.rpc_service());
    let reloaded: AgentProfilesSnapshot = client
        .call_as(methods::LIST_AGENT_PROFILES, json!({}))
        .await
        .unwrap();
    assert_eq!(reloaded, edited);
    let after_delete: AgentProfilesSnapshot = client
        .call_as(
            methods::DELETE_AGENT_PROFILE,
            json!({"profileId": "ui-work"}),
        )
        .await
        .unwrap();
    assert_eq!(ids(&after_delete), ["reviewer"]);
}
